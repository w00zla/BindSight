//! Apply a binding profile or a backup to the live files, device by device —
//! `apply_source`, the one entry point of the Bindings and the Config mode's
//! Apply dialog:
//!
//! - **bindings**: for every chosen device the live `actionmaps.xml` ends up
//!   with exactly the source's rebinds for it — the source's rebinds are
//!   written, live rebinds the source lacks are removed (so the shipped
//!   default applies again, as in the source). Through `rebind.rs`, the
//!   out-of-game counterpart of the keybinding screen.
//! - **settings**: the device's `<options>` children and `<deviceoptions>`
//!   values become exactly the source's (`devconfig::apply_settings`), and,
//!   from a backup that holds an `attributes.xml`, the device's managed game
//!   settings (`devconfig::copy_attributes`); a profile or an older backup
//!   leaves those untouched.
//! - a backup with both and every device chosen, each on its own slot
//!   ([`every_device_chosen`]), is put back byte for byte — plus its managed
//!   game settings, never the whole `attributes.xml`.
//!
//! Everything else in the files stays as it is. One verified backup of both
//! files is taken first (`gamefile::replace_live_config`).

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Mutex;

use log::{error, info};
use serde::Deserialize;
use tauri::{AppHandle, State};

use crate::devconfig::{self, DeviceConfig, ManagedAttribute, SlotPair};
use crate::rebind::{self, RebindChange};
use crate::scdata::{parse_actionmaps, parse_rebind, ActionMapsFile, DeviceKind, OptionTree, Rebind};
use crate::{backups, config, diff, gamefile, AppData, LoadStatus};

/// One device to take over: `kb1`, `gp1` or `jsN`. A joystick's bindings
/// can land on another slot (`target`): the source's `js1_` rebinds are
/// written as `js2_` — the resort that goes with an apply when the sticks
/// are enumerated differently here.
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceSel {
    pub kind: DeviceKind,
    pub instance: u32,
    #[serde(default)]
    pub target: Option<u32>,
}

fn prefix(kind: DeviceKind, instance: u32) -> String {
    format!("{}{}_", kind.token_prefix(), instance)
}

fn target_of(d: &DeviceSel) -> u32 {
    match (d.kind, d.target) {
        (DeviceKind::Joystick, Some(t)) => t,
        _ => d.instance,
    }
}

/// `(actionmap, action) -> rebind` for the rebinds of one device in a file,
/// matched by what the input targets.
fn rebinds_of(file: &ActionMapsFile, kind: DeviceKind, instance: u32) -> BTreeMap<(&str, &str), &Rebind> {
    file.rebinds
        .iter()
        .filter(|r| parse_rebind(&r.input).is_some_and(|t| t.kind == kind && t.instance == instance))
        .map(|r| ((r.actionmap.as_str(), r.action.as_str()), r))
        .collect()
}

/// `(actionmap, action) -> (input on the target slot, attributes)`.
type Planned<'a> = BTreeMap<(&'a str, &'a str), (String, &'a [(String, String)])>;

/// The rebind changes that make `live` match `source` for `devices`: the
/// source's rebinds where they differ, a removal where the live file has a
/// rebind the source lacks. Empty when nothing differs.
pub fn plan_apply(live: &ActionMapsFile, source: &ActionMapsFile, devices: &[DeviceSel]) -> Vec<RebindChange> {
    let mut out = Vec::new();
    for d in devices {
        let from_prefix = prefix(d.kind, d.instance);
        let to_prefix = prefix(d.kind, target_of(d));
        // The source's rebinds, renamed onto the target slot (the `jsN_`
        // part only — a modifier in front stays where it is).
        let from: Planned = rebinds_of(source, d.kind, d.instance)
            .into_iter()
            .map(|(k, r)| (k, (r.input.replacen(&from_prefix, &to_prefix, 1), r.attrs.as_slice())))
            .collect();
        let now = rebinds_of(live, d.kind, target_of(d));
        for (&(actionmap, action), (input, attrs)) in &from {
            let same = now
                .get(&(actionmap, action))
                .is_some_and(|r| r.input == *input && r.attrs.as_slice() == *attrs);
            if !same {
                out.push(RebindChange {
                    actionmap: actionmap.into(),
                    action: action.into(),
                    kind: d.kind,
                    input: input.clone(),
                    attrs: Some(attrs.to_vec()),
                });
            }
        }
        for &(actionmap, action) in now.keys() {
            if !from.contains_key(&(actionmap, action)) {
                out.push(RebindChange {
                    actionmap: actionmap.into(),
                    action: action.into(),
                    kind: d.kind,
                    input: String::new(),
                    attrs: None,
                });
            }
        }
    }
    out
}

