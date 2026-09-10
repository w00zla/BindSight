use std::sync::{Arc, Mutex};

use log::{debug, error, info, warn};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_log::{RotationStrategy, Target, TargetKind, TimezoneStrategy};

pub mod backups;
pub mod bindings;
pub mod config;
pub mod diff;
pub mod gamelog;
pub mod guid;
pub mod hid;
pub mod imagemap;
pub mod input;
pub mod kblayout;
pub mod binding_profiles;
pub mod rebind;
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
    /// The active environment failed `scinstall::validate_install`: nothing
    /// of it is read (no game data, no profile, no Game.log) until it changes.
    invalid_install: bool,
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

/// Runtime state that depends on the configured SC install. `profile`,
/// `index` and `game_log` are snapshots taken by [`reload_sc`] — at start, on
/// a base-path change, after a resort, and on every Refresh.
pub(crate) struct AppData {
    pub(crate) config: config::Config,
    sc: ScState,
    profile: Option<scdata::UserProfile>,
    index: bindings::BindingIndex,
    /// SC's device enumeration from `Game.log` as of the last reload.
    game_log: Result<gamelog::LogEnumeration, gamelog::GameLogError>,
    /// Why the last `reload_profile` left `profile` empty, for `get_load_status`.
    profile_error: Option<String>,
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

/// Return the current device list, maintained by the input thread (joysticks,
/// gamepads, and the synthetic keyboard last).
#[tauri::command]
fn list_devices(devices: State<input::DeviceList>) -> Vec<input::DeviceInfo> {
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

/// Return the resolved bindings (all devices) from the currently loaded profile.
#[tauri::command]
fn get_bindings(data: State<Mutex<AppData>>) -> Vec<bindings::ResolvedBinding> {
    current_bindings(&data.lock().unwrap())
}

/// The resolved bindings for the loaded profile ("Current" in `diff.rs`'s
/// terms), or empty when nothing (or no game data) is loaded.
pub(crate) fn current_bindings(data: &AppData) -> Vec<bindings::ResolvedBinding> {
    match &data.profile {
        Some(profile) if !data.sc.data.actions.is_empty() => bindings::resolve_bindings(&data.sc.data.actions, profile),
        _ => Vec::new(),
    }
}

/// The clash report for the loaded profile against the last loaded `Game.log`
/// enumeration (see [`reload_sc`]). Devices the user declared invisible to SC
/// count as unplugged.
fn clash_report(data: &AppData, devices: &input::DeviceList) -> bindings::ClashReport {
    let Some(profile) = &data.profile else {
        return bindings::ClashReport::default();
    };
    let devices = devices.lock().map(|d| d.clone()).unwrap_or_default();
    let devices = bindings::without_ignored(&devices, &data.config.ignored_devices);
    bindings::analyze_clash(profile, &devices, data.game_log.as_ref().map_err(Clone::clone))
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
/// not be running (it would overwrite the file on exit). While auto-backups
/// are on, a backup of the original is taken first via `backups::create`
/// (reason "before order fix"). Reloads the
/// profile afterwards and returns the load status, like `set_base_path`.
#[tauri::command]
fn apply_resort(
    app: AppHandle,
    devices: State<input::DeviceList>,
    data: State<Mutex<AppData>>,
) -> Result<LoadStatus, String> {
    let mut data = data.lock().unwrap();
    let report = clash_report(&data, devices.inner());
    // GUI messages: the Status panel's tooltip already names the Game.log problem.
    if report.log_error.is_some() {
        return Err("No joystick order found".into());
    }
    if report.resort.is_empty() {
        return Err("Nothing to fix".into());
    }

    let path = config::actionmaps_path(data.config.base_path());
    let xml = std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let rewritten = resort::rewrite_actionmaps(&xml, &report.resort)?;

    let backup = if data.config.auto_backup {
        let version = data.sc.version.as_ref().map(|v| v.label.as_str());
        let root = backups::backups_root(&app)?;
        Some(backups::create(&root, &path, "before order fix", version, &data.sc.data.actions)?.id)
    } else {
        None
    };
    std::fs::write(&path, rewritten).map_err(|e| format!("write {}: {e}", path.display()))?;
    let moves: Vec<String> = report.resort.iter().map(|m| format!("js{}->js{}", m.from, m.to)).collect();
    let backup = backup.map_or_else(|| "auto-backup off".to_string(), |id| format!("backup {id}"));
    info!("resort applied to {}: {} ({backup})", path.display(), moves.join(" "));

    Ok(reload_profile(&mut data))
}

/// Write rebinds into the live `actionmaps.xml` — what the in-game keybinding
/// screen does, applied from outside. The game must not be running (it would
/// overwrite the file on exit). While auto-backups are on, a backup of the
/// original is taken first (reason "before rebind"). Reloads the profile
/// afterwards and returns the load status, like `apply_resort`.
#[tauri::command]
fn save_rebinds(
    changes: Vec<rebind::RebindChange>,
    app: AppHandle,
    data: State<Mutex<AppData>>,
) -> Result<LoadStatus, String> {
    let mut data = data.lock().unwrap();
    if data.profile.is_none() {
        return Err("No bindings loaded".into());
    }
    let path = config::actionmaps_path(data.config.base_path());
    let xml = std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let rewritten = rebind::apply_rebinds(&xml, &changes)?;

    let backup = if data.config.auto_backup {
        let version = data.sc.version.as_ref().map(|v| v.label.as_str());
        let root = backups::backups_root(&app)?;
        Some(backups::create(&root, &path, "before rebind", version, &data.sc.data.actions)?.id)
    } else {
        None
    };
    std::fs::write(&path, rewritten).map_err(|e| format!("write {}: {e}", path.display()))?;
    let summary: Vec<String> = changes
        .iter()
        .map(|c| format!("{}/{}={}", c.actionmap, c.action, c.input.trim()))
        .collect();
    let backup = backup.map_or_else(|| "auto-backup off".to_string(), |id| format!("backup {id}"));
    info!("rebinds written to {}: {} ({backup})", path.display(), summary.join(" "));

    Ok(reload_profile(&mut data))
}

/// Facts about the loaded `actionmaps.xml` for the Bindings mode: where it
/// is, its size and mtime, how many rebinds it holds and which joysticks its
/// `<options>` name.
#[derive(Serialize)]
struct ProfileInfo {
    path: String,
    /// Unix seconds; 0 if unknown.
    modified: u64,
    size: u64,
    rebinds: usize,
    joysticks: Vec<scdata::JoystickDevice>,
}

/// `None` while no profile is loaded.
#[tauri::command]
fn get_profile_info(data: State<Mutex<AppData>>) -> Option<ProfileInfo> {
    let data = data.lock().unwrap();
    let profile = data.profile.as_ref()?;
    let path = config::actionmaps_path(data.config.base_path());
    let meta = std::fs::metadata(&path).ok();
    let modified = meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    Some(ProfileInfo {
        path: path.display().to_string(),
        modified,
        size: meta.map_or(0, |m| m.len()),
        rebinds: profile.rebinds.len(),
        joysticks: profile.joysticks.clone(),
    })
}

/// Re-read everything from the SC install (actionmaps.xml and Game.log)
/// without touching the config — what Refresh does.
#[tauri::command]
fn reload(data: State<Mutex<AppData>>) -> LoadStatus {
    reload_profile(&mut data.lock().unwrap())
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

/// Persist whether BindSight backs up `actionmaps.xml` before overwriting it
/// (Settings Save).
#[tauri::command]
fn set_auto_backup(enabled: bool, app: AppHandle, data: State<Mutex<AppData>>) {
    let mut data = data.lock().unwrap();
    if data.config.auto_backup == enabled {
        return;
    }
    data.config.auto_backup = enabled;
    info!("auto-backup set: {enabled}");
    if let Err(e) = config::save(&app, &data.config) {
        error!("failed to save config: {e}");
    }
}

/// Cap the log level: DEBUG while Settings "Enable debug logging" is on, INFO
/// otherwise. `log::set_max_level` is global and immediate; dependencies stay
/// at WARN through the plugin's per-target filter. Webview records bypass the
/// cap (the plugin's `log` command), `src/logging.ts` drops those itself.
fn apply_log_level(debug: bool) {
    log::set_max_level(if debug { log::LevelFilter::Debug } else { log::LevelFilter::Info });
}

/// Persist the debug logging switch and apply it right away (Settings Save).
#[tauri::command]
fn set_debug_logging(enabled: bool, app: AppHandle, data: State<Mutex<AppData>>) {
    let mut data = data.lock().unwrap();
    if data.config.debug_logging == enabled {
        return;
    }
    data.config.debug_logging = enabled;
    apply_log_level(enabled);
    info!("debug logging set: {enabled}");
    if let Err(e) = config::save(&app, &data.config) {
        error!("failed to save config: {e}");
    }
}

/// Open the app's log folder in the system file manager (Settings).
#[tauri::command]
fn open_log_dir(app: AppHandle) -> Result<(), String> {
    let dir = app.path().app_log_dir().map_err(|e| format!("app log dir: {e}"))?;
    tauri_plugin_opener::open_path(&dir, None::<&str>).map_err(|e| format!("open {}: {e}", dir.display()))
}

/// Write a text file to a path the user picked in a save dialog (the Devices
/// log).
#[tauri::command]
fn write_text_file(path: String, text: String) -> Result<(), String> {
    std::fs::write(&path, text).map_err(|e| format!("write {path}: {e}"))?;
    info!("wrote {path}");
    Ok(())
}

/// Store the environments (Settings Save). When the active one's path or
/// label source changed, the install's game data and actionmaps.xml are
/// reloaded in the background (result via `scdata-changed`); returns whether
/// that reload was started.
#[tauri::command]
fn set_environments(
    environments: std::collections::BTreeMap<String, config::Environment>,
    app: AppHandle,
    data: State<Mutex<AppData>>,
) -> bool {
    let reload = {
        let mut data = data.lock().unwrap();
        let before = data.config.active().clone();
        for (slug, env) in environments {
            if config::ENVIRONMENTS.contains(&slug.as_str()) {
                data.config.environments.insert(slug, env);
            } else {
                warn!("ignoring unknown environment {slug}");
            }
        }
        info!("environments set: {:?}", data.config.environments);
        if let Err(e) = config::save(&app, &data.config) {
            error!("failed to save config: {e}");
        }
        *data.config.active() != before
    };
    if reload {
        spawn_sc_load(app);
    }
    reload
}

/// Switch the active environment (top-bar chip): persist it and reload the
/// install's game data and actionmaps.xml in the background (result via
/// `scdata-changed`). Returns whether anything changed.
#[tauri::command]
fn set_active_env(slug: String, app: AppHandle, data: State<Mutex<AppData>>) -> Result<bool, String> {
    {
        let mut data = data.lock().unwrap();
        if !config::ENVIRONMENTS.contains(&slug.as_str()) {
            return Err(format!("unknown environment {slug}"));
        }
        if data.config.active_env == slug {
            return Ok(false);
        }
        info!("active environment set: {slug} ({})", data.config.environments[&slug].path);
        data.config.active_env = slug;
        if let Err(e) = config::save(&app, &data.config) {
            error!("failed to save config: {e}");
        }
    }
    spawn_sc_load(app);
    Ok(true)
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

/// Resolve a live keyboard/gamepad press against a list of candidate SC
/// tokens, most specific first — the frontend offers every held-modifier combo
/// (`kb1_lalt+x`) before the bare token (`kb1_x`), because SC stores the combo
/// in one token. The first candidate with bound actions wins; if none is
/// bound, the first candidate is reported as the token so the GUI can still
/// name the input.
#[tauri::command]
fn resolve_tokens(candidates: Vec<String>, data: State<Mutex<AppData>>) -> InputResolution {
    let data = data.lock().unwrap();
    for candidate in &candidates {
        let actions = data.index.resolve(candidate);
        if !actions.is_empty() {
            return InputResolution { token: Some(candidate.clone()), actions: actions.to_vec() };
        }
    }
    InputResolution { token: candidates.into_iter().next(), actions: Vec::new() }
}

/// Snapshot the SC install into `data`: parse actionmaps.xml and resolve it
/// against the current game data (profile + binding index), then read
/// Game.log. Returns the actionmaps load status. Called at start (once the
/// game data is in), on a base-path change, after a resort or restore, and
/// on every Refresh.
pub(crate) fn reload_profile(data: &mut AppData) -> LoadStatus {
    let am_path = config::actionmaps_path(data.config.base_path());

    let mut status = LoadStatus {
        base_path: data.config.base_path().to_string(),
        actionmaps_path: am_path.display().to_string(),
        loaded: false,
        error: None,
        bindings: Vec::new(),
        sc: data.sc.status(),
    };

    // An invalid environment is reported once (the SC-data error) and
    // otherwise left alone: nothing of it is read.
    if data.sc.invalid_install {
        data.profile = None;
        data.index = bindings::BindingIndex::default();
        data.game_log = Err(gamelog::GameLogError::NotFound {
            path: config::game_log_path(data.config.base_path()).display().to_string(),
            reason: "environment not loaded".into(),
        });
        return status;
    }

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
                "profile loaded from {}: {} rebinds, {} resolved bindings, options: [{}]",
                status.actionmaps_path,
                profile.rebinds.len(),
                status.bindings.len(),
                options.join(", ")
            );
            data.profile = Some(profile);
            data.profile_error = None;
        }
        Err(e) => {
            warn!("profile not loaded: {e}");
            status.error = Some(e.clone());
            data.index = bindings::BindingIndex::default();
            data.profile = None;
            data.profile_error = Some(e);
        }
    }
    data.game_log = read_game_log(data.config.base_path());
    status
}

/// The outcome of the last `reload_profile` without reading anything again —
/// for a frontend that mounts after the first load already finished (its
/// `scdata-changed` listener was not up yet).
#[tauri::command]
fn get_load_status(data: State<Mutex<AppData>>) -> LoadStatus {
    let data = data.lock().unwrap();
    LoadStatus {
        base_path: data.config.base_path().to_string(),
        actionmaps_path: config::actionmaps_path(data.config.base_path()).display().to_string(),
        loaded: data.profile.is_some(),
        error: data.profile_error.clone(),
        bindings: current_bindings(&data),
        sc: data.sc.status(),
    }
}

/// Read SC's `Game.log` and log what it says about the device order — the
/// one thing remote troubleshooting of a `jsN` clash always needs.
fn read_game_log(base_path: &str) -> Result<gamelog::LogEnumeration, gamelog::GameLogError> {
    let path = config::game_log_path(base_path);
    let result = gamelog::read(&path);
    match &result {
        Ok(log) => {
            let devices: Vec<String> = log
                .joysticks
                .iter()
                .map(|j| format!("js{}={} {}", j.instance, j.product_name, j.product_guid.as_deref().unwrap_or("?")))
                .collect();
            info!(
                "Game.log {} (started {}): SC sees [{}], gamepads: [{}]",
                path.display(),
                log.timestamp.as_deref().unwrap_or("?"),
                devices.join(", "),
                log.gamepads.join(", ")
            );
        }
        Err(e) => warn!("Game.log {}: {e:?}", path.display()),
    }
    result
}

/// Load the configured install's game data in the background (version from
/// `build_manifest.id`, then the cached JSON or a fresh StarBreaker extraction
/// of `Data.p4k`), reload the profile against it and emit `scdata-changed`
/// with the load status. Completed steps are reported as `scdata-progress`
/// (payload: the `ScStatus`). A load superseded by a newer one discards its
/// result.
fn spawn_sc_load(app: AppHandle) {
    let (base_path, global_ini, generation) = {
        let state = app.state::<Mutex<AppData>>();
        let mut data = state.lock().unwrap();
        data.sc.loading = true;
        data.sc.progress = 0;
        data.sc.error = None;
        data.sc.generation += 1;
        (data.config.base_path().to_string(), data.config.global_ini_override(), data.sc.generation)
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

        // Validate the folder first; one missing file aborts the whole load.
        let valid = scinstall::validate_install(&base_path);
        let version = valid.clone().and_then(|()| scinstall::read_version(&base_path));
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
            scinstall::load(&cache_root, &sidecar, &base_path, version, global_ini.as_deref(), &progress)
        });

        let state = app.state::<Mutex<AppData>>();
        let mut data = state.lock().unwrap();
        if data.sc.generation != generation {
            return;
        }
        data.sc.loading = false;
        data.sc.version = version.ok();
        data.sc.invalid_install = valid.is_err();
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
        "config: active_env={} environments={:?} ignored_devices={:?} imagemap_choices={:?}",
        config.active_env, config.environments, config.ignored_devices, config.imagemap_choices
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
            apply_log_level(config.debug_logging);
            log_startup(app.handle(), &config);
            // The profile and Game.log are read once the game data is in
            // (`spawn_sc_load` -> `reload_profile`).
            app.manage(Mutex::new(AppData {
                game_log: Err(gamelog::GameLogError::NotFound {
                    path: config::game_log_path(config.base_path()).display().to_string(),
                    reason: "not read yet".into(),
                }),
                config,
                sc: ScState::default(),
                profile: None,
                index: bindings::BindingIndex::default(),
                profile_error: None,
            }));
            spawn_sc_load(app.handle().clone());

            let devices: input::DeviceList = Arc::new(Mutex::new(Vec::new()));
            app.manage(devices.clone());
            input::spawn(app.handle().clone(), devices);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_devices,
            kblayout::keyboard_layout,
            get_actions,
            get_tokens,
            get_sc_status,
            get_config,
            get_bindings,
            get_load_status,
            reload,
            get_clash_report,
            apply_resort,
            save_rebinds,
            get_profile_info,
            set_ignored_devices,
            set_environments,
            set_active_env,
            resolve_input,
            resolve_tokens,
            write_text_file,
            open_log_dir,
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
            imagemap::set_imagemap_choice,
            binding_profiles::list_binding_profiles,
            binding_profiles::import_binding_profile,
            binding_profiles::export_binding_profile,
            backups::list_backups,
            backups::create_backup,
            backups::delete_backup,
            backups::restore_backup,
            backups::open_backup_dir,
            backups::open_backups_dir,
            set_auto_backup,
            set_debug_logging,
            diff::compare_bindings
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
