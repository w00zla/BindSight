//! Persistent app config: the SC environments (one base path — the folder
//! that contains `Data.p4k` — per channel LIVE / HOTFIX / PTU / EPTU, plus an
//! optional `global.ini` override each) and which one is active, stored as
//! JSON in the app config dir. Everything the app needs from the SC install
//! is derived from the active environment's path.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

/// The SC channels, in GUI order. The config always holds all four.
pub const ENVIRONMENTS: [&str; 4] = ["LIVE", "HOTFIX", "PTU", "EPTU"];

/// Default install root — the standard Windows install; the channel folder
/// is appended. Users on other setups (e.g. Linux/Wine) point each
/// environment at their own folder.
const DEFAULT_INSTALL_ROOT: &str = r"C:\Program Files\Roberts Space Industries\StarCitizen";

/// One SC channel install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Environment {
    /// The folder that holds `Data.p4k`.
    pub path: String,
    /// Take the action labels from `global_ini` instead of the install's own
    /// `global.ini` (e.g. a community translation).
    #[serde(default)]
    pub global_ini_override: bool,
    /// Path of the override file; kept even while the override is off.
    #[serde(default)]
    pub global_ini: String,
}

impl Environment {
    fn default_for(slug: &str) -> Self {
        Self {
            path: format!("{DEFAULT_INSTALL_ROOT}\\{slug}"),
            global_ini_override: false,
            global_ini: String::new(),
        }
    }
}

fn default_environments() -> BTreeMap<String, Environment> {
    ENVIRONMENTS.iter().map(|s| (s.to_string(), Environment::default_for(s))).collect()
}

fn default_active_env() -> String {
    ENVIRONMENTS[0].to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Keyed by slug (see [`ENVIRONMENTS`]); missing entries are filled with
    /// defaults on load.
    #[serde(default = "default_environments")]
    pub environments: BTreeMap<String, Environment>,
    /// Slug of the environment the app reads.
    #[serde(default = "default_active_env")]
    pub active_env: String,
    /// Which image-map to show per device: lowercase SC Product GUID -> image-map
    /// id. Only needed when several image-maps exist for one device.
    #[serde(default)]
    pub imagemap_choices: HashMap<String, String>,
    /// Back up `actionmaps.xml` before BindSight overwrites it (Fix via config,
    /// before rebind, restore). On unless the user switched it off.
    #[serde(default = "default_auto_backup")]
    pub auto_backup: bool,
    /// Write DEBUG records to the app log; INFO and up otherwise.
    #[serde(default)]
    pub debug_logging: bool,
    /// Check for an update at startup (the App Update dialog opens when one
    /// is found; nothing is ever installed unasked). On unless the user
    /// switched it off; a manual check stays possible either way.
    #[serde(default = "default_update_check")]
    pub update_check: bool,
    /// Which release feed the updater reads (see `update.rs`).
    #[serde(default)]
    pub update_channel: UpdateChannel,
}

/// The updater's channel: stable = GitHub's latest full release, beta =
/// the newest published release including pre-releases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateChannel {
    #[default]
    Stable,
    Beta,
}

fn default_auto_backup() -> bool {
    true
}

fn default_update_check() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            environments: default_environments(),
            active_env: default_active_env(),
            imagemap_choices: HashMap::new(),
            auto_backup: default_auto_backup(),
            debug_logging: false,
            update_check: default_update_check(),
            update_channel: UpdateChannel::Stable,
        }
    }
}

impl Config {
    /// Make sure every known environment exists and the active one is valid.
    fn normalize(mut self) -> Self {
        for slug in ENVIRONMENTS {
            self.environments
                .entry(slug.to_string())
                .or_insert_with(|| Environment::default_for(slug));
        }
        if !ENVIRONMENTS.contains(&self.active_env.as_str()) {
            self.active_env = default_active_env();
        }
        self
    }

    /// The active environment (always present after [`load`]).
    pub fn active(&self) -> &Environment {
        self.environments
            .get(&self.active_env)
            .or_else(|| self.environments.get(ENVIRONMENTS[0]))
            .expect("config holds every environment")
    }

    /// Base path of the active environment.
    pub fn base_path(&self) -> &str {
        &self.active().path
    }

    /// The active environment's `global.ini` override, when switched on.
    pub fn global_ini_override(&self) -> Option<PathBuf> {
        let env = self.active();
        (env.global_ini_override && !env.global_ini.is_empty()).then(|| PathBuf::from(&env.global_ini))
    }
}

fn config_file(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|dir| dir.join("config.json"))
}