/// The texts an apply works on.
pub struct ApplyTexts<'a> {
    /// The live `actionmaps.xml`.
    pub live: &'a str,
    /// The live `attributes.xml`, `None` when there is none.
    pub live_attributes: Option<&'a str>,
    /// The profile's or the backup's `actionmaps.xml`.
    pub source: &'a str,
    /// The backup's `attributes.xml`; `None` for a profile and for a backup
    /// made without one.
    pub source_attributes: Option<&'a str>,
    pub source_is_backup: bool,
}

/// What an apply writes.
#[derive(Debug, Default)]
pub struct ApplyPlan {
    /// The new `actionmaps.xml`, `None` when it stays as it is.
    pub actionmaps: Option<String>,
    /// The new `attributes.xml`, `None` when it stays as it is.
    pub attributes: Option<String>,
    /// The backup's `actionmaps.xml` is put back byte for byte.
    pub restore: bool,
    /// Rebind changes in the apply (none for a restore).
    pub rebinds: usize,
}

/// Every device chosen is one the game has, each slot taken once.
fn check_devices(devices: &[DeviceSel]) -> Result<(), String> {
    if devices.is_empty() {
        return Err("No device chosen".into());
    }
    let mut targets = HashSet::new();
    for d in devices {
        let slot = format!("{}{}", d.kind.token_prefix(), target_of(d));
        let valid = match d.kind {
            DeviceKind::Keyboard | DeviceKind::Gamepad => d.instance == 1,
            DeviceKind::Joystick => d.instance >= 1 && target_of(d) >= 1,
        };
        if !valid {
            return Err(format!("invalid device {}{}", d.kind.token_prefix(), d.instance));
        }
        if !targets.insert((d.kind, target_of(d))) {
            return Err(format!("two devices chosen for {slot}"));
        }
    }
    Ok(())
}

/// Whether `devices` covers every device the two files hold anything for,
/// each on its own slot — the rule for putting a backup back byte for byte:
/// the keyboard and the gamepad always, every `jsN` a rebind (a blank one
/// too) or an `<options>` element of either file names. A slot neither file
/// names has nothing a byte-for-byte restore could change.
pub fn every_device_chosen(devices: &[DeviceSel], files: [&ActionMapsFile; 2], configs: [&DeviceConfig; 2]) -> bool {
    if devices.iter().any(|d| target_of(d) != d.instance) {
        return false;
    }
    let mut needed: BTreeSet<(DeviceKind, u32)> = [(DeviceKind::Keyboard, 1), (DeviceKind::Gamepad, 1)].into();
    for f in files {
        needed.extend(
            f.rebinds
                .iter()
                .filter_map(|r| parse_rebind(&r.input))
                .filter(|t| t.kind == DeviceKind::Joystick)
                .map(|t| (DeviceKind::Joystick, t.instance)),
        );
    }
    for c in configs {
        needed.extend(c.options.iter().filter(|o| o.kind == DeviceKind::Joystick).map(|o| (DeviceKind::Joystick, o.instance)));
    }
    needed.iter().all(|n| devices.iter().any(|d| (d.kind, d.instance) == *n))
}

