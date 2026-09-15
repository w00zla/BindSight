use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use log::{debug, error, info, warn};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_log::{RotationStrategy, Target, TargetKind, TimezoneStrategy};

pub mod apply;
pub mod backups;
pub mod bindings;
pub mod config;
pub mod cryxml;
pub mod diff;
pub mod dinput;
pub mod gamefile;
pub mod gamelog;
pub mod guid;
pub mod hid;
pub mod imagemap;
pub mod input;
pub mod kblayout;
pub mod logwatch;
pub mod names;
pub mod order;
pub mod p4k;
pub mod binding_profiles;
pub mod rebind;
pub mod resort;
pub mod scdata;
pub mod scinstall;
pub mod textpath;
pub mod update;
pub mod wineorder;
pub mod xmltext;

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
    /// of it is read (no game data, no bindings file, no Game.log) until it
    /// changes.
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

/// Runtime state that depends on the configured SC install. `bindings_file`,
/// `index` and `game_log` are snapshots taken by [`reload_bindings`] — at
/// start, on a base-path change, after a resort, and on every Refresh;
/// `device_order` also on every clash report (hot-plug).
pub(crate) struct AppData {
    pub(crate) config: config::Config,
    sc: ScState,
    bindings_file: Option<scdata::ActionMapsFile>,
    index: bindings::BindingIndex,
    /// SC's joystick order — the one thing every `jsN` comes from: the
    /// platform's live source (`order::live`, stamped when it changed).
    /// Errors say why there is none.
    device_order: Result<order::DeviceOrder, String>,
    /// `Game.log` as of the last read: what the game started with — the
    /// second opinion (`ClashReport::logged_order`), never the source.
    game_log: Result<order::DeviceOrder, String>,
    /// The SDL device list (the same `Arc` the input thread maintains), for
    /// the live order source — taken briefly, never while it is slow.
    devices: input::DeviceList,
    /// Why the last `reload_bindings` left `bindings_file` empty, for
    /// `get_load_status`.
    bindings_error: Option<String>,
    /// The last clash summary written to the log, so the same outcome is not
    /// logged again on every Refresh / hot-plug (see [`clash_report`]).
    last_clash_log: String,
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

/// The hidapi joystick-class devices SDL does not list (Device List only).
#[tauri::command]
fn hid_only_devices(devices: State<input::DeviceList>) -> Vec<input::HidOnlyDevice> {
    let devices = devices.lock().map(|d| d.clone()).unwrap_or_default();
    input::hid_only_devices(&devices)
}

/// The HID interface keys Wine registers for the listed devices, in Wine's
/// order (Device Info only): empty where the order does not come from Wine
/// (Windows) or its enumeration fails.
#[tauri::command]
fn wine_keys(devices: State<input::DeviceList>) -> Vec<wineorder::WineKey> {
    #[cfg(target_os = "linux")]
    {
        let devices = devices.lock().map(|d| d.clone()).unwrap_or_default();
        wineorder::live_keys(&devices).unwrap_or_default()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = devices;
        Vec::new()
    }
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

/// Return the resolved bindings (all devices) from the currently loaded
/// actionmaps.xml.
#[tauri::command]
fn get_bindings(data: State<Mutex<AppData>>) -> Vec<bindings::ResolvedBinding> {
    current_bindings(&data.lock().unwrap())
}

/// The resolved bindings for the loaded actionmaps.xml ("Current" in
/// `diff.rs`'s terms), or empty when nothing (or no game data) is loaded.
pub(crate) fn current_bindings(data: &AppData) -> Vec<bindings::ResolvedBinding> {
    match &data.bindings_file {
        Some(profile) if !data.sc.data.actions.is_empty() => bindings::resolve_bindings(&data.sc.data.actions, profile),
        _ => Vec::new(),
    }
}

/// The clash report for the loaded actionmaps.xml against SC's joystick order
/// (see [`AppData::device_order`]; `live` is [`live_order`], taken by the
/// caller *before* the lock — DirectInput can take a while — so a hot-plug
/// is in). `logged_order` carries the game's logged order when it differs.
fn clash_report(
    data: &mut AppData,
    devices: &input::DeviceList,
    live: Option<Result<order::DeviceOrder, String>>,
) -> bindings::ClashReport {
    refresh_device_order(data, live);
    // Without a bindings file (no install configured, or it failed to load)
    // the order still says which joysticks the game sees: the report is
    // taken against an empty file, so `connected` and `unseen` are right
    // and nothing is saved anywhere (no clash, nothing to resort).
    let empty = scdata::ActionMapsFile { joysticks: Vec::new(), rebinds: Vec::new() };
    let profile = data.bindings_file.as_ref().unwrap_or(&empty);
    let devices = devices.lock().map(|d| d.clone()).unwrap_or_default();
    let mut report = bindings::analyze_clash(profile, &devices, data.device_order.as_ref().map_err(Clone::clone));
    // "Game devices update": when the game last listed its joysticks.
    report.log_timestamp = data.game_log.as_ref().ok().and_then(|l| l.timestamp.clone());
    // The game keeps the order it started with: a live order ranking the
    // devices differently means "restart the game".
    if let (Ok(live), Ok(logged)) = (&data.device_order, &data.game_log) {
        if !live.same_ranking(logged) {
            report.logged_order = Some(logged.clone());
        }
    }
    // The report is recomputed on every Refresh, hot-plug and write; only a
    // changed outcome is worth a line. The unseen devices are named: SDL
    // lists them, the game's enumeration does not (the GUI says so only in
    // the Device List).
    let unseen: Vec<String> = report
        .unseen
        .iter()
        .map(|u| format!("{} {}", u.name.as_deref().unwrap_or("?"), u.sc_product_guid.as_deref().unwrap_or("?")))
        .collect();
    let summary = format!(
        "clash: has_clash={} connected={} missing={} unseen=[{}] log_differs={} resort=[{}]",
        report.has_clash,
        report.connected.len(),
        report.missing.len(),
        unseen.join(", "),
        report.logged_order.is_some(),
        report.resort_commands.join(" | ")
    );
    if summary != data.last_clash_log {
        info!("{summary}");
        data.last_clash_log = summary;
    }
    report
}

/// Compare SC's saved device order against SC's actual device order to detect
/// the `jsN` switch clash (SC assigns `jsN` by start-time device order, ignoring
/// name/GUID). Empty when no actionmaps.xml is loaded.
#[tauri::command]
fn get_clash_report(
    devices: State<input::DeviceList>,
    data: State<Mutex<AppData>>,
) -> bindings::ClashReport {
    let live = live_order(&devices);
    clash_report(&mut data.lock().unwrap(), devices.inner(), live)
}

/// Apply the clash report's resort to the live `actionmaps.xml` — the
/// out-of-game equivalent of the `pp_resortdevices` commands. The game must
/// not be running (it would overwrite the file on exit). While auto-backups
/// are on, a backup of the original is taken first via `backups::create`
/// (reason "before order fix"). Reloads the
/// bindings afterwards and returns the load status, like `set_environments`
/// / `set_active_env`.
#[tauri::command]
fn apply_resort(
    app: AppHandle,
    devices: State<input::DeviceList>,
    data: State<Mutex<AppData>>,
) -> Result<LoadStatus, String> {
    let live = live_order(&devices);
    let mut data = data.lock().unwrap();
    let report = clash_report(&mut data, devices.inner(), live);
    // GUI messages: the Status panel already names the order problem.
    if report.order_error.is_some() {
        return Err("No joystick order found".into());
    }
    if report.resort.is_empty() {
        return Err("Nothing to fix".into());
    }
    write_resort(&app, &mut data, &report.resort, "before order fix", "resort")
}

/// Swap two joystick slots in the live `actionmaps.xml` — the user's own
/// reorder from the Bindings mode, one swap at a time, the same guarded
/// rewrite as the order fix (backup "before reorder"). Reloads afterwards.
#[tauri::command]
fn apply_reorder(a: u32, b: u32, app: AppHandle, data: State<Mutex<AppData>>) -> Result<LoadStatus, String> {
    if a == 0 || b == 0 {
        return Err("Joystick slots start at js1".into());
    }
    if a == b {
        return Err("Choose two different slots".into());
    }
    let mut data = data.lock().unwrap();
    if data.bindings_file.is_none() {
        return Err("No bindings loaded".into());
    }
    let name = |slot: u32| -> Option<String> {
        data.bindings_file.as_ref()?.joysticks.iter().find(|j| j.instance == slot).map(|j| j.product_name.clone())
    };
    let moves = [
        bindings::ResortMove { from: a, to: b, name: name(a) },
        bindings::ResortMove { from: b, to: a, name: name(b) },
    ];
    write_resort(&app, &mut data, &moves, "before reorder", "reorder")
}

/// The one write behind the order fix and the reorder: the textual rewrite
/// (validated and verified in `resort`), then the guarded replacement of
/// the live file. `what` names the caller in the log.
fn write_resort(
    app: &AppHandle,
    data: &mut AppData,
    moves: &[bindings::ResortMove],
    reason: &str,
    what: &str,
) -> Result<LoadStatus, String> {
    let path = config::actionmaps_path(data.config.base_path());
    let xml = std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let rewritten = resort::rewrite_actionmaps(&xml, moves).map_err(|e| {
        error!("{what} rewrite refused ({} move(s)): {e}", moves.len());
        e
    })?;

    let version = data.sc.version.as_ref().map(|v| v.label.as_str());
    let root = backups::backups_root(app)?;
    let backup =
        gamefile::replace_live_file(&root, &path, &rewritten, reason, data.config.auto_backup, version, &data.sc.data.actions)?;
    let listed: Vec<String> = moves.iter().map(|m| format!("js{}->js{}", m.from, m.to)).collect();
    info!("{what} applied to {}: {} ({})", path.display(), listed.join(" "), gamefile::backup_label(&backup));

    Ok(reload_bindings(data))
}

/// Write rebinds into the live `actionmaps.xml` — what the in-game keybinding
/// screen does, applied from outside. The game must not be running (it would
/// overwrite the file on exit). While auto-backups are on, a backup of the
/// original is taken first (reason "before rebind"). Reloads the bindings
/// afterwards and returns the load status, like `apply_resort`.
#[tauri::command]
fn save_rebinds(
    changes: Vec<rebind::RebindChange>,
    app: AppHandle,
    data: State<Mutex<AppData>>,
) -> Result<LoadStatus, String> {
    let mut data = data.lock().unwrap();
    if data.bindings_file.is_none() {
        return Err("No bindings loaded".into());
    }
    let path = config::actionmaps_path(data.config.base_path());
    let xml = std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let rewritten = rebind::apply_rebinds(&xml, &changes).map_err(|e| {
        error!("rebind rewrite refused ({} change(s)): {e}", changes.len());
        e
    })?;

    let version = data.sc.version.as_ref().map(|v| v.label.as_str());
    let root = backups::backups_root(&app)?;
    let backup = gamefile::replace_live_file(&root, &path, &rewritten, "before rebind", data.config.auto_backup, version, &data.sc.data.actions)?;
    let summary: Vec<String> = changes
        .iter()
        .map(|c| format!("{}/{}={}", c.actionmap, c.action, c.input.trim()))
        .collect();
    info!("rebinds written to {}: {} ({})", path.display(), summary.join(" "), gamefile::backup_label(&backup));

    Ok(reload_bindings(&mut data))
}

/// Facts about the loaded `actionmaps.xml` for the Bindings mode: where it
/// is, its size and mtime, how many rebinds it holds and which joysticks its
/// `<options>` name.
#[derive(Serialize)]
struct CurrentBindingsInfo {
    path: String,
    /// Unix seconds; 0 if unknown.
    modified: u64,
    size: u64,
    rebinds: usize,
    joysticks: Vec<scdata::JoystickDevice>,
}

/// `None` while no actionmaps.xml is loaded.
#[tauri::command]
fn get_current_bindings_info(data: State<Mutex<AppData>>) -> Option<CurrentBindingsInfo> {
    let data = data.lock().unwrap();
    let profile = data.bindings_file.as_ref()?;
    let path = config::actionmaps_path(data.config.base_path());
    let meta = std::fs::metadata(&path).ok();
    let modified = meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    Some(CurrentBindingsInfo {
        path: path.display().to_string(),
        modified,
        size: meta.map_or(0, |m| m.len()),
        rebinds: profile.rebinds.len(),
        joysticks: profile.joysticks.clone(),
    })
}

/// Re-read the bindings file and re-take the joystick order without
/// touching the config — what Refresh does.
#[tauri::command]
fn reload(data: State<Mutex<AppData>>) -> LoadStatus {
    reload_bindings(&mut data.lock().unwrap())
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

/// Persist the startup update-check switch (Settings Save).
#[tauri::command]
fn set_update_check(enabled: bool, app: AppHandle, data: State<Mutex<AppData>>) {
    let mut data = data.lock().unwrap();
    if data.config.update_check == enabled {
        return;
    }
    data.config.update_check = enabled;
    info!("update check at startup set: {enabled}");
    if let Err(e) = config::save(&app, &data.config) {
        error!("failed to save config: {e}");
    }
}

/// Persist the updater's channel (Settings Save).
#[tauri::command]
fn set_update_channel(channel: config::UpdateChannel, app: AppHandle, data: State<Mutex<AppData>>) {
    let mut data = data.lock().unwrap();
    if data.config.update_channel == channel {
        return;
    }
    data.config.update_channel = channel;
    info!("update channel set: {channel:?}");
    if let Err(e) = config::save(&app, &data.config) {
        error!("failed to save config: {e}");
    }
}

/// Open a folder in the system file manager. Inside an AppImage the bundle
/// ships its own (build-host) `xdg-open` and puts `$APPDIR/usr/bin` first on
/// `PATH`; that `xdg-open`, and the file manager it launches, then load the
/// bundled glib and die (`undefined symbol …`), so "Open Folder" does nothing
/// — worse from a CI-built bundle than a locally built one. Bypass it: run the
/// *system* `xdg-open` (`PATH` with the `$APPDIR` entries dropped) with the
/// bundle's library paths cleared, so the file manager loads the host's
/// libraries. Everywhere else — dev build, other platforms, or a missing
/// `xdg-open` — fall back to the opener plugin, the old behaviour.
pub(crate) fn open_dir(dir: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    if let Some(appdir) = std::env::var_os("APPDIR") {
        let appdir = appdir.to_string_lossy();
        let system_path = std::env::var("PATH")
            .unwrap_or_default()
            .split(':')
            .filter(|p| !p.is_empty() && !p.starts_with(&*appdir))
            .collect::<Vec<_>>()
            .join(":");
        match std::process::Command::new("xdg-open")
            .arg(dir)
            .env("PATH", &system_path)
            .env_remove("LD_LIBRARY_PATH")
            .env_remove("LD_PRELOAD")
            .spawn()
        {
            Ok(_) => return Ok(()),
            Err(e) => log::warn!("system xdg-open failed ({e}); falling back to the opener plugin"),
        }
    }
    tauri_plugin_opener::open_path(dir, None::<&str>).map_err(|e| format!("open {}: {e}", dir.display()))
}

/// Open the app's log folder in the system file manager (Settings).
#[tauri::command]
fn open_log_dir(app: AppHandle) -> Result<(), String> {
    let dir = app.path().app_log_dir().map_err(|e| format!("app log dir: {e}"))?;
    open_dir(&dir)
}

/// Write a text file to a path the user picked in a save dialog (the Devices
/// log). Only an absolute path into an existing folder is accepted — the
/// dialog produces nothing else, anything else is not the dialog.
#[tauri::command]
fn write_text_file(path: String, text: String) -> Result<(), String> {
    let target = std::path::Path::new(&path);
    if !target.is_absolute() || path.contains('\0') || target.components().any(|c| c == std::path::Component::ParentDir) {
        return Err(format!("refusing to write to {path:?}"));
    }
    gamefile::write_atomic(target, text.as_bytes())?;
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

/// The SC token a live input resolves to (if the device is in the loaded
/// actionmaps.xml) and the action(s) bound to it.
#[derive(Default, Serialize)]
struct InputResolution {
    token: Option<String>,
    actions: Vec<bindings::BoundAction>,
}

/// Resolve a live input to its SC token and bound action(s). `kind` is
/// "button", "hat" or "axis"; `direction` is required for hats. The `jsN`
/// is the device's rank in SC's joystick order (`order.rs` — the saved
/// `<options>` slot may be stale), so without an order no joystick input
/// resolves. Empty for unknown or unlisted devices,
/// unbound inputs, or axes whose SC name is unknown (no usable HID
/// descriptor — see `DeviceInfo::axes_error`).
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
    if data.bindings_file.is_none() {
        return InputResolution::default();
    }
    let Some(sc_guid) = guid::sdl_guid_to_sc_product(&guid) else {
        return InputResolution::default();
    };
    let Some(instance) = data.device_order.as_ref().ok().and_then(|o| o.instance_for_guid(&sc_guid)) else {
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
/// against the current game data (bindings file + binding index), then
/// re-take the joystick order. Returns the actionmaps load status. Called
/// at start (once the game data is in), on a base-path change, after a
/// resort or restore, and on every Refresh. Game.log is not read here — the
/// watch thread does that, outside the lock.
pub(crate) fn reload_bindings(data: &mut AppData) -> LoadStatus {
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
        data.bindings_file = None;
        data.index = bindings::BindingIndex::default();
        data.device_order = Err("environment not loaded".into());
        data.game_log = data.device_order.clone();
        return status;
    }

    let profile = std::fs::read_to_string(&am_path)
        .map_err(|e| format!("actionmaps.xml not found: {} ({e})", status.actionmaps_path))
        .and_then(|xml| {
            scdata::parse_actionmaps(&xml).map_err(|e| format!("actionmaps.xml could not be parsed: {e}"))
        });
    match profile {
        Ok(profile) => {
            status.loaded = true;
            // Without game data there is nothing to resolve against (the
            // Status panel says why); the bindings file itself still serves
            // the clash report.
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
                "bindings loaded from {}: {} rebinds, {} resolved bindings, options: [{}]",
                status.actionmaps_path,
                profile.rebinds.len(),
                status.bindings.len(),
                options.join(", ")
            );
            data.bindings_file = Some(profile);
            data.bindings_error = None;
        }
        Err(e) => {
            warn!("bindings not loaded: {e}");
            status.error = Some(e.clone());
            data.index = bindings::BindingIndex::default();
            data.bindings_file = None;
            data.bindings_error = Some(e);
        }
    }
    // Game.log is the watch thread's business (read outside the lock); the
    // order is re-taken here so a reload never shows a stale one.
    let live = live_order(&data.devices);
    refresh_device_order(data, live);
    status
}

/// The outcome of the last `reload_bindings` without reading anything again —
/// for a frontend that mounts after the first load already finished (its
/// `scdata-changed` listener was not up yet).
#[tauri::command]
fn get_load_status(data: State<Mutex<AppData>>) -> LoadStatus {
    let data = data.lock().unwrap();
    LoadStatus {
        base_path: data.config.base_path().to_string(),
        actionmaps_path: config::actionmaps_path(data.config.base_path()).display().to_string(),
        loaded: data.bindings_file.is_some(),
        error: data.bindings_error.clone(),
        bindings: current_bindings(&data),
        sc: data.sc.status(),
    }
}

/// The log line for a joystick order (`source`: `Game.log` or `live`) — the
/// one thing remote troubleshooting of a `jsN` clash always needs.
fn log_order(source: &str, result: &Result<order::DeviceOrder, String>) {
    match result {
        Ok(o) => info!("{source} joystick order (as of {}): [{}]", o.timestamp.as_deref().unwrap_or("?"), o.describe()),
        Err(e) => warn!("{source} joystick order: {e}"),
    }
}

/// The platform's live joystick order for the devices currently listed
/// (`order::live`), to be taken before the `AppData` lock wherever possible:
/// DirectInput can take a while, the Wine replication reads sysfs.
fn live_order(devices: &input::DeviceList) -> Option<Result<order::DeviceOrder, String>> {
    let snapshot = devices.lock().map(|d| d.clone()).unwrap_or_default();
    order::live(&snapshot)
}

/// Re-take SC's joystick order: `live` is [`live_order`] (taken by the
/// caller, ideally before the lock) — only a changed outcome replaces the
/// snapshot (stamped with the time of the change) and is logged, so a
/// hot-plug of a device without a slot leaves it alone. A platform without
/// a live source (none built today) has no order.
fn refresh_device_order(data: &mut AppData, live: Option<Result<order::DeviceOrder, String>>) {
    let Some(fresh) = live else {
        data.device_order = Err("no joystick order source on this platform".into());
        return;
    };
    let same = match (&fresh, &data.device_order) {
        (Ok(a), Ok(b)) => a.joysticks == b.joysticks,
        (Err(a), Err(b)) => a == b,
        _ => false,
    };
    if same {
        return;
    }
    let fresh = fresh.map(|mut o| {
        o.timestamp = Some(backups::iso_utc(backups::now_secs()));
        o
    });
    log_order("live", &fresh);
    data.device_order = fresh;
}

/// Watch SC's `Game.log` for a new file (see [`logwatch`]): the game writes a
/// fresh log at every start, and its device order with it. A new file is
/// re-read for the whole settle window — the joystick lines arrive one by
/// one (200 ms apart on a two-stick setup) and a read may land between
/// them, so the first line found is not the order yet. Every changed
/// outcome replaces the snapshot (and the order source, where the log is
/// it) and `gamelog-changed` tells the frontend to redo the clash report,
/// `started` only with the first outcome of a new file. An environment
/// that is invalid or still loading is left alone (`reload_bindings` reads
/// then).
fn spawn_game_log_watch(app: AppHandle) {
    std::thread::spawn(move || {
        let mut watch = logwatch::Watch::default();
        let mut tail = logwatch::Tail::default();
        // Whether the frontend has been told about the current new file.
        let mut announced = false;
        loop {
            let state = app.state::<Mutex<AppData>>();
            // Only the facts are taken under the lock; every read of the
            // file happens without it — the input path must never wait for
            // a log to be read.
            let path = {
                let data = state.lock().unwrap();
                if data.sc.invalid_install {
                    drop(data);
                    std::thread::sleep(logwatch::POLL);
                    continue;
                }
                config::game_log_path(data.config.base_path())
            };
            let step = watch.poll(&path, logwatch::stamp(&path), std::time::Instant::now());
            let (started, settled) = match step {
                logwatch::Step::Idle => {
                    std::thread::sleep(logwatch::POLL);
                    continue;
                }
                logwatch::Step::Adopt => {
                    tail.reset();
                    (false, true)
                }
                logwatch::Step::Read { settled, new_file } => {
                    if new_file {
                        tail.reset();
                        announced = false;
                    }
                    (true, settled)
                }
            };
            let result = match tail.read(&path) {
                Ok(()) => gamelog::parse(&tail.text).ok_or_else(|| format!("{}: no joystick lines", path.display())),
                Err(e) => Err(format!("{}: {e}", path.display())),
            };
            // A new file gets its lines seconds after it appears: an error
            // before the window passes is not an outcome yet.
            if result.is_err() && !settled {
                std::thread::sleep(logwatch::POLL);
                continue;
            }
            let live = live_order(&app.state::<input::DeviceList>());
            let mut data = state.lock().unwrap();
            if result != data.game_log {
                info!("Game.log {}", if started { "replaced, re-read" } else { "read" });
                log_order("Game.log", &result);
                data.game_log = result;
                refresh_device_order(&mut data, live);
                drop(data);
                let first = started && !announced;
                announced = true;
                let _ = app.emit("gamelog-changed", GameLogChanged { started: first });
            }
            std::thread::sleep(logwatch::POLL);
        }
    });
}

/// Payload of `gamelog-changed`: `started` when the game wrote a new log
/// (it started), false for the read at start-up or after an environment
/// change.
#[derive(Clone, Serialize)]
struct GameLogChanged {
    started: bool,
}

/// Load the configured install's game data in the background (version from
/// `build_manifest.id`, then the cached JSON or a fresh extraction from
/// `Data.p4k`), reload the bindings against it and emit `scdata-changed`
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
            scinstall::load(&cache_root, &base_path, version, global_ini.as_deref(), &progress)
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
        let status = reload_bindings(&mut data);
        drop(data);
        let _ = app.emit("scdata-changed", &status);
        start_input(&app);
    });
}

