//! Guarded writes to the game's files — the one road every change to the
//! live `actionmaps.xml` and `attributes.xml` takes (see CLAUDE.md,
//! "Game-file safety").
//!
//! [`write_atomic`] never leaves a half-written file behind: the bytes go to
//! a temporary file next to the target, are flushed to disk, and only then
//! replace the target by rename; the result is read back and compared.
//! [`replace_live_file`] adds the rest of the contract: the new text must
//! parse as an `actionmaps.xml`, a backup of the current file is taken and
//! verified byte for byte, and only then the atomic write runs. The backup
//! is skipped only when the user switched auto-backups off in Settings —
//! their explicit choice, made against a warning (the one exception to the
//! game-file safety rule); the parse check and the atomic write still run.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use log::{error, info, warn};

use crate::{backups, devconfig, scdata};

/// Write `bytes` to `path` atomically (temp file in the same directory,
/// `sync_all`, rename over the target), then read the file back and compare.
/// On any error the target is untouched (or, after a rename that succeeded
/// but a read-back that did not, the error says so) and the temp file is
/// removed.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).ok_or_else(|| format!("{}: no parent directory", path.display()))?;
    if !dir.is_dir() {
        return Err(format!("{}: directory does not exist", dir.display()));
    }
    if path.is_dir() {
        return Err(format!("{}: is a directory", path.display()));
    }
    let tmp = temp_path(path);
    let result = (|| {
        let mut file = fs::File::create(&tmp).map_err(|e| format!("create {}: {e}", tmp.display()))?;
        file.write_all(bytes).map_err(|e| format!("write {}: {e}", tmp.display()))?;
        file.sync_all().map_err(|e| format!("sync {}: {e}", tmp.display()))?;
        drop(file);
        fs::rename(&tmp, path).map_err(|e| format!("replace {}: {e}", path.display()))
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    let written = fs::read(path).map_err(|e| format!("read back {}: {e}", path.display()))?;
    if written != bytes {
        return Err(format!("{}: read back differs from what was written", path.display()));
    }
    Ok(())
}

/// `<dir>/.<file>.bindsight-<uuid>.tmp` — unique, in the target's directory
/// so the final rename stays on one file system.
fn temp_path(path: &Path) -> PathBuf {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    path.with_file_name(format!(".{name}.bindsight-{}.tmp", uuid::Uuid::new_v4()))
}

/// Replace the live `actionmaps.xml` at `path` with `new_xml`, the guarded
/// way: `new_xml` must parse, the current file is backed up under
/// `backups_root` (reason `reason`, verified byte for byte by
/// [`backups::create`]) unless `auto_backup` is off, then the atomic write
/// runs. Returns the backup's id, `None` when none was made. A file that
/// does not exist yet cannot be replaced (there is nothing to back up) — the
/// game writes it first.
pub fn replace_live_file(
    backups_root: &Path,
    path: &Path,
    new_xml: &str,
    reason: &str,
    auto_backup: bool,
    game_version: Option<&str>,
    actions: &[scdata::ActionMap],
) -> Result<Option<String>, String> {
    if !path.is_file() {
        return Err(format!("{}: not found", path.display()));
    }
    scdata::parse_actionmaps(new_xml).map_err(|e| {
        error!("refusing to write {}: new content is not readable: {e}", path.display());
        format!("refusing to write: new content is not readable: {e}")
    })?;
    let backup = if auto_backup {
        match backups::create(backups_root, path, reason, game_version, actions) {
            Ok(summary) => Some(summary.id),
            Err(e) => {
                error!("{}: no backup made, not writing: {e}", path.display());
                return Err(e);
            }
        }
    } else {
        warn!("{}: writing without a backup (auto-backups off)", path.display());
        None
    };
    if let Err(e) = write_atomic(path, new_xml.as_bytes()) {
        error!("write of {} failed ({}): {e}", path.display(), backup_label(&backup));
        return Err(e);
    }
    info!("{} replaced ({})", path.display(), backup_label(&backup));
    Ok(backup)
}

/// Replace the live `actionmaps.xml` at `actionmaps` and / or the game's
/// `attributes.xml` next to it (`backups::attributes_beside`) the guarded
/// way, for the device settings: each new text must parse as its kind of
/// file, then ONE backup of both files is taken (verified, unless
/// `auto_backup` is off), then the atomic writes run — `actionmaps.xml`
/// first. A file passed as `None` stays untouched. Should the second write
/// fail after the first succeeded, the error says so; the backup holds both
/// originals. Returns the backup's id, `None` when none was made.
#[allow(clippy::too_many_arguments)]
pub fn replace_live_config(
    backups_root: &Path,
    actionmaps: &Path,
    new_actionmaps: Option<&str>,
    new_attributes: Option<&str>,
    reason: &str,
    auto_backup: bool,
    game_version: Option<&str>,
    actions: &[scdata::ActionMap],
) -> Result<Option<String>, String> {
    if new_actionmaps.is_none() && new_attributes.is_none() {
        return Err("nothing to write".into());
    }
    // The backup needs the bindings file even when only the settings change.
    if !actionmaps.is_file() {
        return Err(format!("{}: not found", actionmaps.display()));
    }
    let attributes = backups::attributes_beside(actionmaps);
    if let Some(xml) = new_actionmaps {
        scdata::parse_actionmaps(xml).map_err(|e| {
            error!("refusing to write {}: new content is not readable: {e}", actionmaps.display());
            format!("refusing to write: new content is not readable: {e}")
        })?;
    }
    if let Some(xml) = new_attributes {
        if !attributes.is_file() {
            return Err(format!("{}: not found", attributes.display()));
        }
        devconfig::parse_attributes(xml).map_err(|e| {
            error!("refusing to write {}: new content is not readable: {e}", attributes.display());
            format!("refusing to write: new content is not readable: {e}")
        })?;
    }
    let backup = if auto_backup {
        match backups::create(backups_root, actionmaps, reason, game_version, actions) {
            Ok(summary) => Some(summary.id),
            Err(e) => {
                error!("{}: no backup made, not writing: {e}", actionmaps.display());
                return Err(e);
            }
        }
    } else {
        warn!("{}: writing without a backup (auto-backups off)", actionmaps.display());
        None
    };
    let mut written: Vec<String> = Vec::new();
    for (path, text) in [(actionmaps, new_actionmaps), (attributes.as_path(), new_attributes)] {
        let Some(text) = text else { continue };
        if let Err(e) = write_atomic(path, text.as_bytes()) {
            let done = if written.is_empty() { String::new() } else { format!(", already written: {}", written.join(", ")) };
            error!("write of {} failed ({}{done}): {e}", path.display(), backup_label(&backup));
            return Err(if written.is_empty() { e } else { format!("{e} (already written: {})", written.join(", ")) });
        }
        info!("{} replaced ({})", path.display(), backup_label(&backup));
        written.push(path.display().to_string());
    }
    Ok(backup)
}

/// `backup <id>` or `no backup`, for log lines.
pub fn backup_label(backup: &Option<String>) -> String {
    backup.as_ref().map_or_else(|| "no backup".to_string(), |id| format!("backup {id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);

    impl Tmp {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("bindsight-gamefile-{}", uuid::Uuid::new_v4()));
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

    const XML: &str = "<ActionMaps>\n <ActionProfiles version=\"1\" profileName=\"default\">\n  <actionmap name=\"m\">\n   <action name=\"a\">\n    <rebind input=\"js1_button1\"/>\n   </action>\n  </actionmap>\n </ActionProfiles>\n</ActionMaps>\n";

    fn leftovers(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".bindsight-"))
            .collect()
    }

    #[test]
    fn write_atomic_creates_replaces_and_leaves_no_temp_file() {
        let t = Tmp::new();
        let p = t.path("actionmaps.xml");
        write_atomic(&p, b"one").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"one");
        write_atomic(&p, "zw\u{f6}lf \r\n".as_bytes()).unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "zw\u{f6}lf \r\n");
        assert!(leftovers(&t.0).is_empty());
    }

    #[test]
    fn write_atomic_refuses_a_missing_directory_and_a_directory_target() {
        let t = Tmp::new();
        let missing = t.path("nope").join("actionmaps.xml");
        assert!(write_atomic(&missing, b"x").unwrap_err().contains("does not exist"));
        assert!(!missing.exists());
        assert!(write_atomic(&t.0, b"x").unwrap_err().contains("is a directory"));
        assert!(leftovers(&t.0).is_empty());
    }

    #[test]
    fn replace_live_file_backs_up_first_and_verifies() {
        let t = Tmp::new();
        let live = t.path("actionmaps.xml");
        let root = t.path("backups");
        fs::write(&live, XML).unwrap();
        let new_xml = XML.replace("js1_button1", "js1_button2");
        let id = replace_live_file(&root, &live, &new_xml, "test", true, Some("4.10"), &[]).unwrap().unwrap();
        assert_eq!(fs::read_to_string(&live).unwrap(), new_xml);
        // The backup holds the old bytes, exactly.
        assert_eq!(fs::read_to_string(root.join(&id).join("actionmaps.xml")).unwrap(), XML);
        assert!(leftovers(&t.0).is_empty());
    }

    #[test]
    fn replace_live_file_without_auto_backup_still_checks_and_writes_atomically() {
        let t = Tmp::new();
        let live = t.path("actionmaps.xml");
        let root = t.path("backups");
        fs::write(&live, XML).unwrap();
        let new_xml = XML.replace("js1_button1", "js1_button2");
        assert_eq!(replace_live_file(&root, &live, &new_xml, "test", false, None, &[]).unwrap(), None);
        assert_eq!(fs::read_to_string(&live).unwrap(), new_xml);
        assert!(!root.exists(), "no backup folder was made");
        assert!(replace_live_file(&root, &live, "<ActionMaps><oops", "test", false, None, &[]).is_err());
        assert_eq!(fs::read_to_string(&live).unwrap(), new_xml);
        assert!(leftovers(&t.0).is_empty());
    }

    #[test]
    fn replace_live_file_refuses_unreadable_content_and_touches_nothing() {
        let t = Tmp::new();
        let live = t.path("actionmaps.xml");
        let root = t.path("backups");
        fs::write(&live, XML).unwrap();
        let err = replace_live_file(&root, &live, "<ActionMaps><oops", "test", true, None, &[]).unwrap_err();
        assert!(err.contains("refusing to write"), "{err}");
        assert_eq!(fs::read_to_string(&live).unwrap(), XML);
        assert!(!root.exists(), "no backup for a refused write");
    }

    const ATTRS: &str = "<Attributes Version=\"35\">\n <Attr name=\"Sensitivity\" value=\"1\"/>\n</Attributes>\n";

    #[test]
    fn replace_live_config_backs_up_both_files_once_then_writes_what_changed() {
        let t = Tmp::new();
        let live = t.path("actionmaps.xml");
        let attributes = t.path("attributes.xml");
        let root = t.path("backups");
        fs::write(&live, XML).unwrap();
        fs::write(&attributes, ATTRS).unwrap();

        let new_xml = XML.replace("js1_button1", "js1_button2");
        let new_attrs = ATTRS.replace("value=\"1\"", "value=\"1.2\"");
        let id = replace_live_config(&root, &live, Some(&new_xml), Some(&new_attrs), "before config", true, None, &[]).unwrap().unwrap();
        assert_eq!(fs::read_to_string(&live).unwrap(), new_xml);
        assert_eq!(fs::read_to_string(&attributes).unwrap(), new_attrs);
        assert_eq!(fs::read_to_string(root.join(&id).join("actionmaps.xml")).unwrap(), XML);
        assert_eq!(fs::read_to_string(root.join(&id).join("attributes.xml")).unwrap(), ATTRS);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1, "one backup for both files");

        // Only the settings: the bindings file is not rewritten.
        let id = replace_live_config(&root, &live, None, Some(ATTRS), "before config", true, None, &[]).unwrap().unwrap();
        assert_eq!(fs::read_to_string(&live).unwrap(), new_xml);
        assert_eq!(fs::read_to_string(&attributes).unwrap(), ATTRS);
        assert_eq!(fs::read_to_string(root.join(&id).join("attributes.xml")).unwrap(), new_attrs);
        // Auto-backups off: written, no backup.
        assert_eq!(replace_live_config(&root, &live, Some(XML), None, "before config", false, None, &[]).unwrap(), None);
        assert_eq!(fs::read_to_string(&live).unwrap(), XML);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
        assert!(leftovers(&t.0).is_empty());
    }

    #[test]
    fn replace_live_config_refuses_and_touches_nothing() {
        let t = Tmp::new();
        let live = t.path("actionmaps.xml");
        let attributes = t.path("attributes.xml");
        let root = t.path("backups");
        fs::write(&live, XML).unwrap();
        // Unreadable new content, either file.
        assert!(replace_live_config(&root, &live, Some("<ActionMaps><oops"), None, "x", true, None, &[]).unwrap_err().contains("refusing"));
        fs::write(&attributes, ATTRS).unwrap();
        assert!(replace_live_config(&root, &live, Some(XML), Some("<ActionMaps/>"), "x", true, None, &[]).unwrap_err().contains("refusing"));
        assert_eq!(fs::read_to_string(&attributes).unwrap(), ATTRS);
        // No settings file to replace.
        fs::remove_file(&attributes).unwrap();
        assert!(replace_live_config(&root, &live, None, Some(ATTRS), "x", true, None, &[]).unwrap_err().contains("not found"));
        assert!(!attributes.exists());
        assert!(replace_live_config(&root, &live, None, None, "x", true, None, &[]).is_err());
        assert_eq!(fs::read_to_string(&live).unwrap(), XML);
        assert!(!root.exists(), "no backup for a refused write");
    }

    #[test]
    fn replace_live_file_needs_an_existing_file() {
        let t = Tmp::new();
        let live = t.path("actionmaps.xml");
        let err = replace_live_file(&t.path("backups"), &live, XML, "test", true, None, &[]).unwrap_err();
        assert!(err.contains("not found"));
        assert!(!live.exists());
    }
}
