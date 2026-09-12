//! Backups of the live `actionmaps.xml`: one folder per backup under
//! `<app_data_dir>/backups/<id>/`, holding a copy of the file (`actionmaps.xml`)
//! plus `meta.json` (when it was made, why, and for which game version). Taken
//! manually (Bindings mode) and, always, before every write to the live file
//! (`gamefile::replace_live_file`: rebind, apply, order fix) and before a
//! restore (so a restore is itself undoable). A backup is verified byte for
//! byte against its source before it counts as made.
//!
//! The pure logic works on `&Path` roots so it is testable without an
//! `AppHandle`; the `#[tauri::command]` wrappers only resolve the root.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::{binding_profiles, config, gamefile, scdata, AppData, LoadStatus};

const META_FILE: &str = "meta.json";
const XML_FILE: &str = "actionmaps.xml";

/// `meta.json` contents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupMeta {
    /// Unix seconds.
    pub created: u64,
    pub reason: String,
    /// `ScVersion::label` of the install loaded when the backup was made;
    /// `None` when none was loaded, or for backups made before it was recorded.
    #[serde(default)]
    pub game_version: Option<String>,
}

/// Listing entry for one backup.
#[derive(Debug, Clone, Serialize)]
pub struct BackupSummary {
    /// Folder name (see [`format_timestamp`]).
    pub id: String,
    pub created: u64,
    pub reason: String,
    pub game_version: Option<String>,
    /// Bindings in the backed-up file across all devices, per
    /// [`binding_profiles::summarize`].
    pub bindings: usize,
}

/// True for a plain folder name that cannot escape `root`: non-empty, no path
/// separators, not `.`/`..`.
fn is_bare_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
        // `C:x` is drive-relative on Windows and would leave the root.
        && !name.contains(':')
}

/// Civil (proleptic Gregorian) date from a day count since the Unix epoch.
/// Howard Hinnant's `civil_from_days`; only ever called with non-negative
/// `days` here (unix seconds are always >= 0), but handles negative input too.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as u32, d as u32)
}

/// `created` (unix seconds) formatted as `YYYYMMDD-HHMMSS` in UTC. Used as the
/// backup folder id.
fn format_timestamp(created: u64) -> String {
    let secs = created as i64;
    let days = secs.div_euclid(86400);
    let tod = secs.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", tod / 3600, (tod % 3600) / 60, tod % 60)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Create `root/base`, or `root/base-2`, `base-3`, … — the first folder that
/// did not exist. Creating (not checking) is what makes two backups in the
/// same second, even from two threads, land in two folders.
fn create_unique_dir(root: &Path, base: &str) -> Result<String, String> {
    fs::create_dir_all(root).map_err(|e| format!("{}: {e}", root.display()))?;
    let candidates = std::iter::once(base.to_string()).chain((2..1000).map(|n| format!("{base}-{n}")));
    for id in candidates {
        match fs::create_dir(root.join(&id)) {
            Ok(()) => return Ok(id),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("{}: {e}", root.join(&id).display())),
        }
    }
    Err(format!("{base}: too many backups in one second"))
}

/// Read one backup folder's `meta.json` + the bindings count of its
/// `actionmaps.xml`.
fn read_summary(dir: &Path, id: &str, actions: &[scdata::ActionMap]) -> Result<BackupSummary, String> {
    let meta_path = dir.join(META_FILE);
    let text = fs::read_to_string(&meta_path).map_err(|e| format!("{}: {e}", meta_path.display()))?;
    let meta: BackupMeta = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", meta_path.display()))?;
    let summary = binding_profiles::summarize(&dir.join(XML_FILE), actions)?;
    Ok(BackupSummary {
        id: id.to_string(),
        created: meta.created,
        reason: meta.reason,
        game_version: meta.game_version,
        bindings: summary.bindings,
    })
}

