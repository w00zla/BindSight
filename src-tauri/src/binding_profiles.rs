//! SC's exported keybinding layouts ("binding profiles"): the XML files SC
//! writes to `controls/mappings/` when the user picks "Export" in the
//! keybindings options menu. Structurally these are the same
//! `<options>`/`<actionmap>`/`<action>`/`<rebind>` content as the live
//! `actionmaps.xml`, just wrapped in an `<ActionMaps profileName="...">` root
//! with a `<CustomisationUIHeader>` instead of `<ActionMaps><ActionProfiles>`
//! — [`scdata::parse_actionmaps`] reads both, since it scans by element
//! name rather than depth.
//!
//! This module lists what is on disk, copies files in/out of that folder
//! (import/export), saves the live file as a new profile and deletes one.
//! Applying a binding profile — writing it into the live `actionmaps.xml`
//! — is `apply.rs`'s job.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use log::warn;
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use serde::Serialize;
use tauri::State;

use crate::names::{is_safe_name, sanitize_name};
use crate::{bindings, config, scdata, AppData};

/// Listing entry for one exported binding profile file.
#[derive(Debug, Clone, Serialize)]
pub struct BindingProfileSummary {
    /// Bare file name inside the binding profiles folder.
    pub file: String,
    /// The `profileName` the export was saved under, or the file stem if
    /// that attribute is missing/blank.
    pub name: String,
    /// Bindings in the binding profile across all devices, per
    /// [`bindings::resolve_bindings`].
    pub bindings: usize,
    /// File mtime, unix seconds; 0 if unknown.
    pub modified: u64,
}

/// A plain `.xml` file name that cannot escape its folder: non-empty, no path
/// separators, not `.`/`..`, and a case-insensitive `.xml` extension.
pub(crate) fn is_bare_xml_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
        && Path::new(name)
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("xml"))
}

/// The `profileName` attribute of the root `ActionMaps` element, trimmed;
/// `None` if absent, empty, or the XML doesn't parse.
pub fn profile_name(xml: &str) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if e.name().as_ref() == b"ActionMaps" => {
                let value = e
                    .attributes()
                    .flatten()
                    .find(|a| a.key.as_ref() == b"profileName")
                    .and_then(|a| a.unescape_value().ok())?;
                let value = value.trim();
                return if value.is_empty() { None } else { Some(value.to_string()) };
            }
            Ok(Event::Eof) | Err(_) => return None,
            _ => {}
        }
    }
}

/// File mtime as unix seconds, or 0 if unavailable.
fn modified_secs(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Read, parse and resolve one binding profile file into its summary.
pub fn summarize(path: &Path, actions: &[scdata::ActionMap]) -> Result<BindingProfileSummary, String> {
    let xml = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let profile = scdata::parse_actionmaps(&xml)?;
    let resolved = bindings::resolve_bindings(actions, &profile);

    let file = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("{}: not a file name", path.display()))?
        .to_string();
    let name = profile_name(&xml).unwrap_or_else(|| {
        path.file_stem().and_then(|s| s.to_str()).unwrap_or(&file).to_string()
    });

    Ok(BindingProfileSummary { file, name, bindings: resolved.len(), modified: modified_secs(path) })
}

/// Every `*.xml` file directly under `dir` that summarizes OK, newest first
/// (then by name). Unreadable/unparsable files are logged and skipped; a
/// missing `dir` yields an empty list.
pub fn list(dir: &Path, actions: &[scdata::ActionMap]) -> Vec<BindingProfileSummary> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let is_xml = path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("xml"));
        if !path.is_file() || !is_xml {
            continue;
        }
        match summarize(&path, actions) {
            Ok(s) => out.push(s),
            Err(e) => warn!("skipping binding profile {}: {e}", path.display()),
        }
    }
    out.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out
}

/// Copy `source` (an exported binding profile, anywhere on disk) into `dir`
/// under its own file name. Refuses a non-`.xml`/non-bare name, an
/// unparsable source, and a name already present in `dir`.
pub fn import(dir: &Path, source: &Path, actions: &[scdata::ActionMap]) -> Result<BindingProfileSummary, String> {
    let file = source
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("{}: not a file name", source.display()))?;
    if !is_bare_xml_name(file) {
        return Err(format!("{file:?} is not a valid binding profile file name"));
    }
    // Must parse before it is accepted as a binding profile.
    summarize(source, actions)?;

    let dest = dir.join(file);
    if dest.exists() {
        return Err(format!("{file} already exists"));
    }
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    fs::copy(source, &dest).map_err(|e| e.to_string())?;
    summarize(&dest, actions)
}

