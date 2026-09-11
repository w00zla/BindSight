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

use log::info;
use serde::Deserialize;
use tauri::{AppHandle, State};

use crate::rebind::{self, RebindChange};
use crate::scdata::{parse_actionmaps, ActionMapsFile, DeviceKind};
use crate::{backups, config, diff, AppData, LoadStatus};

/// One device to take over: `kb1`, `gp1` or `jsN`.
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceSel {
    pub kind: DeviceKind,
    pub instance: u32,
}

fn prefix(d: &DeviceSel) -> String {
    format!("{}{}_", d.kind.token_prefix(), d.instance)
}

/// `(actionmap, action) -> input` for the rebinds of one device in a file.
fn rebinds_of<'a>(file: &'a ActionMapsFile, prefix: &str) -> BTreeMap<(&'a str, &'a str), &'a str> {
    file.rebinds
        .iter()
        .filter(|r| r.input.starts_with(prefix))
        .map(|r| ((r.actionmap.as_str(), r.action.as_str()), r.input.as_str()))
        .collect()
}

/// The rebind changes that make `live` match `source` for `devices`: the
/// source's rebinds where they differ, a removal where the live file has a
/// rebind the source lacks. Empty when nothing differs.
pub fn plan_apply(live: &ActionMapsFile, source: &ActionMapsFile, devices: &[DeviceSel]) -> Vec<RebindChange> {
    let mut out = Vec::new();
    for d in devices {
        let p = prefix(d);
        let from = rebinds_of(source, &p);
        let now = rebinds_of(live, &p);
        for (&(actionmap, action), &input) in &from {
            if now.get(&(actionmap, action)) != Some(&input) {
                out.push(RebindChange {
                    actionmap: actionmap.into(),
                    action: action.into(),
                    kind: d.kind,
                    input: input.into(),
                });
            }
        }
        for &(actionmap, action) in now.keys() {
            if !from.contains_key(&(actionmap, action)) {
                out.push(RebindChange { actionmap: actionmap.into(), action: action.into(), kind: d.kind, input: String::new() });
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
    if changes.is_empty() {
        info!("apply: nothing differs for the chosen devices");
        return Ok(crate::reload_bindings(&mut data));
    }
    let rewritten = rebind::apply_rebinds(&xml, &changes)?;

    let backup = if data.config.auto_backup {
        let version = data.sc.version.as_ref().map(|v| v.label.as_str());
        Some(backups::create(&backups_root, &path, "before apply", version, &data.sc.data.actions)?.id)
    } else {
        None
    };
    std::fs::write(&path, rewritten).map_err(|e| format!("write {}: {e}", path.display()))?;
    let backup = backup.map_or_else(|| "auto-backup off".to_string(), |id| format!("backup {id}"));
    info!("applied {:?} to {}: {} change(s) ({backup})", source, path.display(), changes.len());
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
                .map(|(m, a, i)| Rebind { actionmap: m.to_string(), action: a.to_string(), input: i.to_string() })
                .collect(),
        }
    }

    fn sel(kind: DeviceKind, instance: u32) -> DeviceSel {
        DeviceSel { kind, instance }
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
}