/// Start the input thread once, after the first game-data load has ended
/// (whatever its outcome): devices are not enumerated and no input is
/// reported while the game files are still being read.
fn start_input(app: &AppHandle) {
    static INPUT_STARTED: std::sync::Once = std::sync::Once::new();
    INPUT_STARTED.call_once(|| {
        let devices = app.state::<input::DeviceList>().inner().clone();
        input::spawn(app.clone(), devices);
    });
}

/// Log everything about the environment that remote troubleshooting might
/// need: versions, platform, the app's directories and the loaded config.
/// The environment facts of the startup log, for the Device Info dumps: a
/// user sends those files, not bindsight.log.
#[derive(Serialize)]
struct SystemInfo {
    app_version: String,
    os: String,
    arch: String,
    tauri: String,
    webview: String,
    sdl: String,
    /// This build checks for and installs its own updates.
    updater: bool,
}

/// Whether this install updates itself: only the Windows installer and the
/// Linux AppImage. The bundler stamps the binary it packs with its bundle
/// type; the bare executable (the standalone build, `target/release/`) and
/// the deb / rpm packages (the package manager's business) get no updater.
fn updater_available() -> bool {
    use tauri::utils::config::BundleType;
    use tauri::utils::platform::bundle_type;
    matches!(bundle_type(), Some(BundleType::Nsis | BundleType::AppImage))
}

