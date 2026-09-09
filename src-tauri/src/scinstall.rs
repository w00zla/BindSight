//! The configured SC install: its version (from `build_manifest.id`) and the
//! game data BindSight needs from it — the action master list and the input
//! token labels. Both are pulled out of `Data.p4k` with the bundled StarBreaker
//! sidecar, converted to compact JSON (the app never parses the raw XML/INI
//! twice) and cached per game version under the app cache dir, so a game
//! patch is picked up automatically and older installs (PTU/LIVE) stay apart.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;

use log::{debug, info, warn};
use serde::{Deserialize, Serialize};

use crate::scdata::{self, ActionMap};

/// Files inside `Data.p4k` (forward or backward slashes, StarBreaker is not
/// consistent between platforms), extracted in one run.
const P4K_REGEX: &str = r"^Data[\\/]Libs[\\/]Config[\\/](defaultProfile|keybinding_localization)\.xml$|^Data[\\/]Localization[\\/]english[\\/]global\.ini$";

const ACTIONS_FILE: &str = "scdata.json";
const TOKENS_FILE: &str = "tokens.json";

/// Shape of the cached JSON. Bump it whenever `ActionMap` / `Action` or the
/// token map change meaning, so an older cache is re-extracted instead of
/// loading with silently missing fields (serde fills a missing `Option`
/// with `None`). 2 = keyboard / gamepad defaults and `kb1_` / `gp1_` labels.
const CACHE_FORMAT: u32 = 2;

/// `scdata.json`: the action master list behind its format stamp.
#[derive(Serialize, Deserialize)]
struct CachedActions {
    format: u32,
    actionmaps: Vec<ActionMap>,
}

/// Display labels for input tokens (e.g. `"button9"` -> `"Button 9"`).
pub type TokenLabels = HashMap<String, String>;

/// Version of an SC install, from its `build_manifest.id`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScVersion {
    /// Display label and cache key: the branch without its `sc-alpha-`
    /// prefix, then the Perforce changelist — e.g. `4.10.0-hotfix.12572603`.
    pub label: String,
    pub branch: String,
    pub version: String,
    pub changelist: String,
    pub build_id: String,
}

#[derive(Deserialize)]
struct Manifest {
    #[serde(rename = "Data")]
    data: ManifestData,
}

#[derive(Deserialize)]
struct ManifestData {
    #[serde(rename = "Branch")]
    branch: String,
    #[serde(rename = "Version")]
    version: String,
    #[serde(rename = "RequestedP4ChangeNum")]
    changelist: String,
    #[serde(rename = "BuildId", default)]
    build_id: String,
}

/// The converted game data.
#[derive(Debug, Clone, Default)]
pub struct ScData {
    pub actions: Vec<ActionMap>,
    pub tokens: TokenLabels,
}

/// Files an SC environment must have, relative to its base path. Checked
/// before anything is read; one missing = the environment is not loaded.
pub const REQUIRED_FILES: [&str; 3] = [
    "Data.p4k",
    "build_manifest.id",
    "user/client/0/Profiles/default/actionmaps.xml",
];

/// Check that `base_path` is a folder holding every [`REQUIRED_FILES`]
/// entry. The error names the folder, or lists what is missing in it.
pub fn validate_install(base_path: &str) -> Result<(), String> {
    let base = Path::new(base_path);
    if !base.is_dir() {
        return Err(format!("Folder not found: {base_path}"));
    }
    let missing: Vec<&str> = REQUIRED_FILES.iter().copied().filter(|f| !base.join(f).is_file()).collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("Missing in {base_path}: {}", missing.join(", ")))
    }
}

/// `build_manifest.id`, written next to `Data.p4k` by the launcher.
pub fn manifest_path(base_path: &str) -> PathBuf {
    PathBuf::from(base_path).join("build_manifest.id")
}

pub fn p4k_path(base_path: &str) -> PathBuf {
    PathBuf::from(base_path).join("Data.p4k")
}

/// Parse a `build_manifest.id`.
pub fn parse_manifest(json: &str) -> Result<ScVersion, String> {
    let m: Manifest = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let d = m.data;
    if d.branch.is_empty() || d.changelist.is_empty() {
        return Err("Branch or RequestedP4ChangeNum missing".into());
    }
    let short = d.branch.strip_prefix("sc-alpha-").unwrap_or(&d.branch);
    Ok(ScVersion {
        label: format!("{short}.{}", d.changelist),
        branch: d.branch.clone(),
        version: d.version,
        changelist: d.changelist,
        build_id: d.build_id,
    })
}

