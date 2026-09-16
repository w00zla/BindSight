//! Apply a binding profile or a backup to the live `actionmaps.xml`, device
//! by device: for every chosen device the live file ends up with exactly the
//! source's rebinds for it — the source's rebinds are written, live rebinds
//! the source lacks are removed (so the shipped default applies again, as
//! in the source). Everything else in the file (other devices, `<options>`,
//! `<deviceoptions>`) stays as it is. The write goes through `rebind.rs`, the
//! out-of-game counterpart of the keybinding screen, with an auto-backup
//! first.

use std::collections::BTreeMap;
use std::sync::Mutex;

use log::{error, info};
use serde::Deserialize;
use tauri::{AppHandle, State};

use crate::rebind::{self, RebindChange};
use crate::scdata::{parse_actionmaps, parse_rebind, ActionMapsFile, DeviceKind, Rebind};
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

/// Apply `source` for `devices` to the live file (auto-backup first, reason
/// "before apply"), then reload the bindings.
#[tauri::command]
pub(crate) fn apply_bindings(
    source: diff::Source,
    devices: Vec<DeviceSel>,
    app: AppHandle,
    data: State<Mutex<AppData>>,
) -> Result<LoadStatus, String> {
    let mut data = data.lock().unwrap();
    if data.bindings_file.is_none() {
        return Err("No bindings loaded".into());
    }
    if devices.is_empty() {
        return Err("No device chosen".into());
    }
    let backups_root = backups::backups_root(&app)?;
    let source_xml = diff::source_xml(&source, data.config.base_path(), &backups_root)?;
    let source_file = parse_actionmaps(&source_xml)?;

    let path = config::actionmaps_path(data.config.base_path());
    let xml = std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let live = parse_actionmaps(&xml)?;

    let changes = plan_apply(&live, &source_file, &devices);
    // The chosen devices with the slot each one lands on, for the log lines.
    let selection: Vec<String> = devices
        .iter()
        .map(|d| {
            let p = d.kind.token_prefix();
            format!("{p}{}->{p}{}", d.instance, target_of(d))
        })
        .collect();
    if changes.is_empty() {
        info!("apply: {:?} matches [{}], nothing to write", source, selection.join(" "));
        return Ok(crate::reload_bindings(&mut data));
    }
    let rewritten = rebind::apply_rebinds(&xml, &changes).map_err(|e| {
        error!("apply rewrite refused ({} change(s)): {e}", changes.len());
        e
    })?;

    let version = data.sc.version.as_ref().map(|v| v.label.as_str());
    let backup = gamefile::replace_live_file(&backups_root, &path, &rewritten, "before apply", data.config.auto_backup, version, &data.sc.data.actions)?;
    info!(
        "applied {:?} to {}: devices [{}], {} change(s) ({})",
        source,
        path.display(),
        selection.join(" "),
        changes.len(),
        gamefile::backup_label(&backup)
    );
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
}
