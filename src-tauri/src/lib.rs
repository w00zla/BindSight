use std::sync::{Arc, Mutex};

use log::{debug, error, info, warn};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_log::{RotationStrategy, Target, TargetKind, TimezoneStrategy};

pub mod bindings;
pub mod config;
pub mod gamelog;
pub mod guid;
pub mod hid;
pub mod imagemap;
pub mod input;
pub mod resort;
pub mod scdata;
pub mod scinstall;

/// Game data of the configured install (action master list + token labels),
/// and how loading it went. Empty while loading or after an error.
#[derive(Default)]
struct ScState {
    version: Option<scinstall::ScVersion>,
    data: scinstall::ScData,
    loading: bool,
    /// Completed load steps (0..=`scinstall::LOAD_STEPS`) while loading.
    progress: u8,
    error: Option<String>,
    /// Bumped per load request so a slow, superseded load discards its result.
    generation: u64,
}

impl ScState {
    fn status(&self) -> ScStatus {
        ScStatus {
            version: self.version.clone(),
            loading: self.loading,
            progress: self.progress,
            steps: scinstall::LOAD_STEPS,
            error: self.error.clone(),
        }
    }
}

/// The install's version and the game-data load state, for the GUI.
#[derive(Clone, Serialize)]
struct ScStatus {
    version: Option<scinstall::ScVersion>,
    loading: bool,
    progress: u8,
    steps: u8,
    error: Option<String>,
}

/// Runtime state that depends on the configured SC install.
#[derive(Default)]
pub(crate) struct AppData {
    pub(crate) config: config::Config,
    sc: ScState,
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
    sc: ScStatus,
}

/// Return the current device list, maintained by the input thread.
#[tauri::command]
fn list_joysticks(devices: State<input::DeviceList>) -> Vec<input::DeviceInfo> {
    devices.lock().map(|d| d.clone()).unwrap_or_default()
}

/// Return the SC action master list of the configured install.
#[tauri::command]
fn get_actions(data: State<Mutex<AppData>>) -> Vec<scdata::ActionMap> {
    data.lock().unwrap().sc.data.actions.clone()
}

/// Return the input token display labels of the configured install.
#[tauri::command]
fn get_tokens(data: State<Mutex<AppData>>) -> scinstall::TokenLabels {
    data.lock().unwrap().sc.data.tokens.clone()
}

/// Return the install's version and the game-data load state.
#[tauri::command]
fn get_sc_status(data: State<Mutex<AppData>>) -> ScStatus {
    data.lock().unwrap().sc.status()
}

/// Return the current config.
#[tauri::command]
fn get_config(data: State<Mutex<AppData>>) -> config::Config {
    data.lock().unwrap().config.clone()
}