/// Plan an apply of `texts.source` for `devices`: `bindings` and / or
/// `settings` (see the module doc), checked and rewritten in memory —
/// nothing is written here.
pub fn plan_source(texts: &ApplyTexts, devices: &[DeviceSel], bindings: bool, settings: bool, trees: &[OptionTree]) -> Result<ApplyPlan, String> {
    if !bindings && !settings {
        return Err("Nothing to apply".into());
    }
    check_devices(devices)?;
    let live = parse_actionmaps(texts.live)?;
    let source = parse_actionmaps(texts.source)?;
    let live_config = devconfig::parse_device_config(texts.live)?;
    let source_config = devconfig::parse_device_config(texts.source)?;

    let mut plan = ApplyPlan {
        restore: texts.source_is_backup && bindings && settings && every_device_chosen(devices, [&live, &source], [&live_config, &source_config]),
        ..ApplyPlan::default()
    };
    let actionmaps = if plan.restore {
        texts.source.to_string()
    } else {
        let mut out = texts.live.to_string();
        if bindings {
            let changes = plan_apply(&live, &source, devices);
            plan.rebinds = changes.len();
            if !changes.is_empty() {
                out = rebind::apply_rebinds(&out, &changes)?;
            }
        }
        if settings {
            let pairs: Vec<SlotPair> = devices.iter().map(|d| SlotPair { kind: d.kind, source: d.instance, target: target_of(d) }).collect();
            out = devconfig::apply_settings(&out, &source_config, &pairs, trees)?;
        }
        out
    };
    plan.actionmaps = (actionmaps != texts.live).then_some(actionmaps);

    // The game settings travel only with a backup that holds them.
    if let (true, Some(source_attributes)) = (settings, texts.source_attributes) {
        let source_attributes: BTreeMap<String, String> =
            devconfig::parse_attributes(source_attributes).map_err(|e| format!("the backup's game settings are unreadable: {e}"))?;
        let mut managed: Vec<ManagedAttribute> = Vec::new();
        for m in devices.iter().flat_map(|d| devconfig::managed_attributes(d.kind)) {
            if !managed.contains(m) {
                managed.push(*m);
            }
        }
        match texts.live_attributes {
            Some(live_attributes) if !managed.is_empty() => {
                let out = devconfig::copy_attributes(live_attributes, &source_attributes, &managed)?;
                plan.attributes = (out != live_attributes).then_some(out);
            }
            None if managed.iter().any(|m| source_attributes.contains_key(m.attr_name())) => {
                return Err("the game's settings file was not found".into());
            }
            _ => {}
        }
    }
    Ok(plan)
}