/// Copy `file` (a bare name that must exist in `dir`) to `dest`. Overwrites
/// `dest` if present — the save dialog already asked.
pub fn export(dir: &Path, file: &str, dest: &Path) -> Result<(), String> {
    if !is_bare_xml_name(file) {
        return Err(format!("{file:?} is not a valid binding profile file name"));
    }
    let src = dir.join(file);
    if !src.is_file() {
        return Err(format!("{file}: not found"));
    }
    fs::copy(&src, dest).map_err(|e| e.to_string())?;
    Ok(())
}

/// The live `actionmaps.xml` rewritten in SC's export layout: the content of
/// `<ActionProfiles …>` under an `<ActionMaps profileName="…">` root with a
/// `<CustomisationUIHeader>` naming the devices the file's `<options>` name
/// (keyboard, mouse and gamepad always). Textual, so the bindings stay byte
/// for byte.
pub fn to_profile_xml(live_xml: &str, name: &str) -> Result<String, String> {
    let open = live_xml.find("<ActionProfiles").ok_or("no <ActionProfiles> in actionmaps.xml")?;
    let open_end = live_xml[open..].find('>').ok_or("unterminated <ActionProfiles> tag")? + open + 1;
    let close = live_xml.find("</ActionProfiles>").ok_or("<ActionProfiles> without </ActionProfiles>")?;
    let head = &live_xml[open..open_end];
    let attr = |key: &str| -> String {
        let k = format!(" {key}=\"");
        head.find(&k)
            .and_then(|i| {
                let v = &head[i + k.len()..];
                v.find('"').map(|j| v[..j].to_string())
            })
            .unwrap_or_else(|| "1".to_string())
    };
    let eol = if live_xml.contains("\r\n") { "\r\n" } else { "\n" };
    let file = scdata::parse_actionmaps(live_xml)?;
    let mut instances: Vec<u32> = file.joysticks.iter().map(|j| j.instance).collect();
    instances.sort_unstable();
    instances.dedup();
    let label = name.replace('&', "&amp;").replace('"', "&quot;");

    let mut out = String::new();
    out.push_str(&format!(
        "<ActionMaps version=\"{}\" optionsVersion=\"{}\" rebindVersion=\"{}\" profileName=\"{label}\">{eol}",
        attr("version"),
        attr("optionsVersion"),
        attr("rebindVersion")
    ));
    out.push_str(&format!(" <CustomisationUIHeader label=\"{label}\" description=\"\" image=\"\">{eol}"));
    out.push_str(&format!("  <devices>{eol}"));
    out.push_str(&format!("   <keyboard instance=\"1\"/>{eol}"));
    out.push_str(&format!("   <mouse instance=\"1\"/>{eol}"));
    out.push_str(&format!("   <gamepad instance=\"1\"/>{eol}"));
    for i in instances {
        out.push_str(&format!("   <joystick instance=\"{i}\"/>{eol}"));
    }
    out.push_str(&format!("  </devices>{eol}"));
    out.push_str(&format!(" </CustomisationUIHeader>{eol}"));
    out.push_str(live_xml[open_end..close].trim_matches(|c| c == '\r' || c == '\n'));
    out.push_str(&format!("{eol}</ActionMaps>{eol}"));
    scdata::parse_actionmaps(&out).map_err(|e| format!("rewrite produced unreadable XML: {e}"))?;
    Ok(out)
}

/// Save the live file under `dir/<name>.xml` as a binding profile. Refuses
/// an invalid name and an existing file.
pub fn save_profile(dir: &Path, actionmaps: &Path, name: &str, actions: &[scdata::ActionMap]) -> Result<BindingProfileSummary, String> {
    let name = sanitize_name(name, "");
    if !is_safe_name(&name) {
        return Err("invalid profile name (letters, digits, space, _ - and brackets only)".into());
    }
    let dest = dir.join(format!("{name}.xml"));
    if dest.exists() {
        return Err(format!("{name}.xml already exists"));
    }
    let xml = fs::read_to_string(actionmaps).map_err(|e| format!("{}: {e}", actionmaps.display()))?;
    let profile = to_profile_xml(&xml, &name)?;
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    fs::write(&dest, profile).map_err(|e| format!("{}: {e}", dest.display()))?;
    summarize(&dest, actions)
}