#[tauri::command]
fn system_info() -> SystemInfo {
    let os = os_info::get();
    let sdl = sdl2::version::version();
    SystemInfo {
        app_version: env!("CARGO_PKG_VERSION").into(),
        os: format!("{} {} ({})", os.os_type(), os.version(), os.bitness()),
        arch: std::env::consts::ARCH.into(),
        tauri: tauri::VERSION.into(),
        webview: tauri::webview_version().unwrap_or_else(|_| "?".into()),
        sdl: format!("{}.{}.{}", sdl.major, sdl.minor, sdl.patch),
        updater: updater_available(),
    }
}

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
        "config: active_env={} auto_backup={} debug_logging={} update_check={} update_channel={:?} environments={:?} imagemap_choices={:?}",
        config.active_env,
        config.auto_backup,
        config.debug_logging,
        config.update_check,
        config.update_channel,
        config.environments,
        config.imagemap_choices
    );
}

/// The window-close guard. Closing (top-bar button or window manager) has
/// to settle unsaved changes in the frontend first, so every
/// `CloseRequested` is prevented here and handed to the webview as our own
/// `close-requested` event. Not Tauri's `tauri://close-requested`: with a
/// JS listener on that one Tauri waits for the webview to close the
/// window itself, forever if the webview is gone (WebKit's web process
/// died, a blank window nothing could close). The webview acknowledges a
/// request at once (`ack_close`), then asks its dialogs and calls
/// `destroy`. No acknowledgement within `CLOSE_ACK_TIMEOUT` means the
/// webview is dead: the window is destroyed here, and should even that
/// leave the process running, the app exits.
struct CloseGuard {
    /// Counts the close requests; an acknowledgement names the request.
    requests: AtomicU64,
    /// The highest request the webview acknowledged.
    acked: AtomicU64,
}

