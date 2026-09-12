//! Guarded writes to the game's files — the one road every change to the
//! live `actionmaps.xml` takes (see CLAUDE.md, "Game-file safety").
//!
//! [`write_atomic`] never leaves a half-written file behind: the bytes go to
//! a temporary file next to the target, are flushed to disk, and only then
//! replace the target by rename; the result is read back and compared.
//! [`replace_live_file`] adds the rest of the contract: the new text must
//! parse as an `actionmaps.xml`, a backup of the current file is taken and
//! verified byte for byte, and only then the atomic write runs. Nothing here
//! is gated by a setting — a live-file write without a backup does not exist.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use log::{info, warn};

use crate::{backups, scdata};

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
/// [`backups::create`]), then the atomic write runs. Returns the backup's id.
/// A file that does not exist yet cannot be replaced (there is nothing to
/// back up) — the game writes it first.
pub fn replace_live_file(
    backups_root: &Path,
    path: &Path,
    new_xml: &str,
    reason: &str,
    game_version: Option<&str>,
    actions: &[scdata::ActionMap],
) -> Result<String, String> {
    if !path.is_file() {
        return Err(format!("{}: not found", path.display()));
    }
    scdata::parse_actionmaps(new_xml).map_err(|e| format!("refusing to write: new content is not readable: {e}"))?;
    let backup = backups::create(backups_root, path, reason, game_version, actions)?;
    if let Err(e) = write_atomic(path, new_xml.as_bytes()) {
        warn!("write of {} failed after backup {}: {e}", path.display(), backup.id);
        return Err(e);
    }
    info!("{} replaced (backup {})", path.display(), backup.id);
    Ok(backup.id)
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
        let id = replace_live_file(&root, &live, &new_xml, "test", Some("4.10"), &[]).unwrap();
        assert_eq!(fs::read_to_string(&live).unwrap(), new_xml);
        // The backup holds the old bytes, exactly.
        assert_eq!(fs::read_to_string(root.join(&id).join("actionmaps.xml")).unwrap(), XML);
        assert!(leftovers(&t.0).is_empty());
    }

    #[test]
    fn replace_live_file_refuses_unreadable_content_and_touches_nothing() {
        let t = Tmp::new();
        let live = t.path("actionmaps.xml");
        let root = t.path("backups");
        fs::write(&live, XML).unwrap();
        let err = replace_live_file(&root, &live, "<ActionMaps><oops", "test", None, &[]).unwrap_err();
        assert!(err.contains("refusing to write"), "{err}");
        assert_eq!(fs::read_to_string(&live).unwrap(), XML);
        assert!(!root.exists(), "no backup for a refused write");
    }

    #[test]
    fn replace_live_file_needs_an_existing_file() {
        let t = Tmp::new();
        let live = t.path("actionmaps.xml");
        let err = replace_live_file(&t.path("backups"), &live, XML, "test", None, &[]).unwrap_err();
        assert!(err.contains("not found"));
        assert!(!live.exists());
    }
}