/// Delete `file` (a bare name) from `dir`.
pub fn delete(dir: &Path, file: &str) -> Result<(), String> {
    if !is_bare_xml_name(file) {
        return Err(format!("{file:?} is not a valid binding profile file name"));
    }
    let path = dir.join(file);
    if !path.is_file() {
        return Err(format!("{file}: not found"));
    }
    fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// Tauri commands
//
// Crate-visible: `generate_handler!` in lib.rs is the only caller.
// ---------------------------------------------------------------------------

#[tauri::command]
pub(crate) fn save_binding_profile(name: String, data: State<Mutex<AppData>>) -> Result<BindingProfileSummary, String> {
    let dir = binding_profiles_dir(&data);
    let data = data.lock().unwrap();
    let path = config::actionmaps_path(data.config.base_path());
    save_profile(&dir, &path, &name, &data.sc.data.actions)
}

#[tauri::command]
pub(crate) fn delete_binding_profile(file: String, data: State<Mutex<AppData>>) -> Result<(), String> {
    delete(&binding_profiles_dir(&data), &file)
}

/// Open the binding profiles folder in the system file manager (created
/// when missing).
#[tauri::command]
pub(crate) fn open_binding_profiles_dir(data: State<Mutex<AppData>>) -> Result<(), String> {
    let dir = binding_profiles_dir(&data);
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    tauri_plugin_opener::open_path(&dir, None::<&str>).map_err(|e| format!("open {}: {e}", dir.display()))
}

#[tauri::command]
pub(crate) fn list_binding_profiles(data: State<Mutex<AppData>>) -> Vec<BindingProfileSummary> {
    let dir = binding_profiles_dir(&data);
    list(&dir, &data.lock().unwrap().sc.data.actions)
}

#[tauri::command]
pub(crate) fn import_binding_profile(
    source_path: String,
    data: State<Mutex<AppData>>,
) -> Result<BindingProfileSummary, String> {
    let dir = binding_profiles_dir(&data);
    import(&dir, Path::new(&source_path), &data.lock().unwrap().sc.data.actions)
}

#[tauri::command]
pub(crate) fn export_binding_profile(file: String, dest_path: String, data: State<Mutex<AppData>>) -> Result<(), String> {
    let dir = binding_profiles_dir(&data);
    export(&dir, &file, Path::new(&dest_path))
}

fn binding_profiles_dir(data: &State<Mutex<AppData>>) -> PathBuf {
    config::binding_profiles_dir(data.lock().unwrap().config.base_path())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::time::{Duration, SystemTime};

    /// Fresh temp dir per test, removed on drop.
    struct Tmp(PathBuf);

    impl Tmp {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("bindsight-binding-profiles-{}", uuid::Uuid::new_v4()));
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

    /// A minimal export-shaped layout: header + one joystick + two joystick
    /// rebinds + one keyboard rebind (all three count — the summary covers
    /// every device).
    fn layout_xml(profile_name: &str) -> String {
        format!(
            r#"<ActionMaps version="1" optionsVersion="2" rebindVersion="2" profileName="{profile_name}">
 <CustomisationUIHeader label="{profile_name}" description="" image="">
  <devices>
   <keyboard instance="1"/>
   <joystick instance="1"/>
  </devices>
 </CustomisationUIHeader>
 <options type="joystick" instance="1" Product="Stick {{0200231D-0000-0000-0000-504944564944}}"/>
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
        )
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

    fn set_mtime(path: &Path, secs: u64) {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(secs);
        File::options().write(true).open(path).unwrap().set_modified(t).unwrap();
    }

    #[test]
    fn profile_name_present_or_missing() {
        assert_eq!(profile_name(&layout_xml("w00z14")).as_deref(), Some("w00z14"));
        assert_eq!(profile_name(r#"<ActionMaps version="1"></ActionMaps>"#), None);
        assert_eq!(profile_name(r#"<ActionMaps profileName="  "></ActionMaps>"#), None);
        assert_eq!(profile_name("not xml at all"), None);
    }

    #[test]
    fn summarize_counts_bindings_of_every_device() {
        let t = Tmp::new();
        let path = t.path("layout_test_exported.xml");
        fs::write(&path, layout_xml("Test")).unwrap();

        let s = summarize(&path, &sample_actions()).unwrap();
        assert_eq!(s.file, "layout_test_exported.xml");
        assert_eq!(s.name, "Test");
        assert_eq!(s.bindings, 3); // js1_button1 + js1_button2 + kb1_space
    }

    #[test]
    fn summarize_falls_back_to_file_stem_without_profile_name() {
        let t = Tmp::new();
        let path = t.path("no_name.xml");
        fs::write(&path, r#"<ActionMaps><actionmap name="m"></actionmap></ActionMaps>"#).unwrap();

        let s = summarize(&path, &[]).unwrap();
        assert_eq!(s.name, "no_name");
        assert_eq!(s.bindings, 0);
    }

    #[test]
    fn list_sorts_by_modified_desc_then_skips_bad_files() {
        let t = Tmp::new();
        let dir = t.path("profiles");
        fs::create_dir_all(&dir).unwrap();

        let older = dir.join("older.xml");
        fs::write(&older, layout_xml("Older")).unwrap();
        set_mtime(&older, 1_700_000_000);

        let newer = dir.join("newer.xml");
        fs::write(&newer, layout_xml("Newer")).unwrap();
        set_mtime(&newer, 1_700_000_060);

        fs::write(dir.join("notes.txt"), b"not a layout").unwrap();
        fs::write(dir.join("broken.xml"), b"<ActionMaps <<< not valid").unwrap();

        let found = list(&dir, &sample_actions());
        assert_eq!(found.len(), 2, "notes.txt and broken.xml are skipped");
        assert_eq!(found[0].name, "Newer");
        assert_eq!(found[1].name, "Older");

        assert!(list(&t.path("missing"), &sample_actions()).is_empty());
    }

    #[test]
    fn import_copies_and_refuses_duplicates_and_bad_sources() {
        let t = Tmp::new();
        let dir = t.path("profiles");
        let src = t.path("layout_a_exported.xml");
        fs::write(&src, layout_xml("A")).unwrap();

        let s = import(&dir, &src, &sample_actions()).unwrap();
        assert_eq!(s.file, "layout_a_exported.xml");
        assert_eq!(s.bindings, 3);
        assert!(dir.join("layout_a_exported.xml").is_file());

        // Importing the same file name again is refused.
        assert!(import(&dir, &src, &sample_actions()).unwrap_err().contains("already exists"));

        // An unparsable source is refused, and nothing is copied.
        let broken = t.path("layout_b_exported.xml");
        fs::write(&broken, b"<ActionMaps <<< not valid").unwrap();
        assert!(import(&dir, &broken, &sample_actions()).is_err());
        assert!(!dir.join("layout_b_exported.xml").exists());

        // A non-xml name is refused.
        let txt = t.path("layout_c.txt");
        fs::write(&txt, b"whatever").unwrap();
        assert!(import(&dir, &txt, &sample_actions()).unwrap_err().contains("not a valid"));
    }

    #[test]
    fn save_profile_wraps_the_live_file_in_the_export_layout() {
        let t = Tmp::new();
        let live = t.path("actionmaps.xml");
        fs::write(&live, concat!(
            "<ActionMaps>\n",
            " <ActionProfiles version=\"1\" optionsVersion=\"2\" rebindVersion=\"2\" profileName=\"default\">\n",
            "  <options type=\"joystick\" instance=\"2\" Product=\"Stick {0200231D-0000-0000-0000-504944564944}\"/>\n",
            "  <actionmap name=\"seat_general\">\n",
            "   <action name=\"v_eject\">\n",
            "    <rebind input=\"js2_button1\"/>\n",
            "   </action>\n",
            "  </actionmap>\n",
            " </ActionProfiles>\n",
            "</ActionMaps>\n",
        )).unwrap();
        let dir = t.path("mappings");
        let s = save_profile(&dir, &live, " My Layout (v2) ", &sample_actions()).unwrap();
        assert_eq!(s.file, "My Layout (v2).xml");
        assert_eq!(s.name, "My Layout (v2)");
        let xml = fs::read_to_string(dir.join(&s.file)).unwrap();
        assert!(xml.starts_with("<ActionMaps version=\"1\" optionsVersion=\"2\" rebindVersion=\"2\" profileName=\"My Layout (v2)\">"));
        assert!(xml.contains("<CustomisationUIHeader label=\"My Layout (v2)\""));
        assert!(xml.contains("<joystick instance=\"2\"/>"));
        assert!(xml.contains("<rebind input=\"js2_button1\"/>"));
        assert!(!xml.contains("ActionProfiles"));
        assert_eq!(profile_name(&xml).as_deref(), Some("My Layout (v2)"));
        // The bindings survive the rewrite.
        assert_eq!(s.bindings, 1);

        // Same name again: refused. Bad name: refused.
        assert!(save_profile(&dir, &live, "My Layout (v2)", &sample_actions()).unwrap_err().contains("exists"));
        assert!(save_profile(&dir, &live, "///", &sample_actions()).is_err());

        delete(&dir, &s.file).unwrap();
        assert!(!dir.join(&s.file).exists());
        assert!(delete(&dir, "../x.xml").is_err());
        assert!(delete(&dir, "missing.xml").is_err());
    }

    #[test]
    fn export_copies_to_destination() {
        let t = Tmp::new();
        let dir = t.path("profiles");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("layout_a_exported.xml"), layout_xml("A")).unwrap();

        let dest = t.path("out.xml");
        export(&dir, "layout_a_exported.xml", &dest).unwrap();
        assert_eq!(fs::read_to_string(&dest).unwrap(), layout_xml("A"));

        assert!(export(&dir, "missing.xml", &t.path("out2.xml")).unwrap_err().contains("not found"));
        assert!(export(&dir, "../escape.xml", &t.path("out3.xml")).unwrap_err().contains("not a valid"));
    }
}
