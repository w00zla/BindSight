//! Persistent app config: the SC base path (the folder that contains
//! `Data.p4k`), stored as JSON in the app config dir. Everything the app needs
//! from the SC install is derived from this one path.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

/// Default SC base path — the standard Windows install. Users on other setups
/// (e.g. Linux/Wine) point it at their own channel folder (LIVE/PTU/…).
const DEFAULT_BASE_PATH: &str = r"C:\Program Files\Roberts Space Industries\StarCitizen\LIVE";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub base_path: String,
    /// SC Product GUIDs of connected devices the user declared invisible to SC
    /// ("SC doesn't see this device"), e.g. a keyboard that Wine hides from the
    /// game. Lives in the per-OS config dir, so the list is naturally
    /// platform-specific — the same device can be visible on Windows and
    /// hidden under Wine. Older config files without the field still load.
    #[serde(default)]
    pub ignored_devices: Vec<String>,
    /// Which hardware profile to show per device: lowercase SC Product GUID
    /// -> profile id. Only needed when several profiles exist for one device.
    #[serde(default)]
    pub profile_choices: HashMap<String, String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            base_path: DEFAULT_BASE_PATH.to_string(),
            ignored_devices: Vec::new(),
            profile_choices: HashMap::new(),
        }
    }
}

fn config_file(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|dir| dir.join("config.json"))
}

/// Load the config, or the default if none is stored / it cannot be read.
pub fn load(app: &AppHandle) -> Config {
    config_file(app)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Persist the config to the app config dir.
pub fn save(app: &AppHandle, config: &Config) -> Result<(), String> {
    let path = config_file(app).ok_or("no config directory available")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_without_ignored_devices_still_loads() {
        // Config files written before the field existed.
        let c: Config = serde_json::from_str(r#"{"base_path":"/sc/LIVE"}"#).unwrap();
        assert_eq!(c.base_path, "/sc/LIVE");
        assert!(c.ignored_devices.is_empty());
        assert!(c.profile_choices.is_empty());
    }
}