/// Copy `actionmaps` into a fresh backup folder under `root`, with `reason`
/// (trimmed; empty becomes `"manual"`) and `game_version` recorded in
/// `meta.json`.
pub fn create(
    root: &Path,
    actionmaps: &Path,
    reason: &str,
    game_version: Option<&str>,
    actions: &[scdata::ActionMap],
) -> Result<BackupSummary, String> {
    if !actionmaps.is_file() {
        return Err(format!("{}: not found", actionmaps.display()));
    }
    let reason = {
        let r = reason.trim();
        if r.is_empty() { "manual" } else { r }
    };
    let created = now_secs();
    let id = create_unique_dir(root, &format_timestamp(created))?;
    let dir = root.join(&id);
    // A backup counts only once its bytes are on disk and identical to the
    // source; a half-made folder is removed again so it never lists.
    let made = (|| {
        let source = fs::read(actionmaps).map_err(|e| format!("{}: {e}", actionmaps.display()))?;
        gamefile::write_atomic(&dir.join(XML_FILE), &source)?;
        let meta = BackupMeta { created, reason: reason.to_string(), game_version: game_version.map(str::to_string) };
        let json = serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?;
        gamefile::write_atomic(&dir.join(META_FILE), json.as_bytes())?;
        read_summary(&dir, &id, actions)
    })();
    if made.is_err() {
        let _ = fs::remove_dir_all(&dir);
    }
    made
}

/// Every readable backup under `root`, newest first (then by id descending).
/// Folders missing `meta.json` or an unparsable `actionmaps.xml` are logged
/// and skipped; a missing `root` yields an empty list.
pub fn list(root: &Path, actions: &[scdata::ActionMap]) -> Vec<BackupSummary> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        let Some(id) = dir.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !dir.is_dir() {
            continue;
        }
        match read_summary(&dir, id, actions) {
            Ok(s) => out.push(s),
            Err(e) => warn!("skipping backup {id}: {e}"),
        }
    }
    out.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| b.id.cmp(&a.id)));
    out
}

/// The `actionmaps.xml` inside backup `id`. Errors on a bad id or a missing
/// backup.
pub fn path_of(root: &Path, id: &str) -> Result<PathBuf, String> {
    if !is_bare_name(id) {
        return Err(format!("invalid backup id {id:?}"));
    }
    let path = root.join(id).join(XML_FILE);
    if !path.is_file() {
        return Err(format!("unknown backup {id:?}"));
    }
    Ok(path)
}

/// Delete a backup folder.
pub fn delete(root: &Path, id: &str) -> Result<(), String> {
    if !is_bare_name(id) {
        return Err(format!("invalid backup id {id:?}"));
    }
    fs::remove_dir_all(root.join(id)).map_err(|e| format!("{id}: {e}"))
}

/// Restore backup `id` over the live `actionmaps`, byte for byte: the backup
/// must still parse as an `actionmaps.xml` (a broken file is never written
/// over a working one), a safety backup of the current file is made first
/// (reason `"before restore"`) unless `actionmaps` doesn't exist yet, then
/// the backup's bytes replace it atomically. Never touches the backup
/// itself. Returns the safety backup's summary, or `None` when nothing
/// existed to back up.
pub fn restore(
    root: &Path,
    id: &str,
    actionmaps: &Path,
    game_version: Option<&str>,
    actions: &[scdata::ActionMap],
) -> Result<Option<BackupSummary>, String> {
    let backup_path = path_of(root, id)?;
    let bytes = fs::read(&backup_path).map_err(|e| format!("{}: {e}", backup_path.display()))?;
    scdata::parse_actionmaps(&String::from_utf8_lossy(&bytes))
        .map_err(|e| format!("backup {id} is not a readable bindings file, not restored: {e}"))?;
    let safety = if actionmaps.is_file() {
        Some(create(root, actionmaps, "before restore", game_version, actions)?)
    } else {
        None
    };
    if let Some(parent) = actionmaps.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    gamefile::write_atomic(actionmaps, &bytes).map_err(|e| format!("restore: {e}"))?;
    Ok(safety)
}

// ---------------------------------------------------------------------------
// Tauri commands
//
// Crate-visible: `generate_handler!` in lib.rs is the only caller.
// ---------------------------------------------------------------------------