/// Read the install's version from its `build_manifest.id`.
pub fn read_version(base_path: &str) -> Result<ScVersion, String> {
    let path = manifest_path(base_path);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let version = parse_manifest(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    info!(
        "sc version read from {}: label={} branch={} version={} changelist={} build_id={}",
        path.display(),
        version.label,
        version.branch,
        version.version,
        version.changelist,
        version.build_id
    );
    Ok(version)
}

/// Path of the StarBreaker sidecar: Tauri places `externalBin` binaries next
/// to the app executable (also in `target/debug` during development).
pub fn sidecar_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("current exe: {e}"))?;
    let dir = exe.parent().ok_or("current exe has no parent dir")?;
    let name = if cfg!(windows) { "starbreaker.exe" } else { "starbreaker" };
    let path = dir.join(name);
    if !path.is_file() {
        return Err(format!("StarBreaker not found: {}", path.display()));
    }
    debug!("sidecar resolved: {}", path.display());
    Ok(path)
}

/// Cache dir for one game version.
fn cache_dir(cache_root: &Path, version: &ScVersion) -> PathBuf {
    // The label comes from the manifest; keep it a plain single path segment.
    let safe: String = version
        .label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
        .collect();
    cache_root.join(safe)
}

fn read_cache(dir: &Path) -> Option<ScData> {
    let actions = std::fs::read_to_string(dir.join(ACTIONS_FILE)).ok()?;
    let tokens = std::fs::read_to_string(dir.join(TOKENS_FILE)).ok()?;
    let actions: CachedActions = match serde_json::from_str(&actions) {
        Ok(a) => a,
        Err(e) => {
            warn!("sc data cache unreadable, rebuilding: {}: {e}", dir.display());
            return None;
        }
    };
    if actions.format != CACHE_FORMAT {
        info!("sc data cache format {} != {CACHE_FORMAT}, rebuilding: {}", actions.format, dir.display());
        return None;
    }
    let tokens = match serde_json::from_str(&tokens) {
        Ok(t) => t,
        Err(e) => {
            warn!("sc data cache unreadable, rebuilding: {}: {e}", dir.display());
            return None;
        }
    };
    Some(ScData { actions: actions.actionmaps, tokens })
}

fn write_cache(dir: &Path, data: &ScData) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let cached = CachedActions { format: CACHE_FORMAT, actionmaps: data.actions.clone() };
    let actions = serde_json::to_string(&cached).map_err(|e| e.to_string())?;
    // Sorted for stable output.
    let sorted: BTreeMap<_, _> = data.tokens.iter().collect();
    let tokens = serde_json::to_string(&sorted).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(ACTIONS_FILE), actions).map_err(|e| format!("{ACTIONS_FILE}: {e}"))?;
    std::fs::write(dir.join(TOKENS_FILE), tokens).map_err(|e| format!("{TOKENS_FILE}: {e}"))
}