/// Apply `source` for `devices` to the live files — `bindings` and / or
/// `settings`, see the module doc. One backup of both files first while
/// auto-backups are on (reason "before restore" for a byte-for-byte
/// restore, else "before apply"), then the atomic writes, then the bindings
/// are reloaded.
#[tauri::command]
pub(crate) fn apply_source(
    source: diff::Source,
    devices: Vec<DeviceSel>,
    bindings: bool,
    settings: bool,
    app: AppHandle,
    data: State<Mutex<AppData>>,
) -> Result<LoadStatus, String> {
    let mut data = data.lock().unwrap();
    if data.bindings_file.is_none() {
        return Err("No bindings loaded".into());
    }
    let read = |path: &std::path::Path| std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()));
    let root = backups::backups_root(&app)?;
    let base = data.config.base_path().to_string();
    let source_xml = diff::source_xml(&source, &base, &root)?;
    let source_attributes = match &source {
        diff::Source::Backup { id } => backups::attributes_path_of(&root, id)?.map(|p| read(&p)).transpose()?,
        _ => None,
    };
    let path = config::actionmaps_path(&base);
    let live = read(&path)?;
    let attributes_path = config::attributes_path(&base);
    let live_attributes = if attributes_path.is_file() { Some(read(&attributes_path)?) } else { None };
    let texts = ApplyTexts {
        live: &live,
        live_attributes: live_attributes.as_deref(),
        source: &source_xml,
        source_attributes: source_attributes.as_deref(),
        source_is_backup: matches!(source, diff::Source::Backup { .. }),
    };
    let plan = plan_source(&texts, &devices, bindings, settings, &data.sc.data.options).map_err(|e| {
        error!("apply of {source:?} refused: {e}");
        e
    })?;

    // The chosen devices with the slot each one lands on, and what was
    // taken over, for the log lines.
    let selection: Vec<String> = devices
        .iter()
        .map(|d| {
            let p = d.kind.token_prefix();
            format!("{p}{}->{p}{}", d.instance, target_of(d))
        })
        .collect();
    let mut what: Vec<String> = Vec::new();
    if plan.restore {
        what.push("restored byte for byte".into());
    } else if bindings {
        what.push(format!("bindings ({} change(s))", plan.rebinds));
    }
    if settings {
        what.push(if plan.attributes.is_some() { "settings incl. game settings" } else { "settings" }.into());
    }
    let (selection, what) = (selection.join(" "), what.join(", "));
    if plan.actionmaps.is_none() && plan.attributes.is_none() {
        info!("apply: {source:?} matches [{selection}] ({what}), nothing to write");
        return Ok(crate::reload_bindings(&mut data));
    }
    let reason = if plan.restore { "before restore" } else { "before apply" };
    let version = data.sc.version.as_ref().map(|v| v.label.as_str());
    let backup = gamefile::replace_live_config(
        &root,
        &path,
        plan.actionmaps.as_deref(),
        plan.attributes.as_deref(),
        reason,
        data.config.auto_backup,
        version,
        &data.sc.data.actions,
    )?;
    info!("applied {source:?}: devices [{selection}], {what} ({})", gamefile::backup_label(&backup));
    Ok(crate::reload_bindings(&mut data))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scdata::Rebind;

    fn file(rebinds: &[(&str, &str, &str)]) -> ActionMapsFile {
        ActionMapsFile {
            joysticks: Vec::new(),
            rebinds: rebinds
                .iter()
                .map(|(m, a, i)| Rebind { actionmap: m.to_string(), action: a.to_string(), input: i.to_string(), attrs: Vec::new() })
                .collect(),
        }
    }

    fn attr(r: Rebind, name: &str, value: &str) -> Rebind {
        Rebind { attrs: vec![(name.to_string(), value.to_string())], ..r }
    }

    #[test]
    fn plan_carries_the_source_rebinds_attributes() {
        let mut live = file(&[("seat", "eject", "js1_button1"), ("seat", "boost", "js1_button2")]);
        let mut source = file(&[("seat", "eject", "js1_button1"), ("seat", "boost", "js1_button2")]);
        // Same inputs, but the source holds eject with activationMode: that
        // alone is a change, and the plan carries the attribute along.
        source.rebinds[0] = attr(source.rebinds[0].clone(), "activationMode", "hold");
        live.rebinds[1] = attr(live.rebinds[1].clone(), "multiTap", "2");
        let plan = plan_apply(&live, &source, &[sel(DeviceKind::Joystick, 1)]);
        let as_text: Vec<String> = plan.iter().map(|c| format!("{}/{}={} {:?}", c.actionmap, c.action, c.input, c.attrs)).collect();
        assert_eq!(
            as_text,
            vec![
                "seat/boost=js1_button2 Some([])",
                "seat/eject=js1_button1 Some([(\"activationMode\", \"hold\")])",
            ]
        );
    }

    fn sel(kind: DeviceKind, instance: u32) -> DeviceSel {
        DeviceSel { kind, instance, target: None }
    }

    #[test]
    fn a_joystick_can_land_on_another_slot() {
        let live = file(&[("seat", "eject", "js2_button1"), ("seat", "menu", "js2_button4"), ("seat", "boost", "js1_button2")]);
        let source = file(&[("seat", "eject", "js1_button9"), ("seat", "lights", "js1_button3")]);
        // Source js1 -> live js2: eject and lights get js2_ tokens, js2's menu
        // goes, js1's boost is not touched.
        let plan = plan_apply(&live, &source, &[DeviceSel { kind: DeviceKind::Joystick, instance: 1, target: Some(2) }]);
        let as_text: Vec<String> = plan.iter().map(|c| format!("{}/{}={}", c.actionmap, c.action, c.input)).collect();
        assert_eq!(as_text, vec!["seat/eject=js2_button9", "seat/lights=js2_button3", "seat/menu="]);
    }

    #[test]
    fn plan_writes_differences_and_removes_extras_per_device() {
        let live = file(&[
            ("seat", "eject", "js1_button1"),
            ("seat", "eject", "kb1_e"),
            ("seat", "boost", "js1_button2"),
            ("seat", "menu", "js2_button1"),
        ]);
        let source = file(&[
            ("seat", "eject", "js1_button9"),
            ("seat", "eject", "kb1_x"),
            ("seat", "lights", "js1_button3"),
        ]);
        let plan = plan_apply(&live, &source, &[sel(DeviceKind::Joystick, 1)]);
        let as_text: Vec<String> = plan.iter().map(|c| format!("{}/{}={}", c.actionmap, c.action, c.input)).collect();
        // js1 only: eject changes, lights is new, boost goes; kb1 and js2 untouched.
        assert_eq!(as_text, vec!["seat/eject=js1_button9", "seat/lights=js1_button3", "seat/boost="]);
        assert!(plan.iter().all(|c| c.kind == DeviceKind::Joystick));

        let plan = plan_apply(&live, &source, &[sel(DeviceKind::Keyboard, 1)]);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].input, "kb1_x");

        // Nothing differs for js2: the source has none, the live one goes.
        let plan = plan_apply(&live, &source, &[sel(DeviceKind::Joystick, 2)]);
        assert_eq!(plan.len(), 1);
        assert_eq!((plan[0].action.as_str(), plan[0].input.as_str()), ("menu", ""));

        // Identical rebinds produce no change.
        assert!(plan_apply(&live, &live, &[sel(DeviceKind::Joystick, 1), sel(DeviceKind::Keyboard, 1)]).is_empty());
    }

    // An SC-shaped live file (LF, one-space indent, an <options> slot).
    const LIVE_XML: &str = concat!(
        "<ActionMaps>\n",
        " <ActionProfiles version=\"1\" optionsVersion=\"2\" rebindVersion=\"2\" profileName=\"default\">\n",
        "  <options type=\"joystick\" instance=\"1\" Product=\" VKB R {0200231D-0000-0000-0000-504944564944}\"/>\n",
        "  <actionmap name=\"spaceship_general\">\n",
        "   <action name=\"v_eject\">\n",
        "    <rebind input=\"js1_button1\"/>\n",
        "    <rebind input=\"kb1_e\"/>\n",
        "   </action>\n",
        "   <action name=\"v_boost\">\n",
        "    <rebind input=\"js1_button2\"/>\n",
        "   </action>\n",
        "  </actionmap>\n",
        "  <actionmap name=\"spaceship_movement\">\n",
        "   <action name=\"v_brake\">\n",
        "    <rebind input=\"js2_button1\"/>\n",
        "   </action>\n",
        "  </actionmap>\n",
        " </ActionProfiles>\n",
        "</ActionMaps>\n",
    );

    /// One rebind as (actionmap, action, input, attributes), comparable.
    type RebindRow = (String, String, String, Vec<(String, String)>);

    /// A device's rebinds in a parsed file, comparable across two files.
    fn js_set(file: &ActionMapsFile, kind: DeviceKind, instance: u32) -> Vec<RebindRow> {
        let mut v: Vec<_> = rebinds_of(file, kind, instance)
            .into_values()
            .map(|r| (r.actionmap.clone(), r.action.clone(), r.input.clone(), r.attrs.clone()))
            .collect();
        v.sort();
        v
    }

    #[test]
    fn plan_then_rewrite_makes_the_live_file_match_the_source() {
        // Source js1: eject moves (with an activationMode), lights is new,
        // and it has no boost — so live's js1 boost must go.
        let mut source = file(&[("spaceship_general", "v_eject", "js1_button9"), ("spaceship_general", "v_lights", "js1_button3")]);
        source.rebinds[0] = attr(source.rebinds[0].clone(), "activationMode", "hold");

        let live = parse_actionmaps(LIVE_XML).unwrap();
        let changes = plan_apply(&live, &source, &[sel(DeviceKind::Joystick, 1)]);

        // The plan's changes are the exact ones apply_rebinds accepts, and the
        // output re-parses: the two tested halves compose.
        let rewritten = rebind::apply_rebinds(LIVE_XML, &changes).unwrap();
        let after = parse_actionmaps(&rewritten).unwrap();

        // js1 now equals the source, input and attributes alike.
        assert_eq!(js_set(&after, DeviceKind::Joystick, 1), js_set(&source, DeviceKind::Joystick, 1));
        // The other kind (kb1) and the other slot (js2) are untouched.
        assert!(after.rebinds.iter().any(|r| r.action == "v_eject" && r.input == "kb1_e"));
        assert!(after.rebinds.iter().any(|r| r.action == "v_brake" && r.input == "js2_button1"));
        // The dropped boost is really gone.
        assert!(!after.rebinds.iter().any(|r| r.input == "js1_button2"));

        // Applying the same source again is now a no-op: the file matches.
        assert!(plan_apply(&after, &source, &[sel(DeviceKind::Joystick, 1)]).is_empty());
    }

    // --- apply_source --------------------------------------------------------

    const TREES: &str = r#"<profile>
  <optiontree type="joystick" instances="8" name="root">
    <optiongroup name="master" UIShowCurve="0" UIShowInvert="0">
      <optiongroup name="joystick_curves" UIShowCurve="-1" UIShowInvert="0">
        <optiongroup name="inversion" UIShowInvert="-1">
          <optiongroup name="flight_move_pitch" UIShowCurve="1" UIShowInvert="1"/>
          <optiongroup name="flight_move_roll" UIShowCurve="1" UIShowInvert="1"/>
        </optiongroup>
      </optiongroup>
    </optiongroup>
  </optiontree>