/// Return the resolved joystick bindings from the currently loaded profile.
#[tauri::command]
fn get_bindings(data: State<Mutex<AppData>>) -> Vec<bindings::ResolvedBinding> {
    let data = data.lock().unwrap();
    match &data.profile {
        Some(profile) if !data.sc.data.actions.is_empty() => bindings::resolve_bindings(&data.sc.data.actions, profile),
        _ => Vec::new(),
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
fn apply_resort(devices: State<input::DeviceList>, data: State<Mutex<AppData>>) -> Result<LoadStatus, String> {
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
    let moves: Vec<String> = report.resort.iter().map(|m| format!("js{}->js{}", m.from, m.to)).collect();
    info!("resort applied to {}: {} (backup {})", path.display(), moves.join(" "), backup.display());

    Ok(reload_profile(&mut data))
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
    info!("ignored devices set: {:?}", data.config.ignored_devices);
    if let Err(e) = config::save(&app, &data.config) {
        error!("failed to save config: {e}");
    }
    data.config.ignored_devices.clone()
}

/// Set the SC base path: persist it and reload the install's game data and
/// actionmaps.xml in the background. The result arrives as a `scdata-changed`
/// event carrying the load status.
#[tauri::command]
fn set_base_path(path: String, app: AppHandle, data: State<Mutex<AppData>>) {
    {
        let mut data = data.lock().unwrap();
        // Keep the rest of the config (ignore list, image-map choices) when
        // only the path changes.
        info!("base path set: {path}");
        data.config.base_path = path;
        if let Err(e) = config::save(&app, &data.config) {
            error!("failed to save config: {e}");
        }
    }
    spawn_sc_load(app);
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

/// (Re)load and resolve the user's actionmaps.xml against the current game
/// data, updating the profile and binding index in place.
fn reload_profile(data: &mut AppData) -> LoadStatus {
    let am_path = config::actionmaps_path(&data.config.base_path);

    let mut status = LoadStatus {
        base_path: data.config.base_path.clone(),
        actionmaps_path: am_path.display().to_string(),
        loaded: false,
        error: None,
        bindings: Vec::new(),
        sc: data.sc.status(),
    };

    let profile = std::fs::read_to_string(&am_path)
        .map_err(|e| format!("actionmaps.xml not found: {} ({e})", status.actionmaps_path))
        .and_then(|xml| {
            scdata::parse_user_profile(&xml).map_err(|e| format!("actionmaps.xml could not be parsed: {e}"))
        });
    match profile {
        Ok(profile) => {
            status.loaded = true;
            // Without game data there is nothing to resolve against (the
            // Status panel says why); the profile itself still serves the
            // clash report.
            if data.sc.data.actions.is_empty() {
                data.index = bindings::BindingIndex::default();
            } else {
                status.bindings = bindings::resolve_bindings(&data.sc.data.actions, &profile);
                data.index = bindings::BindingIndex::build(&data.sc.data.actions, &profile);
            }
            let options: Vec<String> = profile
                .joysticks
                .iter()
                .map(|j| format!("js{}={} {}", j.instance, j.product_name, j.product_guid.as_deref().unwrap_or("?")))
                .collect();
            info!(
                "profile loaded from {}: {} rebinds, {} resolved joystick bindings, options: [{}]",
                status.actionmaps_path,
                profile.rebinds.len(),
                status.bindings.len(),
                options.join(", ")
            );
            data.profile = Some(profile);
        }
        Err(e) => {
            warn!("profile not loaded: {e}");
            status.error = Some(e);
            data.index = bindings::BindingIndex::default();
            data.profile = None;
        }
    }
    log_game_log(&data.config.base_path);
    status
}

/// Log what SC's `Game.log` says about the device order — the one thing
/// remote troubleshooting of a `jsN` clash always needs.
fn log_game_log(base_path: &str) {
    let path = config::game_log_path(base_path);
    match gamelog::read(&path) {
        Ok(log) => {
            let devices: Vec<String> = log
                .joysticks
                .iter()
                .map(|j| format!("js{}={} {}", j.instance, j.product_name, j.product_guid.as_deref().unwrap_or("?")))
                .collect();
            info!(
                "Game.log {} (started {}): SC sees [{}]",
                path.display(),
                log.timestamp.as_deref().unwrap_or("?"),
                devices.join(", ")
            );
        }
        Err(e) => warn!("Game.log {}: {e:?}", path.display()),
    }
}

/// Load the configured install's game data in the background (version from
/// `build_manifest.id`, then the cached JSON or a fresh StarBreaker extraction
/// of `Data.p4k`), reload the profile against it and emit `scdata-changed`
/// with the load status. Completed steps are reported as `scdata-progress`
/// (payload: the `ScStatus`). A load superseded by a newer one discards its
/// result.
fn spawn_sc_load(app: AppHandle) {
    let (base_path, generation) = {
        let state = app.state::<Mutex<AppData>>();
        let mut data = state.lock().unwrap();
        data.sc.loading = true;
        data.sc.progress = 0;
        data.sc.error = None;
        data.sc.generation += 1;
        (data.config.base_path.clone(), data.sc.generation)
    };

    std::thread::spawn(move || {
        info!("sc data load started for {base_path}");
        // Record a completed step and tell the GUI, unless superseded.
        let progress = |step: u8| {
            let state = app.state::<Mutex<AppData>>();
            let mut data = state.lock().unwrap();
            if data.sc.generation != generation {
                debug!("sc data load step {step} discarded: superseded");
                return;
            }
            debug!("sc data load step {step}/{}", scinstall::LOAD_STEPS);
            data.sc.progress = step;
            let status = data.sc.status();
            drop(data);
            let _ = app.emit("scdata-progress", &status);
        };

        let version = scinstall::read_version(&base_path);
        let loaded = version.as_ref().map_err(Clone::clone).and_then(|version| {
            // The version is known now; show it while the rest loads.
            {
                let state = app.state::<Mutex<AppData>>();
                let mut data = state.lock().unwrap();
                if data.sc.generation == generation {
                    data.sc.version = Some(version.clone());
                }
            }
            progress(1);
            let cache_root = app.path().app_cache_dir().map_err(|e| format!("app cache dir: {e}"))?;
            let sidecar = scinstall::sidecar_path()?;
            scinstall::load(&cache_root, &sidecar, &base_path, version, &progress)
        });

        let state = app.state::<Mutex<AppData>>();
        let mut data = state.lock().unwrap();
        if data.sc.generation != generation {
            return;
        }
        data.sc.loading = false;
        data.sc.version = version.ok();
        match loaded {
            Ok(sc) => {
                info!(
                    "sc data ready: {} actionmaps, {} actions, {} token labels",
                    sc.actions.len(),
                    sc.actions.iter().map(|m| m.actions.len()).sum::<usize>(),
                    sc.tokens.len()
                );
                data.sc.data = sc;
                data.sc.error = None;
            }
            Err(e) => {
                error!("sc data failed: {e}");
                data.sc.data = scinstall::ScData::default();
                data.sc.error = Some(e);
            }
        }
        let status = reload_profile(&mut data);
        drop(data);
        let _ = app.emit("scdata-changed", &status);
    });
}

/// Log everything about the environment that remote troubleshooting might
/// need: versions, platform, the app's directories and the loaded config.
fn log_startup(app: &AppHandle, config: &config::Config) {
    let os = os_info::get();
    info!(
        "BindSight {} starting on {} {} ({}, {})",
        env!("CARGO_PKG_VERSION"),
        os.os_type(),
        os.version(),
        std::env::consts::ARCH,
        os.bitness()
    );
    let sdl = sdl2::version::version();
    info!(
        "tauri {} / webview {} / SDL {}.{}.{}",
        tauri::VERSION,
        tauri::webview_version().unwrap_or_else(|_| "?".into()),
        sdl.major,
        sdl.minor,
        sdl.patch
    );
    let dir = |r: tauri::Result<std::path::PathBuf>| r.map(|p| p.display().to_string()).unwrap_or_else(|e| format!("<{e}>"));
    info!(
        "dirs: config={} data={} cache={} log={}",
        dir(app.path().app_config_dir()),
        dir(app.path().app_data_dir()),
        dir(app.path().app_cache_dir()),
        dir(app.path().app_log_dir())
    );
    info!(
        "exe: {}",
        std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|e| format!("<{e}>"))
    );
    #[cfg(target_os = "linux")]
    {
        let var = |k: &str| std::env::var(k).unwrap_or_else(|_| "<unset>".into());
        info!(
            "session: XDG_SESSION_TYPE={} WAYLAND_DISPLAY={} DISPLAY={} WEBKIT_DISABLE_DMABUF_RENDERER={}",
            var("XDG_SESSION_TYPE"),
            var("WAYLAND_DISPLAY"),
            var("DISPLAY"),
            var("WEBKIT_DISABLE_DMABUF_RENDERER")
        );
    }
    info!(
        "config: base_path={} ignored_devices={:?} imagemap_choices={:?}",
        config.base_path, config.ignored_devices, config.imagemap_choices
    );
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
        .plugin(
            // Stdout for `tauri dev`, plus a rotating file in the app log dir
            // (see `log_startup` for its location). Our own crate and the
            // webview console (`src/logging.ts`) log down to DEBUG;
            // dependencies only when they warn.
            tauri_plugin_log::Builder::new()
                .clear_targets()
                .target(Target::new(TargetKind::Stdout))
                .target(Target::new(TargetKind::LogDir { file_name: Some("bindsight".into()) }))
                .level(log::LevelFilter::Warn)
                .level_for("bindsight_lib", log::LevelFilter::Debug)
                .level_for(tauri_plugin_log::WEBVIEW_TARGET, log::LevelFilter::Debug)
                .max_file_size(2_000_000)
                .rotation_strategy(RotationStrategy::KeepSome(3))
                .timezone_strategy(TimezoneStrategy::UseLocal)
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // A panic still ends the app (release builds abort), but this way
            // the reason reaches the log file first.
            std::panic::set_hook(Box::new(|info| error!("panic: {info}")));

            let config = config::load(app.handle());
            log_startup(app.handle(), &config);
            app.manage(Mutex::new(AppData { config, ..AppData::default() }));
            spawn_sc_load(app.handle().clone());

            let devices: input::DeviceList = Arc::new(Mutex::new(Vec::new()));
            app.manage(devices.clone());
            input::spawn(app.handle().clone(), devices);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_joysticks,
            get_actions,
            get_tokens,
            get_sc_status,
            get_config,
            get_bindings,
            get_clash_report,
            apply_resort,
            set_ignored_devices,
            set_base_path,
            resolve_input,
            imagemap::list_imagemaps,
            imagemap::get_imagemap,
            imagemap::create_imagemap,
            imagemap::save_imagemap,
            imagemap::delete_imagemap,
            imagemap::clone_imagemap,
            imagemap::add_imagemap_image,
            imagemap::remove_imagemap_image,
            imagemap::read_imagemap_image,
            imagemap::export_imagemap,
            imagemap::import_imagemap,
            imagemap::set_imagemap_choice
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