/// Run StarBreaker to pull the three config files out of `Data.p4k` into
/// `out` (it recreates the archive's directory tree there). Returns the
/// paths of `defaultProfile.xml`, `keybinding_localization.xml`, `global.ini`.
fn extract(sidecar: &Path, p4k: &Path, out: &Path) -> Result<[PathBuf; 3], String> {
    if !p4k.is_file() {
        return Err(format!("Data.p4k not found: {}", p4k.display()));
    }
    let mut cmd = Command::new(sidecar);
    cmd.arg("p4k")
        .arg("extract")
        .arg("--p4k")
        .arg(p4k)
        .arg("-o")
        .arg(out)
        .arg("--regex")
        .arg(P4K_REGEX)
        .arg("--convert")
        .arg("cryxml");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    debug!("sc data extract command: {:?}", cmd);
    let output = cmd.output().map_err(|e| format!("run {}: {e}", sidecar.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = stderr.trim().lines().last().or_else(|| stdout.trim().lines().last()).unwrap_or("");
        warn!("starbreaker failed ({}): {}", output.status, stderr.trim());
        return Err(format!("StarBreaker failed ({}): {detail}", output.status));
    }

    let files = [
        out.join("Data").join("Libs").join("Config").join("defaultProfile.xml"),
        out.join("Data").join("Libs").join("Config").join("keybinding_localization.xml"),
        out.join("Data").join("Localization").join("english").join("global.ini"),
    ];
    for f in &files {
        if !f.is_file() {
            return Err(format!("not in Data.p4k: {}", f.strip_prefix(out).unwrap_or(f).display()));
        }
    }
    Ok(files)
}

/// Convert the raw SC files into the app's data: the action master list
/// (unlabeled actions are SC-internal and dropped, as are actionmaps left
/// empty) and the input token labels.
pub fn convert(profile_xml: &str, keybinding_xml: &str, global_ini: &str) -> Result<ScData, String> {
    let loc = scdata::parse_localization(global_ini);
    let mut actions = scdata::parse_default_profile(profile_xml, &loc)?;
    for map in &mut actions {
        map.actions.retain(|a| a.label.is_some());
    }
    actions.retain(|map| !map.actions.is_empty());
    let tokens = scdata::parse_token_labels(keybinding_xml, &loc);
    Ok(ScData { actions, tokens })
}

/// Number of progress steps a full load reports (see [`load`]); step 1 is
/// the manifest read the caller does before calling `load`.
pub const LOAD_STEPS: u8 = 4;

/// Load the game data for the install at `base_path` (whose version is
/// `version`): serve the cached JSON for that version if present, otherwise
/// extract + convert and fill the cache. `cache_root` is the app cache dir.
/// With a `global_ini` override the labels come from that file instead of
/// the install's own, and the cache is bypassed both ways (the file can
/// change any time; the extraction costs about a second). `progress` is
/// called with the completed step (2 = extracted, 3 = converted, 4 =
/// cached; a cache hit jumps straight to 4).
pub fn load(
    cache_root: &Path,
    sidecar: &Path,
    base_path: &str,
    version: &ScVersion,
    global_ini: Option<&Path>,
    progress: &dyn Fn(u8),
) -> Result<ScData, String> {
    let dir = cache_dir(cache_root, version);
    if global_ini.is_none() {
        if let Some(data) = read_cache(&dir) {
            info!("sc data cache hit: {}", dir.display());
            progress(LOAD_STEPS);
            return Ok(data);
        }
    }
    let p4k = p4k_path(base_path);
    match global_ini {
        Some(ini) => info!("sc data with global.ini override, extracting: ini={} p4k={}", ini.display(), p4k.display()),
        None => info!("sc data cache miss, extracting: cache_dir={} p4k={}", dir.display(), p4k.display()),
    }

    let tmp = dir.join("extract");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    let extract_start = std::time::Instant::now();
    let extracted = extract(sidecar, &p4k, &tmp);
    let extract_elapsed = extract_start.elapsed();
    let result = extracted.and_then(|[profile, keybinding, global]| {
        progress(2);
        info!("sc data extracted in {:.2?}", extract_elapsed);
        let read = |p: &Path| std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()));
        let global = global_ini.unwrap_or(&global);
        let data = convert(&read(&profile)?, &read(&keybinding)?, &read(global)?)?;
        let total_actions: usize = data.actions.iter().map(|m| m.actions.len()).sum();
        info!(
            "sc data converted: {} actionmaps, {} actions, {} tokens",
            data.actions.len(),
            total_actions,
            data.tokens.len()
        );
        Ok(data)
    });
    let _ = std::fs::remove_dir_all(&tmp);

    let data = result?;
    progress(3);
    if global_ini.is_none() {
        write_cache(&dir, &data)?;
        info!("sc data cache written: {}", dir.display());
    }
    progress(LOAD_STEPS);
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"{
    "Data": {
        "Branch": "sc-alpha-4.10.0-hotfix",
        "BuildDateStamp": "Thu Sep 03 2026",
        "BuildId": "ef2d1d8a-ba83-4f42-871f-4f3c400040c5",
        "BuildTimeStamp": "03:13:41 PM CST",
        "Config": "shipping",
        "Platform": "pc",
        "RequestedP4ChangeNum": "12572603",
        "Shelved_Change": "",
        "Tag": "public",
        "Version": "1.0.191.55227"
    }
}"#;

    #[test]
    fn manifest_parses_and_labels() {
        let v = parse_manifest(MANIFEST).unwrap();
        assert_eq!(v.label, "4.10.0-hotfix.12572603");
        assert_eq!(v.branch, "sc-alpha-4.10.0-hotfix");
        assert_eq!(v.version, "1.0.191.55227");
        assert_eq!(v.changelist, "12572603");
        assert_eq!(v.build_id, "ef2d1d8a-ba83-4f42-871f-4f3c400040c5");
    }

    #[test]
    fn label_keeps_unknown_branch_prefix() {
        let json = MANIFEST.replace("sc-alpha-4.10.0-hotfix", "ptu-4.11.0");
        assert_eq!(parse_manifest(&json).unwrap().label, "ptu-4.11.0.12572603");
    }

    #[test]
    fn manifest_without_branch_is_an_error() {
        let json = MANIFEST.replace(r#""Branch": "sc-alpha-4.10.0-hotfix","#, "");
        assert!(parse_manifest(&json).is_err());
        assert!(parse_manifest("not json").is_err());
    }

    #[test]
    fn cache_dir_is_versioned_and_sanitized() {
        let mut v = parse_manifest(MANIFEST).unwrap();
        assert_eq!(cache_dir(Path::new("/c"), &v), Path::new("/c/4.10.0-hotfix.12572603"));
        v.label = "a/b\\c d".into();
        assert_eq!(cache_dir(Path::new("/c"), &v), Path::new("/c/a_b_c_d"));
    }

    #[test]
    fn validate_install_names_folder_or_missing_files() {
        let dir = std::env::temp_dir().join(format!("bindsight-validate-{}", uuid::Uuid::new_v4()));
        let base = dir.to_string_lossy().to_string();
        assert_eq!(validate_install(&base), Err(format!("Folder not found: {base}")));

        std::fs::create_dir_all(dir.join("user/client/0/Profiles/default")).unwrap();
        std::fs::write(dir.join("Data.p4k"), b"").unwrap();
        assert_eq!(
            validate_install(&base),
            Err(format!("Missing in {base}: build_manifest.id, user/client/0/Profiles/default/actionmaps.xml"))
        );

        std::fs::write(dir.join("build_manifest.id"), b"").unwrap();
        std::fs::write(dir.join("user/client/0/Profiles/default/actionmaps.xml"), b"").unwrap();
        assert_eq!(validate_install(&base), Ok(()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cache_roundtrip() {
        let dir = std::env::temp_dir().join(format!("bindsight-scinstall-{}", uuid::Uuid::new_v4()));
        assert!(read_cache(&dir).is_none());
        let data = ScData {
            actions: vec![ActionMap {
                name: "spaceship_general".into(),
                label: Some("Flight".into()),
                actions: vec![],
            }],
            tokens: HashMap::from([("button1".to_string(), "Button 1".to_string())]),
        };
        write_cache(&dir, &data).unwrap();
        let back = read_cache(&dir).unwrap();
        assert_eq!(back.actions.len(), 1);
        assert_eq!(back.actions[0].name, "spaceship_general");
        assert_eq!(back.tokens["button1"], "Button 1");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cache_with_another_format_or_shape_is_rebuilt() {
        let dir = std::env::temp_dir().join(format!("bindsight-scinstall-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(TOKENS_FILE), b"{}").unwrap();
        // Format 1 wrote the bare array.
        std::fs::write(dir.join(ACTIONS_FILE), b"[]").unwrap();
        assert!(read_cache(&dir).is_none());
        std::fs::write(dir.join(ACTIONS_FILE), format!(r#"{{"format":{},"actionmaps":[]}}"#, CACHE_FORMAT + 1)).unwrap();
        assert!(read_cache(&dir).is_none());
        std::fs::write(dir.join(ACTIONS_FILE), format!(r#"{{"format":{CACHE_FORMAT},"actionmaps":[]}}"#)).unwrap();
        assert!(read_cache(&dir).is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn extract_reports_missing_p4k() {
        let err = extract(Path::new("/nonexistent/starbreaker"), Path::new("/nonexistent/Data.p4k"), Path::new("/tmp"))
            .unwrap_err();
        assert!(err.contains("Data.p4k not found"), "{err}");
    }
}