</profile>"#;

    const STICK_R: &str = " VKB R {0200231D-0000-0000-0000-504944564944}";

    fn trees() -> Vec<OptionTree> {
        crate::scdata::parse_option_trees(TREES, &std::collections::HashMap::new()).unwrap()
    }

    const LIVE: &str = concat!(
        "<ActionMaps>\n",
        " <ActionProfiles version=\"1\" optionsVersion=\"2\" rebindVersion=\"2\" profileName=\"default\">\n",
        "  <deviceoptions name=\" VKB R {0200231D-0000-0000-0000-504944564944}\">\n",
        "   <option input=\"x\" deadzone=\"0.1485\"/>\n",
        "  </deviceoptions>\n",
        "  <options type=\"keyboard\" instance=\"1\" Product=\"Keyboard  {6F1D2B61-D5A0-11CF-BFC7-444553540000}\"/>\n",
        "  <options type=\"gamepad\" instance=\"1\" Product=\"Controller (Gamepad)\"/>\n",
        "  <options type=\"joystick\" instance=\"1\" Product=\" VKB R {0200231D-0000-0000-0000-504944564944}\">\n",
        "   <flight_move_roll invert=\"1\"/>\n",
        "  </options>\n",
        "  <options type=\"joystick\" instance=\"2\" Product=\" VKB L {0201231D-0000-0000-0000-504944564944}\"/>\n",
        "  <actionmap name=\"spaceship_general\">\n",
        "   <action name=\"v_eject\">\n",
        "    <rebind input=\"js1_button1\"/>\n",
        "    <rebind input=\"kb1_e\"/>\n",
        "   </action>\n",
        "  </actionmap>\n",
        " </ActionProfiles>\n",
        "</ActionMaps>\n",
    );

    /// The live file as it was some time ago: other rebinds, other settings.
    fn source() -> String {
        LIVE.replace("deadzone=\"0.1485\"", "deadzone=\"0.2277\"")
            .replace("<flight_move_roll invert=\"1\"/>", "<flight_move_pitch exponent=\"2\"/>")
            .replace("js1_button1", "js1_button9")
            .replace("kb1_e", "kb1_x")
    }

    const LIVE_ATTRS: &str = "<Attributes Version=\"35\">\n <Attr name=\"MouseSensitivity\" value=\"6.06061\"/>\n <Attr name=\"Sensitivity\" value=\"1\"/>\n <Attr name=\"SysSpec\" value=\"3\"/>\n</Attributes>\n";
    const BACKUP_ATTRS: &str = "<Attributes Version=\"35\">\n <Attr name=\"MouseSensitivity\" value=\"8.88889\"/>\n <Attr name=\"SysSpec\" value=\"1\"/>\n</Attributes>\n";

    fn texts<'a>(source: &'a str, source_attributes: Option<&'a str>, backup: bool) -> ApplyTexts<'a> {
        ApplyTexts { live: LIVE, live_attributes: Some(LIVE_ATTRS), source, source_attributes, source_is_backup: backup }
    }

    fn all() -> Vec<DeviceSel> {
        vec![sel(DeviceKind::Keyboard, 1), sel(DeviceKind::Gamepad, 1), sel(DeviceKind::Joystick, 1), sel(DeviceKind::Joystick, 2)]
    }

    #[test]
    fn a_full_backup_is_put_back_byte_for_byte_with_the_managed_game_settings() {
        // CRLF and a BOM: byte for byte means byte for byte.
        let backup = format!("\u{feff}{}", source().replace('\n', "\r\n"));
        let plan = plan_source(&texts(&backup, Some(BACKUP_ATTRS), true), &all(), true, true, &trees()).unwrap();
        assert!(plan.restore);
        assert_eq!(plan.actionmaps.as_deref(), Some(backup.as_str()));
        // The managed settings only: the mouse sensitivity comes back, the
        // pad's (not in the backup) goes, SysSpec stays the live one.
        assert_eq!(
            plan.attributes.as_deref(),
            Some(LIVE_ATTRS.replace("6.06061", "8.88889").replace(" <Attr name=\"Sensitivity\" value=\"1\"/>\n", "").as_str())
        );

        // Anything less is a device-by-device apply.
        let partial = |devices: &[DeviceSel], bindings: bool, settings: bool, backup_flag: bool| {
            plan_source(&texts(&backup, Some(BACKUP_ATTRS), backup_flag), devices, bindings, settings, &trees()).unwrap().restore
        };
        assert!(!partial(&all()[..3], true, true, true), "js2 left out");
        let swapped = vec![
            sel(DeviceKind::Keyboard, 1),
            sel(DeviceKind::Gamepad, 1),
            DeviceSel { kind: DeviceKind::Joystick, instance: 1, target: Some(2) },
            DeviceSel { kind: DeviceKind::Joystick, instance: 2, target: Some(1) },
        ];
        assert!(!partial(&swapped, true, true, true), "joysticks moved");
        assert!(!partial(&all(), true, false, true), "bindings alone");
        assert!(!partial(&all(), false, true, true), "settings alone");
        assert!(!partial(&all(), true, true, false), "a profile");
        // A slot only the backup's rebinds name counts too.
        let js3 = backup.replace("kb1_x", "js3_button1");
        assert!(!plan_source(&texts(&js3, None, true), &all(), true, true, &trees()).unwrap().restore);
    }

    #[test]
    fn bindings_and_settings_are_taken_over_per_device() {
        let src = source();
        let js1 = [sel(DeviceKind::Joystick, 1)];
        let plan = plan_source(&texts(&src, None, false), &js1, true, true, &trees()).unwrap();
        assert!(!plan.restore);
        assert_eq!(plan.rebinds, 1);
        let out = plan.actionmaps.unwrap();
        // js1's rebind, slot and device settings are the source's; the
        // keyboard was not chosen and keeps its rebind.
        assert!(out.contains("<rebind input=\"js1_button9\"/>") && out.contains("<rebind input=\"kb1_e\"/>"), "{out}");
        assert!(out.contains("   <flight_move_pitch exponent=\"2\"/>\n  </options>") && !out.contains("flight_move_roll"), "{out}");
        assert!(out.contains("<option input=\"x\" deadzone=\"0.2277\"/>"), "{out}");
        assert_eq!(plan.attributes, None, "a profile carries no game settings");

        // One of the two alone leaves the other part as it is.
        let bindings = plan_source(&texts(&src, None, false), &js1, true, false, &trees()).unwrap().actionmaps.unwrap();
        assert!(bindings.contains("js1_button9") && bindings.contains("flight_move_roll") && bindings.contains("0.1485"));
        let settings = plan_source(&texts(&src, None, false), &js1, false, true, &trees()).unwrap().actionmaps.unwrap();
        assert!(settings.contains("js1_button1") && !settings.contains("flight_move_roll") && settings.contains("0.2277"));

        // Source js1 onto live js2: the settings follow the slot and land on
        // js2's device (VKB L), which had none.
        let moved = [DeviceSel { kind: DeviceKind::Joystick, instance: 1, target: Some(2) }];
        let out = plan_source(&texts(&src, None, false), &moved, false, true, &trees()).unwrap().actionmaps.unwrap();
        let config = devconfig::parse_device_config(&out).unwrap();
        let js2 = config.options.iter().find(|o| o.kind == DeviceKind::Joystick && o.instance == 2).unwrap();
        assert_eq!(js2.nodes.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(), ["flight_move_pitch"]);
        let l = config.device_options.iter().find(|d| d.name.contains("VKB L")).unwrap();
        assert_eq!(l.entries[0].attrs, vec![("deadzone".to_string(), "0.2277".to_string())]);
        assert!(out.contains(&format!("<deviceoptions name=\"{STICK_R}\">\n   <option input=\"x\" deadzone=\"0.1485\"/>")), "js1's device untouched");

        // Nothing differs: nothing to write.
        let plan = plan_source(&texts(LIVE, Some(LIVE_ATTRS), true), &all(), true, true, &trees()).unwrap();
        assert!(plan.actionmaps.is_none() && plan.attributes.is_none());
    }

    #[test]
    fn game_settings_come_only_from_a_backup_that_holds_them() {
        let src = source();
        let kb = [sel(DeviceKind::Keyboard, 1)];
        // Keyboard: the mouse's settings, never the pad's.
        let plan = plan_source(&texts(&src, Some(BACKUP_ATTRS), true), &kb, false, true, &trees()).unwrap();
        assert_eq!(plan.attributes.as_deref(), Some(LIVE_ATTRS.replace("6.06061", "8.88889").as_str()));
        // Gamepad: its sensitivity, missing in the backup, goes.
        let plan = plan_source(&texts(&src, Some(BACKUP_ATTRS), true), &[sel(DeviceKind::Gamepad, 1)], false, true, &trees()).unwrap();
        assert_eq!(plan.attributes.as_deref(), Some(LIVE_ATTRS.replace(" <Attr name=\"Sensitivity\" value=\"1\"/>\n", "").as_str()));
        // An older backup without the file, a profile, a joystick, the
        // bindings alone: the live game settings stay untouched.
        assert_eq!(plan_source(&texts(&src, None, true), &kb, false, true, &trees()).unwrap().attributes, None);
        assert_eq!(plan_source(&texts(&src, None, false), &kb, false, true, &trees()).unwrap().attributes, None);
        assert_eq!(plan_source(&texts(&src, Some(BACKUP_ATTRS), true), &[sel(DeviceKind::Joystick, 1)], false, true, &trees()).unwrap().attributes, None);
        assert_eq!(plan_source(&texts(&src, Some(BACKUP_ATTRS), true), &kb, true, false, &trees()).unwrap().attributes, None);
        // No live settings file to write them into: refused, not skipped.
        let mut t = texts(&src, Some(BACKUP_ATTRS), true);
        t.live_attributes = None;
        assert!(plan_source(&t, &kb, true, true, &trees()).unwrap_err().contains("not found"));
        // A backup whose settings file is broken is refused.
        assert!(plan_source(&texts(&src, Some("<Attributes><oops"), true), &kb, false, true, &trees()).is_err());
    }

    #[test]
    fn an_apply_needs_something_to_do_and_valid_devices() {
        let src = source();
        let t = texts(&src, None, false);
        assert!(plan_source(&t, &all(), false, false, &trees()).is_err());
        assert!(plan_source(&t, &[], true, true, &trees()).is_err());
        assert!(plan_source(&t, &[sel(DeviceKind::Keyboard, 2)], true, true, &trees()).is_err());
        assert!(plan_source(&t, &[sel(DeviceKind::Joystick, 0)], true, true, &trees()).is_err());
        let clash = [sel(DeviceKind::Joystick, 2), DeviceSel { kind: DeviceKind::Joystick, instance: 1, target: Some(2) }];
        assert!(plan_source(&t, &clash, true, true, &trees()).unwrap_err().contains("js2"));
        // An unreadable source is refused before anything is planned.
        assert!(plan_source(&texts("<ActionMaps><oops", None, false), &all(), true, true, &trees()).is_err());
    }
}