pub(crate) fn backups_root(app: &AppHandle) -> Result<PathBuf, String> {
    app.path().app_data_dir().map(|d| d.join("backups")).map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn list_backups(app: AppHandle, data: State<Mutex<AppData>>) -> Vec<BackupSummary> {
    match backups_root(&app) {
        Ok(root) => list(&root, &data.lock().unwrap().sc.data.actions),
        Err(e) => {
            error!("no app data dir: {e}");
            Vec::new()
        }
    }
}

#[tauri::command]
pub(crate) fn create_backup(
    reason: String,
    app: AppHandle,
    data: State<Mutex<AppData>>,
) -> Result<BackupSummary, String> {
    let root = backups_root(&app)?;
    let data = data.lock().unwrap();
    let path = config::actionmaps_path(data.config.base_path());
    let version = data.sc.version.as_ref().map(|v| v.label.as_str());
    create(&root, &path, &reason, version, &data.sc.data.actions)
}

#[tauri::command]
pub(crate) fn delete_backup(id: String, app: AppHandle) -> Result<(), String> {
    delete(&backups_root(&app)?, &id)
}

#[tauri::command]
pub(crate) fn restore_backup(id: String, app: AppHandle, data: State<Mutex<AppData>>) -> Result<LoadStatus, String> {
    let root = backups_root(&app)?;
    let mut data = data.lock().unwrap();
    let path = config::actionmaps_path(data.config.base_path());
    let version = data.sc.version.as_ref().map(|v| v.label.as_str());
    restore(&root, &id, &path, version, &data.sc.data.actions)?;
    info!("backup {id} restored to {}", path.display());
    Ok(crate::reload_bindings(&mut data))
}

/// Open backup `id`'s folder in the system file manager.
#[tauri::command]
pub(crate) fn open_backup_dir(id: String, app: AppHandle) -> Result<(), String> {
    let file = path_of(&backups_root(&app)?, &id)?;
    let dir = file.parent().ok_or_else(|| format!("{}: no parent", file.display()))?;
    tauri_plugin_opener::open_path(dir, None::<&str>).map_err(|e| format!("open {}: {e}", dir.display()))
}

/// Open the backups folder in the system file manager (Settings), creating it
/// when no backup exists yet.
#[tauri::command]
pub(crate) fn open_backups_dir(app: AppHandle) -> Result<(), String> {
    let root = backups_root(&app)?;
    fs::create_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
    tauri_plugin_opener::open_path(&root, None::<&str>).map_err(|e| format!("open {}: {e}", root.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A second, different, valid bindings file for the restore tests.
    const OLD_XML: &str = "<ActionMaps>\n <ActionProfiles version=\"1\" profileName=\"default\">\n  <actionmap name=\"m\">\n   <action name=\"a\">\n    <rebind input=\"js1_button9\"/>\n   </action>\n  </actionmap>\n </ActionProfiles>\n</ActionMaps>\n";

    /// Fresh temp dir per test, removed on drop.
    struct Tmp(PathBuf);

    impl Tmp {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("bindsight-backups-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&dir).unwrap();
            Tmp(dir)
        }
        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A minimal actionmaps-shaped XML: one joystick + two joystick rebinds +
    /// one keyboard rebind (all three count — the summary covers every
    /// device).
    fn actionmaps_xml() -> String {
        r#"<ActionMaps version="1" optionsVersion="2" rebindVersion="2">
 <options type="joystick" instance="1" Product="Stick {0200231D-0000-0000-0000-504944564944}"/>
 <modifiers />
 <actionmap name="seat_general">
  <action name="v_eject">
   <rebind input="js1_button1"/>
  </action>
  <action name="v_toggle_flight_mode">
   <rebind input="js1_button2"/>
  </action>
  <action name="v_open_menu">
   <rebind input="kb1_space"/>
  </action>
 </actionmap>
</ActionMaps>"#
            .to_string()
    }

    fn sample_actions() -> Vec<scdata::ActionMap> {
        vec![scdata::ActionMap {
            name: "seat_general".into(),
            label: Some("Seat General".into()),
            actions: vec![
                scdata::Action {
                    name: "v_eject".into(),
                    label: Some("Eject".into()),
                    description: None,
                    joystick_default: None,
                    keyboard_default: None,
                    gamepad_default: None,
                    mouse_default: None,
                },
                scdata::Action {
                    name: "v_toggle_flight_mode".into(),
                    label: Some("Toggle Flight Mode".into()),
                    description: None,
                    joystick_default: None,
                    keyboard_default: None,
                    gamepad_default: None,
                    mouse_default: None,
                },
                scdata::Action {
                    name: "v_open_menu".into(),
                    label: Some("Open Menu".into()),
                    description: None,
                    joystick_default: None,
                    keyboard_default: None,
                    gamepad_default: None,
                    mouse_default: None,
                },
            ],
        }]
    }

    #[test]
    fn format_timestamp_matches_date_u() {
        // `date -u -d @<secs> "+%Y%m%d-%H%M%S"`.
        assert_eq!(format_timestamp(0), "19700101-000000");
        assert_eq!(format_timestamp(1), "19700101-000001");
        assert_eq!(format_timestamp(1_700_000_000), "20231114-221320");
        assert_eq!(format_timestamp(1_757_419_200), "20250909-120000");
    }

    #[test]
    fn create_unique_dir_counts_up() {
        let t = Tmp::new();
        assert_eq!(create_unique_dir(&t.0, "20250909-120000").unwrap(), "20250909-120000");
        assert_eq!(create_unique_dir(&t.0, "20250909-120000").unwrap(), "20250909-120000-2");
        assert_eq!(create_unique_dir(&t.0, "20250909-120000").unwrap(), "20250909-120000-3");
        assert!(t.0.join("20250909-120000-3").is_dir());
    }

    #[test]
    fn create_writes_files_and_summary() {
        let t = Tmp::new();
        let root = t.path("backups");
        let am = t.path("actionmaps.xml");
        fs::write(&am, actionmaps_xml()).unwrap();

        let s = create(&root, &am, "  before order fix  ", Some("4.10.0-hotfix.12572603"), &sample_actions()).unwrap();
        assert_eq!(s.reason, "before order fix");
        assert_eq!(s.game_version.as_deref(), Some("4.10.0-hotfix.12572603"));
        assert_eq!(s.bindings, 3); // js1_button1 + js1_button2 + kb1_space
        assert!(root.join(&s.id).join(XML_FILE).is_file());
        assert_eq!(fs::read_to_string(root.join(&s.id).join(XML_FILE)).unwrap(), actionmaps_xml());

        let meta: BackupMeta = serde_json::from_str(&fs::read_to_string(root.join(&s.id).join(META_FILE)).unwrap()).unwrap();
        assert_eq!(meta.created, s.created);
        assert_eq!(meta.reason, "before order fix");
        assert_eq!(meta.game_version.as_deref(), Some("4.10.0-hotfix.12572603"));

        // Empty reason becomes "manual"; no loaded game version stays unknown.
        let s2 = create(&root, &am, "   ", None, &sample_actions()).unwrap();
        assert_eq!(s2.reason, "manual");
        assert_eq!(s2.game_version, None);

        // Missing source file is refused.
        assert!(create(&root, &t.path("missing.xml"), "x", None, &sample_actions()).unwrap_err().contains("not found"));
    }

    #[test]
    fn create_deduplicates_same_second() {
        let t = Tmp::new();
        let root = t.path("backups");
        let am = t.path("actionmaps.xml");
        fs::write(&am, actionmaps_xml()).unwrap();

        // Force two backups to land on the same id by pre-creating it.
        let id = format_timestamp(now_secs());
        fs::create_dir_all(root.join(&id)).unwrap();
        let s = create(&root, &am, "manual", None, &sample_actions()).unwrap();
        assert_ne!(s.id, id);
        assert!(s.id.starts_with(&format!("{id}-")));
    }

    #[test]
    fn list_sorts_desc_and_skips_broken_folders() {
        let t = Tmp::new();
        let root = t.path("backups");
        fs::create_dir_all(&root).unwrap();

        // The older one is a meta.json from before game versions were recorded.
        let metas = [
            ("20230101-000000", r#"{"created":1672531200,"reason":"manual"}"#.to_string()),
            (
                "20240101-000000",
                serde_json::to_string_pretty(&BackupMeta {
                    created: 1_704_067_200,
                    reason: "manual".into(),
                    game_version: Some("4.10.0-hotfix.12572603".into()),
                })
                .unwrap(),
            ),
        ];
        for (id, meta) in metas {
            let dir = root.join(id);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join(XML_FILE), actionmaps_xml()).unwrap();
            fs::write(dir.join(META_FILE), meta).unwrap();
        }
        // No meta.json: skipped.
        fs::create_dir_all(root.join("broken")).unwrap();
        fs::write(root.join("broken").join(XML_FILE), actionmaps_xml()).unwrap();

        let found = list(&root, &sample_actions());
        assert_eq!(found.len(), 2, "the broken folder is skipped");
        assert_eq!(found[0].id, "20240101-000000");
        assert_eq!(found[0].game_version.as_deref(), Some("4.10.0-hotfix.12572603"));
        assert_eq!(found[1].id, "20230101-000000");
        assert_eq!(found[1].game_version, None);
        for s in &found {
            assert_eq!(s.bindings, 3);
        }

        assert!(list(&t.path("missing"), &sample_actions()).is_empty());
    }

    #[test]
    fn restore_backs_up_first_then_overwrites() {
        let t = Tmp::new();
        let root = t.path("backups");
        let am = t.path("live/actionmaps.xml");
        fs::create_dir_all(am.parent().unwrap()).unwrap();
        fs::write(&am, OLD_XML).unwrap();

        let backup = create(&root, &am, "manual", None, &sample_actions()).unwrap();
        fs::write(&am, actionmaps_xml()).unwrap(); // live file changes after the backup

        let safety = restore(&root, &backup.id, &am, Some("4.10.0-hotfix.12572603"), &sample_actions()).unwrap();
        let safety = safety.expect("a safety backup is made when the live file exists");
        assert_eq!(safety.reason, "before restore");
        assert_eq!(safety.game_version.as_deref(), Some("4.10.0-hotfix.12572603"));
        // Live file now holds the restored (old) content.
        assert_eq!(fs::read_to_string(&am).unwrap(), OLD_XML);
        // The restored backup itself is untouched.
        assert_eq!(fs::read_to_string(root.join(&backup.id).join(XML_FILE)).unwrap(), OLD_XML);
        // The safety backup holds what was live just before the restore.
        assert_eq!(fs::read_to_string(root.join(&safety.id).join(XML_FILE)).unwrap(), actionmaps_xml());
    }

    #[test]
    fn restore_without_a_live_file_makes_no_safety_backup() {
        let t = Tmp::new();
        let root = t.path("backups");
        let am = t.path("live/actionmaps.xml");

        let backup = create(&root, &t.path("source.xml"), "manual", None, &sample_actions()).unwrap_err();
        assert!(backup.contains("not found")); // sanity: no source yet either

        fs::write(t.path("source.xml"), actionmaps_xml()).unwrap();
        let backup = create(&root, &t.path("source.xml"), "manual", None, &sample_actions()).unwrap();

        assert!(!am.is_file());
        let safety = restore(&root, &backup.id, &am, None, &sample_actions()).unwrap();
        assert!(safety.is_none());
        assert_eq!(fs::read_to_string(&am).unwrap(), actionmaps_xml());
    }

    #[test]
    fn restore_refuses_a_backup_that_does_not_parse() {
        let t = Tmp::new();
        let root = t.path("backups");
        let am = t.path("live/actionmaps.xml");
        fs::create_dir_all(am.parent().unwrap()).unwrap();
        fs::write(&am, actionmaps_xml()).unwrap();
        let backup = create(&root, &am, "manual", None, &sample_actions()).unwrap();
        // The backup gets damaged on disk after it was made.
        fs::write(root.join(&backup.id).join(XML_FILE), "<ActionMaps><broken").unwrap();

        let err = restore(&root, &backup.id, &am, None, &sample_actions()).unwrap_err();
        assert!(err.contains("not restored"), "{err}");
        assert_eq!(fs::read_to_string(&am).unwrap(), actionmaps_xml(), "live file untouched");
        assert_eq!(list(&root, &sample_actions()).len(), 0, "no safety backup, and the damaged one no longer lists");
    }

    #[test]
    fn create_verifies_the_copy_byte_for_byte() {
        let t = Tmp::new();
        let root = t.path("backups");
        let am = t.path("actionmaps.xml");
        let content = format!("\u{feff}{}", actionmaps_xml().replace('\n', "\r\n"));
        fs::write(&am, &content).unwrap();
        let s = create(&root, &am, "manual", None, &sample_actions()).unwrap();
        assert_eq!(fs::read(root.join(&s.id).join(XML_FILE)).unwrap(), content.as_bytes());
        // A source that cannot be read leaves no folder behind.
        let before = fs::read_dir(&root).unwrap().count();
        assert!(create(&root, &t.path("missing.xml"), "manual", None, &sample_actions()).is_err());
        assert_eq!(fs::read_dir(&root).unwrap().count(), before);
    }

    #[test]
    fn delete_removes_the_folder() {
        let t = Tmp::new();
        let root = t.path("backups");
        let am = t.path("actionmaps.xml");
        fs::write(&am, actionmaps_xml()).unwrap();
        let s = create(&root, &am, "manual", None, &sample_actions()).unwrap();
        assert!(root.join(&s.id).is_dir());

        delete(&root, &s.id).unwrap();
        assert!(!root.join(&s.id).exists());

        assert!(delete(&root, "missing-id").is_err());
    }

    #[test]
    fn bad_ids_are_refused() {
        let t = Tmp::new();
        let root = t.path("backups");
        for bad in ["..", "../x", "a/b", "a\\b", "", "C:x", "C:"] {
            assert!(path_of(&root, bad).is_err(), "{bad:?} should be rejected");
            assert!(delete(&root, bad).is_err(), "{bad:?} should be rejected");
        }
    }
}