/// Load the config, or the default if none is stored / it cannot be read.
pub fn load(app: &AppHandle) -> Config {
    config_file(app)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<Config>(&text).ok())
        .unwrap_or_default()
        .normalize()
}

/// Persist the config to the app config dir.
pub fn save(app: &AppHandle, config: &Config) -> Result<(), String> {
    let path = config_file(app).ok_or("no config directory available")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let json = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("{}: {e}", path.display()))
}

/// Path to the user's live `actionmaps.xml`, derived from the SC base path.
/// (Note: `controls/mappings/` holds only manually exported layouts — the live
/// bindings live under `Profiles/default/`.)
pub fn actionmaps_path(base_path: &str) -> PathBuf {
    PathBuf::from(base_path)
        .join("user")
        .join("client")
        .join("0")
        .join("Profiles")
        .join("default")
        .join("actionmaps.xml")
}

/// Path to SC's `Game.log`, written next to `Data.p4k` at each game start. SC
/// rotates the previous one into `logbackups/`.
pub fn game_log_path(base_path: &str) -> PathBuf {
    PathBuf::from(base_path).join("Game.log")
}

/// Directory holding SC's exported keybinding layouts (binding profiles,
/// manually saved via the options menu), *not* the live bindings — those
/// live under `Profiles/default/` (see [`actionmaps_path`]). This is SC's own
/// `controls/mappings/` folder.
pub fn binding_profiles_dir(base_path: &str) -> PathBuf {
    PathBuf::from(base_path)
        .join("user")
        .join("client")
        .join("0")
        .join("controls")
        .join("mappings")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_hold_every_environment_with_live_active() {
        let c = Config::default();
        assert_eq!(c.active_env, "LIVE");
        assert_eq!(c.environments.len(), ENVIRONMENTS.len());
        assert_eq!(c.base_path(), r"C:\Program Files\Roberts Space Industries\StarCitizen\LIVE");
        assert_eq!(c.environments["PTU"].path, r"C:\Program Files\Roberts Space Industries\StarCitizen\PTU");
        assert!(c.global_ini_override().is_none());
        assert!(c.auto_backup);
    }

    #[test]
    fn an_old_exclusion_list_is_ignored() {
        // The Exclude feature (0.10 - 0.12) stored `ignored_devices`; a config
        // written back then still loads.
        let json = r#"{"ignored_devices":["gamepad","{0201231D-0000-0000-0000-504944564944}"],"active_env":"PTU"}"#;
        let c = serde_json::from_str::<Config>(json).unwrap().normalize();
        assert_eq!(c.active_env, "PTU");
    }

    #[test]
    fn partial_config_is_filled_and_override_resolves() {
        let json = r#"{"environments":{"PTU":{"path":"/sc/PTU","global_ini_override":true,"global_ini":"/x/global.ini"}},"active_env":"PTU"}"#;
        let c = serde_json::from_str::<Config>(json).unwrap().normalize();
        assert_eq!(c.environments.len(), ENVIRONMENTS.len());
        assert_eq!(c.base_path(), "/sc/PTU");
        assert_eq!(c.global_ini_override(), Some(PathBuf::from("/x/global.ini")));
        // Auto-backups are on unless switched off explicitly; debug logging is
        // off unless switched on.
        assert!(c.auto_backup);
        assert!(!serde_json::from_str::<Config>(r#"{"auto_backup":false}"#).unwrap().auto_backup);
        assert!(!c.debug_logging);
        assert!(serde_json::from_str::<Config>(r#"{"debug_logging":true}"#).unwrap().debug_logging);
        assert!(c.update_check);
        assert!(!serde_json::from_str::<Config>(r#"{"update_check":false}"#).unwrap().update_check);
        assert_eq!(c.update_channel, UpdateChannel::Stable);
        assert_eq!(serde_json::from_str::<Config>(r#"{"update_channel":"beta"}"#).unwrap().update_channel, UpdateChannel::Beta);
        assert!(serde_json::from_str::<Config>(r#"{"update_channel":"nightly"}"#).is_err());
        assert_eq!(serde_json::to_value(UpdateChannel::Beta).unwrap(), "beta");

        // An unknown active slug falls back to LIVE; an override that is off
        // or has no path is none.
        let c = serde_json::from_str::<Config>(r#"{"active_env":"NOPE"}"#).unwrap().normalize();
        assert_eq!(c.active_env, "LIVE");
        assert!(c.global_ini_override().is_none());
    }

    #[test]
    fn binding_profiles_dir_is_under_controls_not_profiles() {
        assert_eq!(
            binding_profiles_dir("/sc/LIVE"),
            PathBuf::from("/sc/LIVE/user/client/0/controls/mappings")
        );
    }
}
