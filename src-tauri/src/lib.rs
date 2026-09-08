use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Manager, State};

pub mod bindings;
pub mod config;
pub mod guid;
pub mod input;
pub mod scdata;

/// Display labels for input tokens (e.g. `"button9"` -> `"Button 9"`).
type TokenLabels = std::collections::HashMap<String, String>;

/// Runtime state that depends on the configured SC install.
#[derive(Default)]
struct AppData {
    config: config::Config,
    profile: Option<scdata::UserProfile>,
    index: bindings::BindingIndex,
}

/// Result of (re)loading the user's actionmaps.xml.
#[derive(Serialize)]
struct LoadStatus {
    base_path: String,
    actionmaps_path: String,
    loaded: bool,
    error: Option<String>,
    bindings: Vec<bindings::ResolvedBinding>,
}

/// Return the current device list, maintained by the input thread.
#[tauri::command]
fn list_joysticks(devices: State<input::DeviceList>) -> Vec<input::DeviceInfo> {
    devices.lock().map(|d| d.clone()).unwrap_or_default()
}

/// Return the bundled SC action master list.
#[tauri::command]
fn get_actions(actions: State<Vec<scdata::ActionMap>>) -> Vec<scdata::ActionMap> {
    actions.inner().clone()
}

/// Return the bundled input token display labels.
#[tauri::command]
fn get_tokens(tokens: State<TokenLabels>) -> TokenLabels {
    tokens.inner().clone()
}

/// Return the current config.
#[tauri::command]
fn get_config(data: State<Mutex<AppData>>) -> config::Config {
    data.lock().unwrap().config.clone()
}

/// Return the resolved joystick bindings from the currently loaded profile.
#[tauri::command]
fn get_bindings(
    actions: State<Vec<scdata::ActionMap>>,
    data: State<Mutex<AppData>>,
) -> Vec<bindings::ResolvedBinding> {
    let data = data.lock().unwrap();
    match &data.profile {
        Some(profile) => bindings::resolve_bindings(actions.inner(), profile),
        None => Vec::new(),
    }
}

/// Set the SC base path: persist it, reload actionmaps.xml, and report the
/// resulting bindings.
#[tauri::command]
fn set_base_path(
    path: String,
    app: AppHandle,
    actions: State<Vec<scdata::ActionMap>>,
    data: State<Mutex<AppData>>,
) -> LoadStatus {
    let config = config::Config { base_path: path.clone() };
    if let Err(e) = config::save(&app, &config) {
        eprintln!("bindsight: failed to save config: {e}");
    }

    let (profile, status) = load_profile(&path, actions.inner());
    let index = match &profile {
        Some(profile) => bindings::BindingIndex::build(actions.inner(), profile),
        None => bindings::BindingIndex::default(),
    };
    let mut data = data.lock().unwrap();
    data.config = config;
    data.profile = profile;
    data.index = index;
    status
}

/// The SC token a live input resolves to (if the device is in the SC profile)
/// and the action(s) bound to it.
#[derive(Default, Serialize)]
struct InputResolution {
    token: Option<String>,
    actions: Vec<bindings::BoundAction>,
}

/// Resolve a live input to its SC token and bound action(s). `kind` is "button"
/// or "hat"; `direction` is required for hats. Empty for axes (no token mapping
/// yet), unknown devices, or unbound inputs.
#[tauri::command]
fn resolve_input(
    guid: String,
    kind: String,
    index: u8,
    direction: Option<String>,
    data: State<Mutex<AppData>>,
) -> InputResolution {
    let data = data.lock().unwrap();
    let Some(profile) = &data.profile else {
        return InputResolution::default();
    };
    let Some(sc_guid) = guid::sdl_guid_to_sc_product(&guid) else {
        return InputResolution::default();
    };
    let Some(instance) = bindings::instance_for_guid(profile, &sc_guid) else {
        return InputResolution::default();
    };
    let token = match kind.as_str() {
        "button" => Some(bindings::button_token(instance, index)),
        "hat" => direction.as_deref().and_then(|d| bindings::hat_token(instance, index, d)),
        _ => None,
    };
    let actions = token.as_deref().map(|t| data.index.resolve(t).to_vec()).unwrap_or_default();
    InputResolution { token, actions }
}

/// Load and resolve the user's actionmaps.xml for a base path.
fn load_profile(base_path: &str, actions: &[scdata::ActionMap]) -> (Option<scdata::UserProfile>, LoadStatus) {
    let am_path = config::actionmaps_path(base_path);
    let am_display = am_path.display().to_string();

    let mut status = LoadStatus {
        base_path: base_path.to_string(),
        actionmaps_path: am_display,
        loaded: false,
        error: None,
        bindings: Vec::new(),
    };

    let xml = match std::fs::read_to_string(&am_path) {
        Ok(xml) => xml,
        Err(_) => {
            status.error = Some("Required game files not found".to_string());
            return (None, status);
        }
    };
    match scdata::parse_user_profile(&xml) {
        Ok(profile) => {
            status.loaded = true;
            status.bindings = bindings::resolve_bindings(actions, &profile);
            (Some(profile), status)
        }
        Err(e) => {
            status.error = Some(e);
            (None, status)
        }
    }
}

/// Read a bundled resource file. Falls back to the source-tree copy in
/// development, where the bundled resource is absent.
fn read_resource(app: &AppHandle, name: &str) -> Option<String> {
    app.path()
        .resolve(format!("resources/{name}"), BaseDirectory::Resource)
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .or_else(|| std::fs::read_to_string(format!("{}/resources/{name}", env!("CARGO_MANIFEST_DIR"))).ok())
}

/// Load the bundled `scdata.json` (the action master list).
fn load_actions(app: &AppHandle) -> Vec<scdata::ActionMap> {
    match read_resource(app, "scdata.json") {
        Some(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            eprintln!("bindsight: failed to parse scdata.json: {e}");
            Vec::new()
        }),
        None => {
            eprintln!("bindsight: scdata.json not found");
            Vec::new()
        }
    }
}

/// Load the bundled `tokens.json` (input token display labels).
fn load_tokens(app: &AppHandle) -> TokenLabels {
    match read_resource(app, "tokens.json") {
        Some(text) => serde_json::from_str(&text).unwrap_or_default(),
        None => TokenLabels::default(),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // WebKitGTK's DMABUF renderer triggers "Error 71 (Protocol error)" on many
    // Wayland/Nvidia setups. Disabling it is the canonical fix. Respect an
    // explicit user override; otherwise force it off on Linux.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let actions = load_actions(app.handle());
            let config = config::load(app.handle());
            let (profile, _) = load_profile(&config.base_path, &actions);
            let index = match &profile {
                Some(profile) => bindings::BindingIndex::build(&actions, profile),
                None => bindings::BindingIndex::default(),
            };
            app.manage(actions);
            app.manage(load_tokens(app.handle()));
            app.manage(Mutex::new(AppData { config, profile, index }));

            let devices: input::DeviceList = Arc::new(Mutex::new(Vec::new()));
            app.manage(devices.clone());
            input::spawn(app.handle().clone(), devices);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_joysticks,
            get_actions,
            get_tokens,
            get_config,
            get_bindings,
            set_base_path,
            resolve_input
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