const CLOSE_ACK_TIMEOUT: Duration = Duration::from_secs(2);
const CLOSE_EXIT_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Serialize)]
struct CloseRequest {
    request: u64,
}

#[tauri::command]
fn ack_close(request: u64, guard: State<CloseGuard>) {
    guard.acked.fetch_max(request, Ordering::SeqCst);
}

fn on_close_requested(window: &tauri::Window, api: &tauri::CloseRequestApi) {
    api.prevent_close();
    let guard = window.state::<CloseGuard>();
    let request = guard.requests.fetch_add(1, Ordering::SeqCst) + 1;
    debug!("close request {request}");
    if let Err(e) = window.emit("close-requested", CloseRequest { request }) {
        warn!("close request {request}: not delivered to the webview ({e}), destroying the window");
        destroy_or_exit(window.clone());
        return;
    }
    let window = window.clone();
    std::thread::spawn(move || {
        std::thread::sleep(CLOSE_ACK_TIMEOUT);
        if window.state::<CloseGuard>().acked.load(Ordering::SeqCst) < request {
            warn!(
                "close request {request}: the webview did not answer within {CLOSE_ACK_TIMEOUT:?}, destroying the window"
            );
            destroy_or_exit(window);
        }
    });
}

/// Destroys the window (no `CloseRequested` for that) and, if the process is
/// still around after `CLOSE_EXIT_TIMEOUT`, exits the app.
fn destroy_or_exit(window: tauri::Window) {
    if let Err(e) = window.destroy() {
        error!("window destroy failed: {e}");
    }
    std::thread::spawn(move || {
        std::thread::sleep(CLOSE_EXIT_TIMEOUT);
        warn!("window still alive {CLOSE_EXIT_TIMEOUT:?} after destroy, exiting");
        window.app_handle().exit(0);
    });
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

    let builder = tauri::Builder::default()
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
        // The updater's plugin side; the checks go through `update.rs`.
        .plugin(tauri_plugin_updater::Builder::new().build());
    builder
        .setup(|app| {
            app.manage(update::UpdateState::default());
            // A panic still ends the app (release builds abort), but this way
            // the reason reaches the log file first.
            std::panic::set_hook(Box::new(|info| error!("panic: {info}")));

            let config = config::load(app.handle());
            apply_log_level(config.debug_logging);
            log_startup(app.handle(), &config);
            // The device list exists from the start (commands read it), but
            // the input thread only starts once the first game-data load is
            // through (`spawn_sc_load` -> `start_input`).
            let devices: input::DeviceList = Arc::new(Mutex::new(Vec::new()));
            // The bindings file and Game.log are read once the game data is
            // in (`spawn_sc_load` -> `reload_bindings`).
            app.manage(Mutex::new(AppData {
                device_order: Err("not read yet".into()),
                game_log: Err("not read yet".into()),
                devices: devices.clone(),
                config,
                sc: ScState::default(),
                bindings_file: None,
                index: bindings::BindingIndex::default(),
                bindings_error: None,
                last_clash_log: String::new(),
            }));
            app.manage(devices);
            app.manage(CloseGuard { requests: AtomicU64::new(0), acked: AtomicU64::new(0) });
            spawn_sc_load(app.handle().clone());
            spawn_game_log_watch(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                on_close_requested(window, api);
            }
        })
        .invoke_handler(tauri::generate_handler![
            list_devices,
            wine_keys,
            hid_only_devices,
            ack_close,
            kblayout::keyboard_layout,
            system_info,
            get_actions,
            get_tokens,
            get_sc_status,
            get_config,
            get_bindings,
            get_load_status,
            reload,
            get_clash_report,
            apply_resort,
            apply_reorder,
            save_rebinds,
            get_current_bindings_info,
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
            textpath::text_path,
            textpath::list_fonts,
            binding_profiles::list_binding_profiles,
            binding_profiles::import_binding_profile,
            binding_profiles::export_binding_profile,
            binding_profiles::save_binding_profile,
            binding_profiles::delete_binding_profile,
            binding_profiles::open_binding_profiles_dir,
            apply::apply_bindings,
            backups::list_backups,
            backups::create_backup,
            backups::delete_backup,
            backups::restore_backup,
            backups::open_backup_dir,
            backups::open_backups_dir,
            set_auto_backup,
            set_debug_logging,
            set_update_check,
            set_update_channel,
            update::check_update,
            update::install_update,
            diff::compare_bindings
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
