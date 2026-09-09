use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Manager, State};

pub mod bindings;
pub mod config;
pub mod gamelog;
pub mod guid;
pub mod hid;
pub mod hwprofile;
pub mod input;
pub mod resort;
pub mod scdata;

/// Display labels for input tokens (e.g. `"button9"` -> `"Button 9"`).
type TokenLabels = std::collections::HashMap<String, String>;

/// Runtime state that depends on the configured SC install.
#[derive(Default)]
pub(crate) struct AppData {
    pub(crate) config: config::Config,
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

/// The clash report for the loaded profile against SC's enumeration in
/// `Game.log` (read fresh each time so a game restart is picked up). Devices
/// the user declared invisible to SC count as unplugged.
fn clash_report(data: &AppData, devices: &input::DeviceList) -> bindings::ClashReport {
    let Some(profile) = &data.profile else {
        return bindings::ClashReport::default();
    };
    let devices = devices.lock().map(|d| d.clone()).unwrap_or_default();
    let devices = bindings::without_ignored(&devices, &data.config.ignored_devices);
    let log = gamelog::read(&config::game_log_path(&data.config.base_path));
    bindings::analyze_clash(profile, &devices, log.as_ref().map_err(Clone::clone))
}

/// Compare SC's saved device order against SC's actual device order to detect
/// the `jsN` switch clash (SC assigns `jsN` by start-time device order, ignoring
/// name/GUID). Empty when no profile is loaded.
#[tauri::command]
fn get_clash_report(
    devices: State<input::DeviceList>,
    data: State<Mutex<AppData>>,
) -> bindings::ClashReport {
    clash_report(&data.lock().unwrap(), devices.inner())
}

/// Apply the clash report's resort to the live `actionmaps.xml` — the
/// out-of-game equivalent of the `pp_resortdevices` commands. The game must
/// not be running (it would overwrite the file on exit). A copy of the
/// original is kept next to it as `actionmaps.xml.<unix time>.bak`. Reloads
/// the profile afterwards and returns the load status, like `set_base_path`.
#[tauri::command]
fn apply_resort(
    devices: State<input::DeviceList>,
    actions: State<Vec<scdata::ActionMap>>,
    data: State<Mutex<AppData>>,
) -> Result<LoadStatus, String> {
    let mut data = data.lock().unwrap();
    let report = clash_report(&data, devices.inner());
    if let Some(err) = report.log_error {
        return Err(format!("no device order from Game.log: {err:?}"));
    }
    if report.resort.is_empty() {
        return Err("nothing to resort".into());
    }

    let path = config::actionmaps_path(&data.config.base_path);
    let xml = std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let rewritten = resort::rewrite_actionmaps(&xml, &report.resort)?;

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup = path.with_file_name(format!("actionmaps.xml.{stamp}.bak"));
    std::fs::copy(&path, &backup).map_err(|e| format!("backup {}: {e}", backup.display()))?;
    std::fs::write(&path, rewritten).map_err(|e| format!("write {}: {e}", path.display()))?;

    let (profile, status) = load_profile(&data.config.base_path, actions.inner());
    data.index = match &profile {
        Some(profile) => bindings::BindingIndex::build(actions.inner(), profile),
        None => bindings::BindingIndex::default(),
    };
    data.profile = profile;
    Ok(status)
}

/// Persist which connected devices the user declared invisible to SC (by SC
/// Product GUID). Returns the stored list.
#[tauri::command]
fn set_ignored_devices(
    guids: Vec<String>,
    app: AppHandle,
    data: State<Mutex<AppData>>,
) -> Vec<String> {
    let mut data = data.lock().unwrap();
    data.config.ignored_devices = guids;
    if let Err(e) = config::save(&app, &data.config) {
        eprintln!("bindsight: failed to save config: {e}");
    }
    data.config.ignored_devices.clone()
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
    // Keep the rest of the config (ignore list, profile choices) when only the
    // path changes.
    let config = config::Config { base_path: path.clone(), ..data.lock().unwrap().config.clone() };
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

/// Resolve a live input to its SC token and bound action(s). `kind` is
/// "button", "hat" or "axis"; `direction` is required for hats. Empty for
/// unknown devices, unbound inputs, or axes whose SC name is unknown (no
/// usable HID descriptor — see `DeviceInfo::axes_error`).
#[tauri::command]
fn resolve_input(
    guid: String,
    kind: String,
    index: u8,
    direction: Option<String>,
    devices: State<input::DeviceList>,
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
    let axis_name = || {
        let list = devices.lock().ok()?;
        let dev = list.iter().find(|d| d.sdl_guid == guid)?;
        dev.axes.get(index as usize).cloned()
    };
    let token = match kind.as_str() {
        "button" => Some(bindings::button_token(instance, index)),
        "hat" => direction.as_deref().and_then(|d| bindings::hat_token(instance, index, d)),
        "axis" => axis_name().map(|a| bindings::axis_token(instance, &a)),
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
        Err(e) => {
            status.error = Some(format!("actionmaps.xml not found: {} ({e})", status.actionmaps_path));
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
            status.error = Some(format!("actionmaps.xml could not be parsed: {e}"));
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
        .plugin(tauri_plugin_dialog::init())
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
            get_clash_report,
            apply_resort,
            set_ignored_devices,
            set_base_path,
            resolve_input,
            hwprofile::list_hw_profiles,
            hwprofile::get_hw_profile,
            hwprofile::create_hw_profile,
            hwprofile::save_hw_profile,
            hwprofile::delete_hw_profile,
            hwprofile::add_hw_profile_image,
            hwprofile::remove_hw_profile_image,
            hwprofile::read_hw_profile_image,
            hwprofile::export_hw_profile,
            hwprofile::import_hw_profile,
            hwprofile::set_hw_profile_choice
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
