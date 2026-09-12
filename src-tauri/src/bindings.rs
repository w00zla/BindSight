//! Bridge between live joystick input and SC's bound actions: build the SC
//! input token for a physical input, and look up what that token is bound to.
//!
//! Flow at runtime: a live event carries the device's SDL GUID -> derive its SC
//! Product GUID -> [`instance_for_guid`] gives the `jsN` number -> [`button_token`]
//! / [`hat_token`] builds the token -> [`BindingIndex::resolve`] returns the
//! bound action(s).

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::gamelog::{GameLogError, LogEnumeration};
use crate::input::DeviceInfo;
use crate::scdata::{parse_rebind, Action, ActionMap, DeviceKind, ActionMapsFile};

/// An action a token is bound to, with the context (actionmap) it applies in.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BoundAction {
    pub actionmap: String,
    pub action: String,
    pub label: Option<String>,
    /// Comes from `defaultProfile.xml`'s default for this device kind (always
    /// on instance 1), not from a user rebind.
    pub is_default: bool,
    /// Which device kind the token belongs to.
    pub device_kind: DeviceKind,
}

/// Instance SC applies `defaultProfile.xml`'s unnumbered defaults to
/// (`joystick="button1"` means `js1_button1`) — confirmed by the user.
/// Keyboard and gamepad only ever have instance 1 anyway.
const DEFAULT_INSTANCE: u32 = 1;

/// Full SC token for an unnumbered joystick default from `defaultProfile.xml`.
pub fn default_token(token: &str) -> String {
    format!("js{DEFAULT_INSTANCE}_{token}")
}

/// Full SC token for an unnumbered default of any device kind.
fn default_token_of(kind: DeviceKind, token: &str) -> String {
    format!("{}{DEFAULT_INSTANCE}_{token}", kind.token_prefix())
}

/// The default bindings an action ships with for one device kind. The
/// keyboard kind carries up to two: `defaultProfile.xml`'s `keyboard=` and
/// `mouse=` (SC binds the mouse as `kb1_mouse1`, one device).
fn defaults_of(action: &Action, kind: DeviceKind) -> Vec<&String> {
    let slots = match kind {
        DeviceKind::Joystick => vec![&action.joystick_default],
        DeviceKind::Keyboard => vec![&action.keyboard_default, &action.mouse_default],
        DeviceKind::Gamepad => vec![&action.gamepad_default],
    };
    let mut out: Vec<&String> = Vec::new();
    for token in slots.into_iter().flatten() {
        if !out.contains(&token) {
            out.push(token);
        }
    }
    out
}

/// Every device kind that carries defaults and bindings, in display order.
const KINDS: [DeviceKind; 3] = [DeviceKind::Joystick, DeviceKind::Keyboard, DeviceKind::Gamepad];

/// Actions whose binding for a device kind the user touched in
/// `actionmaps.xml` — any rebind naming that device, including a blank one
/// (`js1_ `, `kb1_ `) that unbinds the shipped default. For those the default
/// no longer applies. "Touched" is per kind: a keyboard rebind leaves the
/// joystick default alone, and vice versa.
fn touched_per_kind(profile: &ActionMapsFile) -> HashSet<(DeviceKind, &str, &str)> {
    profile
        .rebinds
        .iter()
        .filter_map(|r| {
            let target = parse_rebind(&r.input)?;
            Some((target.kind, r.actionmap.as_str(), r.action.as_str()))
        })
        .collect()
}

/// Lookup from an SC input token (e.g. `"js2_button9"`) to the actions bound to
/// it. One token can map to several actions in different actionmaps/contexts.
#[derive(Debug, Default)]
pub struct BindingIndex {
    by_token: HashMap<String, Vec<BoundAction>>,
}

impl BindingIndex {
    /// Build the index from the action master list and the user's rebinds.
    pub fn build(maps: &[ActionMap], profile: &ActionMapsFile) -> Self {
        // (actionmap, action) -> resolved label
        let mut label_of: HashMap<(&str, &str), Option<String>> = HashMap::new();
        for map in maps {
            for action in &map.actions {
                label_of.insert((map.name.as_str(), action.name.as_str()), action.label.clone());
            }
        }

        let mut by_token: HashMap<String, Vec<BoundAction>> = HashMap::new();
        for rebind in &profile.rebinds {
            let Some(target) = parse_rebind(&rebind.input) else { continue };
            let Some(full_token) = target.full_token() else { continue };
            let label = label_of
                .get(&(rebind.actionmap.as_str(), rebind.action.as_str()))
                .cloned()
                .flatten();
            by_token.entry(full_token).or_default().push(BoundAction {
                actionmap: rebind.actionmap.clone(),
                action: rebind.action.clone(),
                label,
                is_default: false,
                device_kind: target.kind,
            });
        }

        // Shipped defaults (always instance 1) for every device kind the user
        // never touched for that action.
        let touched = touched_per_kind(profile);
        for map in maps {
            for action in &map.actions {
                for kind in KINDS {
                    if touched.contains(&(kind, map.name.as_str(), action.name.as_str())) {
                        continue;
                    }
                    for token in defaults_of(action, kind) {
                        by_token.entry(default_token_of(kind, token)).or_default().push(BoundAction {
                            actionmap: map.name.clone(),
                            action: action.name.clone(),
                            label: action.label.clone(),
                            is_default: true,
                            device_kind: kind,
                        });
                    }
                }
            }
        }

        Self { by_token }
    }

    /// Actions bound to a token, or an empty slice if none.
    pub fn resolve(&self, token: &str) -> &[BoundAction] {
        self.by_token.get(token).map(Vec::as_slice).unwrap_or(&[])
    }
}

/// SC token for a button press. SDL button index `i` maps to SC `button(i+1)`
/// (the `+1` offset, verified under Wine).
pub fn button_token(instance: u32, sdl_button_index: u8) -> String {
    format!("js{instance}_button{}", sdl_button_index as u32 + 1)
}

/// The SC token for an axis by its SC axis name (`x`, `rotz`, `slider1`, …
/// from the HID descriptor, see `hid.rs`): `js2_rotz`.
pub fn axis_token(instance: u32, axis: &str) -> String {
    format!("js{instance}_{axis}")
}

/// SC token for a hat direction, or `None` for a diagonal/centered state that
/// has no single SC cardinal token. SDL hat index `h` maps to SC `hat(h+1)`.
pub fn hat_token(instance: u32, sdl_hat_index: u8, direction: &str) -> Option<String> {
    match direction {
        "up" | "right" | "down" | "left" => {
            Some(format!("js{instance}_hat{}_{direction}", sdl_hat_index as u32 + 1))
        }
        _ => None,
    }
}

/// One user binding, resolved to a device and a label for display.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResolvedBinding {
    pub token: String,
    /// Device name recorded for this binding's `jsN` instance (from
    /// `<options>`), or the fixed `"Keyboard"` / `"Gamepad"`.
    pub device: Option<String>,
    /// SC Product GUID of that instance, to match against connected devices.
    /// Always `None` for keyboard and gamepad.
    pub device_guid: Option<String>,
    /// Which device kind the token belongs to.
    pub device_kind: DeviceKind,
    /// SC instance: `jsN` for joysticks, always 1 for keyboard and gamepad.
    pub instance: u32,
    pub actionmap: String,
    pub action: String,
    pub label: Option<String>,
    /// A shipped default from `defaultProfile.xml` (on instance 1), not a user
    /// rebind.
    pub is_default: bool,
}

/// The fixed device name for a kind without a `<options>` entry of its own.
fn fixed_device_name(kind: DeviceKind) -> Option<String> {
    match kind {
        DeviceKind::Joystick => None,
        DeviceKind::Keyboard => Some("Keyboard/Mouse".to_string()),
        DeviceKind::Gamepad => Some("Gamepad".to_string()),
    }
}

/// Flatten the user's rebinds into resolved bindings: each real (bound) input
/// with its SC token, the device it sits on, and the action's label. Unbound
/// (blank) rebinds are skipped. Shipped instance-1 defaults are
/// appended for every action/device kind the user never touched (see
/// [`touched_per_kind`]).
pub fn resolve_bindings(maps: &[ActionMap], profile: &ActionMapsFile) -> Vec<ResolvedBinding> {
    let mut label_of: HashMap<(&str, &str), Option<String>> = HashMap::new();
    for map in maps {
        for action in &map.actions {
            label_of.insert((map.name.as_str(), action.name.as_str()), action.label.clone());
        }
    }
    let device_of: HashMap<u32, (&str, Option<&str>)> = profile
        .joysticks
        .iter()
        .map(|d| (d.instance, (d.product_name.as_str(), d.product_guid.as_deref())))
        .collect();
    // Joysticks take their name/GUID from `<options>`; kb/gp are fixed.
    let device_for = |kind: DeviceKind, instance: u32| match kind {
        DeviceKind::Joystick => {
            let d = device_of.get(&instance);
            (d.map(|d| d.0.to_string()), d.and_then(|d| d.1.map(String::from)))
        }
        _ => (fixed_device_name(kind), None),
    };

    let mut out = Vec::new();
    for rebind in &profile.rebinds {
        let Some(target) = parse_rebind(&rebind.input) else { continue };
        let Some(token) = target.full_token() else { continue };
        let (device, device_guid) = device_for(target.kind, target.instance);
        out.push(ResolvedBinding {
            token,
            device,
            device_guid,
            device_kind: target.kind,
            instance: target.instance,
            actionmap: rebind.actionmap.clone(),
            action: rebind.action.clone(),
            label: label_of
                .get(&(rebind.actionmap.as_str(), rebind.action.as_str()))
                .cloned()
                .flatten(),
            is_default: false,
        });
    }

    let touched = touched_per_kind(profile);
    for map in maps {
        for action in &map.actions {
            for kind in KINDS {
                if touched.contains(&(kind, map.name.as_str(), action.name.as_str())) {
                    continue;
                }
                for token in defaults_of(action, kind) {
                    let (device, device_guid) = device_for(kind, DEFAULT_INSTANCE);
                    out.push(ResolvedBinding {
                        token: default_token_of(kind, token),
                        device,
                        device_guid,
                        device_kind: kind,
                        instance: DEFAULT_INSTANCE,
                        actionmap: map.name.clone(),
                        action: action.name.clone(),
                        label: action.label.clone(),
                        is_default: true,
                    });
                }
            }
        }
    }
    out
}

/// One joystick in SC's device list: the `jsN` SC assigns it (its 1-based
/// position in SC's own enumeration from `Game.log`) versus the `jsN` recorded
/// for its GUID in the saved `<options>` block.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SlotStatus {
    /// `jsN` SC assigns: 1-based rank in SC's enumeration.
    pub effective_instance: u32,
    /// `jsN` recorded for this device's GUID in `<options>`, or `None` if the
    /// device is not in the saved profile at all.
    pub stored_instance: Option<u32>,
    pub sc_product_guid: Option<String>,
    pub name: Option<String>,
    /// The device is in the saved profile but under a different `jsN` than it now
    /// gets — every binding on its slot lands on the wrong stick.
    pub clash: bool,
    /// Whether SDL sees the device right now. A slot can be in SC's last-start
    /// list yet unplugged since — SC will renumber on its next start.
    pub connected_now: bool,
}

/// A saved `<options>` joystick slot whose device is not in SC's device list
/// (not seen at the last game start). Its bindings dangle, and every device
/// enumerated after it shifts down a slot.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MissingSlot {
    pub stored_instance: u32,
    pub name: String,
    pub sc_product_guid: Option<String>,
}

/// A device SDL sees that SC did not list at its last start: either hidden from
/// SC by Wine, or plugged in after SC started. The log alone can't tell which.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UnseenDevice {
    pub name: Option<String>,
    pub sc_product_guid: Option<String>,
}

/// One step of the resort: every binding saved under `js{from}` belongs on
/// `js{to}`. `name` is the device whose bindings these are (the device SC now
/// lists at `to`, or a saved device SC does not list), `None` for a slot with
/// no saved device.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResortMove {
    pub from: u32,
    pub to: u32,
    pub name: Option<String>,
}

/// Result of comparing SC's saved instance→device map against SC's actual
/// device order, to surface the SC "device order" binding-switch bug. Without
/// a usable `Game.log` there is no order and the report is empty except for
/// `log_error` — nothing is derived from SDL's order, which is not SC's.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ClashReport {
    /// Joysticks in SC's order, each with effective vs stored `jsN`.
    pub connected: Vec<SlotStatus>,
    /// Saved occupied slots not in SC's device list (they cause the shift).
    pub missing: Vec<MissingSlot>,
    /// SDL-visible devices absent from SC's list; informational, never a
    /// clash by itself.
    pub unseen: Vec<UnseenDevice>,
    /// Raw log time of SC's enumeration, when the log was usable.
    pub log_timestamp: Option<String>,
    /// Why `Game.log` was unusable. When set, everything else is empty.
    pub log_error: Option<GameLogError>,
    pub has_clash: bool,
    /// The slot permutation that puts every listed device's bindings on the
    /// `jsN` SC now assigns it (see [`plan_resort`]). Empty when nothing moves.
    pub resort: Vec<ResortMove>,
    /// The same permutation as in-game console commands, to be entered in
    /// order: `pp_resortdevices joystick A B` moves the bindings of `jsA` to
    /// `jsB` (and B's to A), so each cycle becomes a chain of swaps.
    pub resort_commands: Vec<String>,
    /// SC listed a gamepad (`Connected xinput0: …`) at its last start. Pads
    /// have no slot/order of their own; this is purely "does SC see it".
    pub gamepad_seen: bool,
}

/// Case-insensitive SC Product GUID equality; `None` never matches.
fn guid_eq(a: Option<&str>, b: Option<&str>) -> bool {
    matches!((a, b), (Some(a), Some(b)) if a.eq_ignore_ascii_case(b))
}

/// Drop devices the user declared invisible to SC ("SC doesn't see this
/// device", e.g. a keyboard Wine hides from the game). They then count as
/// unplugged for the analysis. Matched by SC Product GUID, case-insensitively.
pub fn without_excluded(devices: &[DeviceInfo], excluded: &[String]) -> Vec<DeviceInfo> {
    devices
        .iter()
        .filter(|d| !excluded.iter().any(|g| guid_eq(Some(g), d.sc_product_guid.as_deref())))
        .cloned()
        .collect()
}

/// Compare the saved `<options>` order against SC's actual device order. SC
/// assigns `jsN` purely by enumeration position and ignores name/GUID
/// (validated by the user: `pp_resortdevices` is only ever needed because of
/// this). So a device whose SC rank differs from its saved `jsN` — or a saved
/// device SC does not list — means its bindings land on the wrong stick.
///
/// `log` is SC's own enumeration from `Game.log`, the only order source.
/// Without it the report carries just the error.
pub fn analyze_clash(
    profile: &ActionMapsFile,
    devices: &[DeviceInfo],
    log: Result<&LogEnumeration, GameLogError>,
) -> ClashReport {
    let log = match log {
        Ok(log) => log,
        Err(err) => return ClashReport { log_error: Some(err), ..Default::default() },
    };

    // Only joysticks have an SC slot; the keyboard and gamepads never take
    // part in the order, and must not turn up as "unseen" either.
    let devices: Vec<&DeviceInfo> = devices.iter().filter(|d| d.kind == DeviceKind::Joystick).collect();

    // A log with only `xinput` lines says SC saw no joystick at all: no
    // order to hold the saved slots against, so none of them is "missing".
    if log.joysticks.is_empty() {
        return ClashReport {
            log_error: Some(GameLogError::NoJoystickLines),
            unseen: devices
                .iter()
                .map(|d| UnseenDevice { name: d.sc_name.clone(), sc_product_guid: d.sc_product_guid.clone() })
                .collect(),
            log_timestamp: log.timestamp.clone(),
            gamepad_seen: !log.gamepads.is_empty(),
            ..Default::default()
        };
    }

    let mut connected = Vec::with_capacity(log.joysticks.len());
    for j in &log.joysticks {
        let stored_instance = j.product_guid.as_deref().and_then(|g| instance_for_guid(profile, g));
        let clash = matches!(stored_instance, Some(i) if i != j.instance);
        connected.push(SlotStatus {
            effective_instance: j.instance,
            stored_instance,
            sc_product_guid: j.product_guid.clone(),
            name: Some(j.product_name.clone()),
            clash,
            connected_now: devices
                .iter()
                .any(|d| guid_eq(d.sc_product_guid.as_deref(), j.product_guid.as_deref())),
        });
    }

    let listed = |guid: Option<&str>| log.joysticks.iter().any(|j| guid_eq(j.product_guid.as_deref(), guid));

    let missing: Vec<MissingSlot> = profile
        .joysticks
        .iter()
        .filter(|js| js.product_guid.is_some() && !listed(js.product_guid.as_deref()))
        .map(|js| MissingSlot {
            stored_instance: js.instance,
            name: js.product_name.clone(),
            sc_product_guid: js.product_guid.clone(),
        })
        .collect();

    let unseen = devices
        .iter()
        .filter(|d| !listed(d.sc_product_guid.as_deref()))
        .map(|d| UnseenDevice { name: d.sc_name.clone(), sc_product_guid: d.sc_product_guid.clone() })
        .collect();

    let resort = plan_resort(&connected, &missing);
    let resort_commands = resort_commands(&resort);
    // A clash is what a resort would fix: a saved slot on the wrong device,
    // or a missing device whose slot shifts the others. A missing device on
    // the highest slot moves nothing — it is listed, not a clash.
    let has_clash = !resort.is_empty();

    ClashReport {
        connected,
        missing,
        unseen,
        log_timestamp: log.timestamp.clone(),
        log_error: None,
        has_clash,
        resort,
        resort_commands,
        gamepad_seen: !log.gamepads.is_empty(),
    }
}

/// The slot permutation that fixes the clash: for every device SC lists that
/// is in the saved profile, its saved slot maps to the slot SC now assigns it.
/// The remaining slots — saved slots of devices SC does not list, and SC slots
/// holding devices without saved bindings — are paired off in ascending order
/// so the result is a bijection over all slots involved: dangling bindings
/// are kept, only renumbered, never dropped. Identity moves are omitted.
pub fn plan_resort(connected: &[SlotStatus], missing: &[MissingSlot]) -> Vec<ResortMove> {
    use std::collections::BTreeSet;

    // Every slot involved: saved ones and SC's current ones.
    let mut slots: BTreeSet<u32> = connected.iter().map(|s| s.effective_instance).collect();
    slots.extend(connected.iter().filter_map(|s| s.stored_instance));
    slots.extend(missing.iter().map(|m| m.stored_instance));

    let mut moves = Vec::new();
    let mut used_from = BTreeSet::new();
    let mut used_to = BTreeSet::new();
    for s in connected {
        let Some(from) = s.stored_instance else {
            continue;
        };
        // A second device under the same saved slot (duplicate GUID) has no
        // slot of its own to move; it takes a leftover like an unsaved one.
        if !used_from.insert(from) {
            continue;
        }
        used_to.insert(s.effective_instance);
        moves.push(ResortMove { from, to: s.effective_instance, name: s.name.clone() });
    }

    let free_from = slots.iter().filter(|n| !used_from.contains(n));
    let free_to = slots.iter().filter(|n| !used_to.contains(n));
    for (&from, &to) in free_from.zip(free_to) {
        let name = missing.iter().find(|m| m.stored_instance == from).map(|m| m.name.clone());
        moves.push(ResortMove { from, to, name });
    }

    moves.retain(|m| m.from != m.to);
    moves.sort_by_key(|m| m.from);
    moves
}

/// Express a slot permutation as `pp_resortdevices joystick A B` console
/// commands, one per swap, to be entered in order. Each cycle
/// `a1 -> a2 -> ... -> ak` (bindings of `a1` go to `a2`, ...) becomes the
/// swaps `(a1 a2) (a1 a3) ... (a1 ak)`: after each swap `a1` holds the
/// bindings that still have to travel on.
pub fn resort_commands(moves: &[ResortMove]) -> Vec<String> {
    use std::collections::{BTreeMap, BTreeSet};

    let next: BTreeMap<u32, u32> = moves.iter().map(|m| (m.from, m.to)).collect();
    let mut done = BTreeSet::new();
    let mut commands = Vec::new();
    for &start in next.keys() {
        if !done.insert(start) {
            continue;
        }
        let mut cur = next[&start];
        while cur != start {
            done.insert(cur);
            commands.push(format!("pp_resortdevices joystick {start} {cur}"));
            cur = next[&cur];
        }
    }
    commands
}

/// Find the SC `jsN` instance for a device by its SC Product GUID.
pub fn instance_for_guid(profile: &ActionMapsFile, sc_product_guid: &str) -> Option<u32> {
    profile
        .joysticks
        .iter()
        .find(|d| {
            d.product_guid
                .as_deref()
                .is_some_and(|g| g.eq_ignore_ascii_case(sc_product_guid))
        })
        .map(|d| d.instance)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scdata::{parse_default_profile, parse_actionmaps};

    /// A minimal connected device for clash tests; only GUID/name/order matter.
    fn dev(guid: &str, name: &str) -> DeviceInfo {
        DeviceInfo {
            sc_name: Some(name.to_string()),
            sdl_name: name.to_string(),
            sc_product_guid: Some(guid.to_string()),
            ..DeviceInfo::default()
        }
    }

    // Real product GUIDs from the measured setup (temp/joyenumtest).
    const KEYCHRON_K2HE: &str = "{0E213434-0000-0000-0000-504944564944}";
    const VKB_L: &str = "{0201231D-0000-0000-0000-504944564944}";
    const VKB_R: &str = "{0200231D-0000-0000-0000-504944564944}";

    /// SC's enumeration as `Game.log` would log it: one `Connected joystickN`
    /// line per (name, guid), in order.
    fn log_of(devices: &[(&str, &str)]) -> LogEnumeration {
        let text: String = devices
            .iter()
            .enumerate()
            .map(|(i, (name, guid))| format!("<2026-09-08T21:06:00.762Z> - Connected joystick{i}: {name} {guid}\n"))
            .collect();
        crate::gamelog::parse(&text).unwrap()
    }

    #[test]
    fn detects_device_order_clash() {
        // Saved order: js1 = Keychron, js2 = VKB L, js3 = VKB R.
        let xml = r#"<ActionMaps>
          <options type="joystick" instance="1" Product="Keychron K2 HE  {0E213434-0000-0000-0000-504944564944}"/>
          <options type="joystick" instance="2" Product=" VKB L {0201231D-0000-0000-0000-504944564944}"/>
          <options type="joystick" instance="3" Product=" VKB R {0200231D-0000-0000-0000-504944564944}"/>
        </ActionMaps>"#;
        let profile = parse_actionmaps(xml).unwrap();
        let all = [dev(KEYCHRON_K2HE, "Keychron"), dev(VKB_L, "VKB L"), dev(VKB_R, "VKB R")];

        // SC lists all three in the saved order -> no clash, nothing to resort.
        let log = log_of(&[("Keychron", KEYCHRON_K2HE), ("VKB L", VKB_L), ("VKB R", VKB_R)]);
        let report = analyze_clash(&profile, &all, Ok(&log));
        assert!(!report.has_clash);
        assert!(report.missing.is_empty());
        assert!(report.connected.iter().all(|s| !s.clash));
        assert!(report.resort.is_empty());
        assert!(report.resort_commands.is_empty());

        // Keychron gone from SC's list: the sticks behind it shift down a slot.
        let log = log_of(&[("VKB L", VKB_L), ("VKB R", VKB_R)]);
        let report = analyze_clash(&profile, &all, Ok(&log));
        assert!(report.has_clash);

        let l = &report.connected[0];
        assert_eq!(l.effective_instance, 1); // SC now calls VKB L js1
        assert_eq!(l.stored_instance, Some(2)); // its bindings live on js2
        assert!(l.clash);

        let r = &report.connected[1];
        assert_eq!(r.effective_instance, 2);
        assert_eq!(r.stored_instance, Some(3));
        assert!(r.clash);

        // The missing Keychron slot is reported.
        assert_eq!(report.missing.len(), 1);
        assert_eq!(report.missing[0].stored_instance, 1);
        assert_eq!(report.missing[0].name, "Keychron K2 HE");

        // Resort: L's binds 2->1, R's 3->2, the dangling Keychron binds 1->3.
        let moves: Vec<(u32, u32, Option<&str>)> =
            report.resort.iter().map(|m| (m.from, m.to, m.name.as_deref())).collect();
        assert_eq!(
            moves,
            vec![(1, 3, Some("Keychron K2 HE")), (2, 1, Some("VKB L")), (3, 2, Some("VKB R"))]
        );
        // One 3-cycle 1->3->2->1: two swaps anchored on slot 1.
        assert_eq!(
            report.resort_commands,
            vec!["pp_resortdevices joystick 1 3", "pp_resortdevices joystick 1 2"]
        );
    }

    /// SC's Linux enumeration exactly as logged on the real setup: R = js1,
    /// L = js2, the Keychron K2 HE absent (hidden by Wine).
    fn linux_log() -> LogEnumeration {
        crate::gamelog::parse(concat!(
            "<2026-09-08T21:06:00.762Z> - Connected joystick0:  VKBsim Gladiator EVO  R    {0200231D-0000-0000-0000-504944564944}\n",
            "<2026-09-08T21:06:00.789Z> - Connected joystick1:  VKBsim Gladiator EVO  L    {0201231D-0000-0000-0000-504944564944}\n",
        ))
        .unwrap()
    }

    #[test]
    fn game_log_is_the_order_source() {
        // Saved options match the log -> no clash, on any platform.
        let xml = r#"<ActionMaps>
          <options type="joystick" instance="1" Product=" VKBsim Gladiator EVO  R    {0200231D-0000-0000-0000-504944564944}"/>
          <options type="joystick" instance="2" Product=" VKBsim Gladiator EVO  L    {0201231D-0000-0000-0000-504944564944}"/>
        </ActionMaps>"#;
        let profile = parse_actionmaps(xml).unwrap();
        // SDL sees the K2 HE too (Linux), in an order that is NOT SC's.
        let sdl = [dev(KEYCHRON_K2HE, "K2 HE"), dev(VKB_R, "VKB R"), dev(VKB_L, "VKB L")];
        let log = linux_log();
        let report = analyze_clash(&profile, &sdl, Ok(&log));

        assert_eq!(report.log_timestamp.as_deref(), Some("2026-09-08T21:06:00.789Z"));
        assert_eq!(report.log_error, None);
        assert!(!report.has_clash);
        assert!(report.missing.is_empty());
        // Order comes from the log, not from SDL: R is js1, L is js2.
        let order: Vec<(u32, Option<&str>)> = report
            .connected
            .iter()
            .map(|s| (s.effective_instance, s.sc_product_guid.as_deref()))
            .collect();
        assert_eq!(order, vec![(1, Some(VKB_R)), (2, Some(VKB_L))]);
        assert!(report.connected.iter().all(|s| s.connected_now));
        // The K2 HE is SDL-only: unseen by SC, but not a clash.
        assert_eq!(report.unseen.len(), 1);
        assert_eq!(report.unseen[0].sc_product_guid.as_deref(), Some(KEYCHRON_K2HE));
    }

    #[test]
    fn game_log_detects_shift_against_stale_options() {
        // Options written with the K2 HE at js1 (e.g. imported from Windows);
        // SC on Linux never sees it, so R/L sit one slot lower than their binds.
        let xml = r#"<ActionMaps>
          <options type="joystick" instance="1" Product="Keychron K2 HE  {0E213434-0000-0000-0000-504944564944}"/>
          <options type="joystick" instance="2" Product=" VKBsim Gladiator EVO  R    {0200231D-0000-0000-0000-504944564944}"/>
          <options type="joystick" instance="3" Product=" VKBsim Gladiator EVO  L    {0201231D-0000-0000-0000-504944564944}"/>
        </ActionMaps>"#;
        let profile = parse_actionmaps(xml).unwrap();
        let sdl = [dev(KEYCHRON_K2HE, "K2 HE"), dev(VKB_R, "VKB R"), dev(VKB_L, "VKB L")];
        let log = linux_log();
        let report = analyze_clash(&profile, &sdl, Ok(&log));

        assert!(report.has_clash);
        assert!(report.connected.iter().all(|s| s.clash));
        assert_eq!(report.connected[0].stored_instance, Some(2)); // R: binds on js2, SC says js1
        assert_eq!(report.connected[1].stored_instance, Some(3)); // L: binds on js3, SC says js2
        // The K2 HE slot is missing from SC's list -- the cause of the shift --
        // even though SDL sees the device (it is also reported as unseen).
        assert_eq!(report.missing.len(), 1);
        assert_eq!(report.missing[0].stored_instance, 1);
        assert_eq!(report.unseen.len(), 1);
    }

    #[test]
    fn game_log_flags_device_unplugged_since_sc_start() {
        let profile = parse_actionmaps("<ActionMaps/>").unwrap();
        // SC saw R and L at start; L has since been unplugged.
        let sdl = [dev(VKB_R, "VKB R")];
        let log = linux_log();
        let report = analyze_clash(&profile, &sdl, Ok(&log));
        assert_eq!(report.connected.len(), 2); // SC's view, not SDL's
        assert!(report.connected[0].connected_now);
        assert!(!report.connected[1].connected_now);
        assert!(report.unseen.is_empty());
    }

    #[test]
    fn a_missing_device_on_the_highest_slot_is_no_clash() {
        // R on js1 as SC says; L saved on js2 but unplugged: nothing shifts.
        let xml = r#"<ActionMaps>
          <options type="joystick" instance="1" Product=" VKB R {0200231D-0000-0000-0000-504944564944}"/>
          <options type="joystick" instance="2" Product=" VKB L {0201231D-0000-0000-0000-504944564944}"/>
        </ActionMaps>"#;
        let profile = parse_actionmaps(xml).unwrap();
        let log = LogEnumeration { joysticks: linux_log().joysticks.into_iter().take(1).collect(), gamepads: Vec::new(), timestamp: None };
        let report = analyze_clash(&profile, &[dev(VKB_R, "R")], Ok(&log));
        assert_eq!(report.missing.len(), 1);
        assert!(report.resort.is_empty());
        assert!(!report.has_clash);
    }

    #[test]
    fn a_pad_only_game_log_is_no_joystick_order() {
        let xml = r#"<ActionMaps>
          <options type="joystick" instance="1" Product=" VKB R {0200231D-0000-0000-0000-504944564944}"/>
        </ActionMaps>"#;
        let profile = parse_actionmaps(xml).unwrap();
        let log = LogEnumeration { joysticks: Vec::new(), gamepads: vec!["Gamepad".into()], timestamp: None };
        let report = analyze_clash(&profile, &[dev(VKB_R, "R")], Ok(&log));
        assert_eq!(report.log_error, Some(GameLogError::NoJoystickLines));
        assert!(report.missing.is_empty());
        assert!(!report.has_clash);
        assert!(report.gamepad_seen);
        assert_eq!(report.unseen.len(), 1);
    }

    #[test]
    fn missing_game_log_yields_no_order_at_all() {
        let xml = r#"<ActionMaps>
          <options type="joystick" instance="1" Product=" VKB L {0201231D-0000-0000-0000-504944564944}"/>
        </ActionMaps>"#;
        let profile = parse_actionmaps(xml).unwrap();
        let err = GameLogError::NoDeviceLines { path: "x/Game.log".into() };
        let report = analyze_clash(&profile, &[dev(VKB_R, "R")], Err(err.clone()));
        // Nothing is derived from SDL's order: no slots, no missing, no clash.
        assert_eq!(report, ClashReport { log_error: Some(err), ..Default::default() });
    }

    #[test]
    fn excluded_devices_are_dropped_by_guid_case_insensitively() {
        let devs = [dev(KEYCHRON_K2HE, "K2 HE"), dev(VKB_R, "R")];
        let excluded = vec![KEYCHRON_K2HE.to_ascii_lowercase()];
        let kept = without_excluded(&devs, &excluded);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].sc_product_guid.as_deref(), Some(VKB_R));
        // Nothing excluded -> untouched.
        assert_eq!(without_excluded(&devs, &[]).len(), 2);
    }

    #[test]
    fn unknown_device_is_not_a_clash() {
        // A listed device absent from the saved profile carries no stored jsN,
        // so it is flagged "not saved" rather than a hard clash. The saved stick
        // stays at its slot, so nothing shifts.
        let xml = r#"<ActionMaps>
          <options type="joystick" instance="1" Product=" VKB L {0201231D-0000-0000-0000-504944564944}"/>
        </ActionMaps>"#;
        let profile = parse_actionmaps(xml).unwrap();
        let devs = [dev(VKB_L, "VKB L"), dev("{DEAD0000-0000-0000-0000-504944564944}", "New Stick")];
        let log = log_of(&[("VKB L", VKB_L), ("New Stick", "{DEAD0000-0000-0000-0000-504944564944}")]);
        let report = analyze_clash(&profile, &devs, Ok(&log));
        assert!(!report.has_clash);
        assert_eq!(report.connected[1].stored_instance, None);
        assert!(!report.connected[1].clash);
        assert!(report.resort.is_empty());
    }

    /// Slot status shorthand for resort tests: (effective, stored, name).
    fn slot(effective: u32, stored: Option<u32>, name: &str) -> SlotStatus {
        SlotStatus {
            effective_instance: effective,
            stored_instance: stored,
            sc_product_guid: None,
            name: Some(name.into()),
            clash: stored.is_some_and(|s| s != effective),
            connected_now: true,
        }
    }

    fn missing(stored: u32, name: &str) -> MissingSlot {
        MissingSlot { stored_instance: stored, name: name.into(), sc_product_guid: None }
    }

    #[test]
    fn resort_plain_swap() {
        // Two sticks swapped: one command, both directions in one go.
        let moves = plan_resort(&[slot(1, Some(2), "L"), slot(2, Some(1), "R")], &[]);
        assert_eq!(
            moves,
            vec![
                ResortMove { from: 1, to: 2, name: Some("R".into()) },
                ResortMove { from: 2, to: 1, name: Some("L".into()) },
            ]
        );
        assert_eq!(resort_commands(&moves), vec!["pp_resortdevices joystick 1 2"]);
    }

    #[test]
    fn resort_keeps_dangling_bindings_on_leftover_slots() {
        // Windows profile js1=L js2=K2HE js3=Link js4=R, but SC (under Wine)
        // lists only R=js1, L=js2. L and R go home; the two hidden devices'
        // bindings are parked on the slots that fall free, never dropped.
        let connected = [slot(1, Some(4), "R"), slot(2, Some(1), "L")];
        let missing = [missing(2, "K2 HE"), missing(3, "Link")];
        let moves = plan_resort(&connected, &missing);
        let flat: Vec<(u32, u32, Option<&str>)> = moves.iter().map(|m| (m.from, m.to, m.name.as_deref())).collect();
        assert_eq!(
            flat,
            vec![(1, 2, Some("L")), (2, 3, Some("K2 HE")), (3, 4, Some("Link")), (4, 1, Some("R"))]
        );
        // One 4-cycle 1->2->3->4->1: three swaps anchored on slot 1.
        assert_eq!(
            resort_commands(&moves),
            vec![
                "pp_resortdevices joystick 1 2",
                "pp_resortdevices joystick 1 3",
                "pp_resortdevices joystick 1 4",
            ]
        );
    }

    #[test]
    fn resort_unsaved_device_takes_a_free_slot() {
        // A new stick sits at js1, pushing the saved one (js1) to js2. The
        // saved bindings follow their device; the new stick inherits the
        // (unrelated) js2 slot, which had no saved device.
        let moves = plan_resort(&[slot(1, None, "New"), slot(2, Some(1), "Old")], &[]);
        let flat: Vec<(u32, u32, Option<&str>)> = moves.iter().map(|m| (m.from, m.to, m.name.as_deref())).collect();
        assert_eq!(flat, vec![(1, 2, Some("Old")), (2, 1, None)]);
        assert_eq!(resort_commands(&moves), vec!["pp_resortdevices joystick 1 2"]);
    }

    #[test]
    fn resort_is_a_bijection_with_two_independent_cycles() {
        let connected = [slot(1, Some(2), "A"), slot(2, Some(1), "B"), slot(3, Some(4), "C"), slot(4, Some(3), "D")];
        let moves = plan_resort(&connected, &[]);
        let mut froms: Vec<u32> = moves.iter().map(|m| m.from).collect();
        let mut tos: Vec<u32> = moves.iter().map(|m| m.to).collect();
        froms.sort();
        tos.sort();
        assert_eq!(froms, tos);
        assert_eq!(
            resort_commands(&moves),
            vec!["pp_resortdevices joystick 1 2", "pp_resortdevices joystick 3 4"]
        );
    }
    #[test]
    fn builds_tokens() {
        assert_eq!(button_token(2, 8), "js2_button9"); // SDL 8 -> SC button9
        assert_eq!(hat_token(1, 0, "up").as_deref(), Some("js1_hat1_up"));
        assert_eq!(hat_token(1, 0, "rightup"), None); // diagonal: no cardinal token
    }

    #[test]
    fn resolves_bindings_across_contexts() {
        let profile_xml = r#"<profile>
          <actionmap name="spaceship_general" UILabel="@m1">
            <action name="v_toggle_flight_mode" UILabel="@a1"/>
            <action name="v_fire" UILabel="@a2"/>
          </actionmap>
          <actionmap name="ui" UILabel="@m2">
            <action name="ready"/>
          </actionmap>
        </profile>"#;
        let mut loc = HashMap::new();
        loc.insert("a1".to_string(), "Toggle Flight Mode".to_string());
        loc.insert("a2".to_string(), "Fire".to_string());
        let maps = parse_default_profile(profile_xml, &loc).unwrap();

        let user_xml = r#"<ActionMaps>
          <options type="joystick" instance="2" Product=" R {0200231D-0000-0000-0000-504944564944}"/>
          <actionmap name="spaceship_general">
            <action name="v_toggle_flight_mode"><rebind input="js1_button6"/></action>
            <action name="v_fire"><rebind input="js2_button1"/></action>
          </actionmap>
          <actionmap name="ui">
            <action name="ready"><rebind input="js2_button1"/></action>
          </actionmap>
        </ActionMaps>"#;
        let profile = parse_actionmaps(user_xml).unwrap();

        let index = BindingIndex::build(&maps, &profile);

        let flight = index.resolve("js1_button6");
        assert_eq!(flight.len(), 1);
        assert_eq!(flight[0].label.as_deref(), Some("Toggle Flight Mode"));

        // same physical button, two actions in different contexts
        let shared = index.resolve("js2_button1");
        assert_eq!(shared.len(), 2);

        assert!(index.resolve("js1_button99").is_empty());

        assert_eq!(
            instance_for_guid(&profile, "{0200231D-0000-0000-0000-504944564944}"),
            Some(2),
        );

        let resolved = resolve_bindings(&maps, &profile);
        // js1_button6 plus the two js2_button1 rebinds.
        assert_eq!(resolved.len(), 3);

        let flight = resolved.iter().find(|b| b.token == "js1_button6").unwrap();
        assert_eq!(flight.label.as_deref(), Some("Toggle Flight Mode"));
        assert_eq!(flight.device, None); // no instance 1 in <options> -> unknown device

        let fire = resolved.iter().find(|b| b.action == "v_fire").unwrap();
        assert_eq!(fire.token, "js2_button1");
        assert_eq!(fire.device.as_deref(), Some("R"));
    }

    #[test]
    fn shipped_defaults_apply_on_js1_unless_the_user_touched_the_action() {
        let profile_xml = r#"<profile>
          <actionmap name="m" UILabel="@m">
            <action name="untouched" joystick="button1" UILabel="@a"/>
            <action name="rebound" joystick="x"/>
            <action name="unbound" joystick="y"/>
            <action name="kb_only" joystick="button2"/>
            <action name="no_default"/>
          </actionmap>
        </profile>"#;
        let mut loc = HashMap::new();
        loc.insert("a".to_string(), "Untouched".to_string());
        let maps = parse_default_profile(profile_xml, &loc).unwrap();

        let user_xml = r#"<ActionMaps>
          <options type="joystick" instance="1" Product=" L {0201231D-0000-0000-0000-504944564944}"/>
          <actionmap name="m">
            <action name="rebound"><rebind input="js2_rotz"/></action>
            <action name="unbound"><rebind input="js1_ "/></action>
            <action name="kb_only"><rebind input="kb1_k"/></action>
          </actionmap>
        </ActionMaps>"#;
        let profile = parse_actionmaps(user_xml).unwrap();

        let resolved = resolve_bindings(&maps, &profile);
        let find = |action: &str| resolved.iter().filter(|b| b.action == action).collect::<Vec<_>>();

        // Untouched: the default shows up on js1, on the js1 device, tagged.
        let u = find("untouched");
        assert_eq!(u.len(), 1);
        assert_eq!(u[0].token, "js1_button1");
        assert!(u[0].is_default);
        assert_eq!(u[0].device.as_deref(), Some("L"));
        assert_eq!(u[0].label.as_deref(), Some("Untouched"));

        // Rebound: only the user's binding, the default is replaced.
        let r = find("rebound");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].token, "js2_rotz");
        assert!(!r[0].is_default);

        // Explicitly unbound (blank js rebind): nothing at all.
        assert!(find("unbound").is_empty());

        // A keyboard-only rebind leaves the joystick default in place; the
        // keyboard binding itself is resolved alongside it.
        let k = find("kb_only");
        assert_eq!(k.len(), 2);
        let js = k.iter().find(|b| b.device_kind == DeviceKind::Joystick).unwrap();
        assert_eq!(js.token, "js1_button2");
        assert!(js.is_default);
        let kb = k.iter().find(|b| b.device_kind == DeviceKind::Keyboard).unwrap();
        assert_eq!(kb.token, "kb1_k");
        assert_eq!((kb.instance, kb.device.as_deref(), &kb.device_guid), (1, Some("Keyboard/Mouse"), &None));
        assert!(!kb.is_default);

        assert!(find("no_default").is_empty());

        // The live index sees the same defaults.
        let index = BindingIndex::build(&maps, &profile);
        let hit = index.resolve("js1_button1");
        assert_eq!(hit.len(), 1);
        assert!(hit[0].is_default);
        assert!(index.resolve("js1_x").is_empty()); // replaced by the js2_rotz rebind
        assert!(index.resolve("js1_y").is_empty()); // unbound
    }

    #[test]
    fn mouse_defaults_are_keyboard_bindings() {
        // `mouse=` is a second keyboard-kind default (SC binds it as
        // `kb1_mouse1`); a keyboard rebind replaces both, a joystick one neither.
        let profile_xml = r#"<profile>
          <actionmap name="m" UILabel="@m">
            <action name="both" keyboard="w" mouse="mouse1" joystick="button1"/>
            <action name="same" keyboard="mwheel_up" mouse="mwheel_up"/>
            <action name="kb_rebound" keyboard="w" mouse="mouse2"/>
            <action name="js_rebound" keyboard="e" mouse="mouse3" joystick="button2"/>
          </actionmap>
        </profile>"#;
        let maps = parse_default_profile(profile_xml, &HashMap::new()).unwrap();

        let user_xml = r#"<ActionMaps>
          <actionmap name="m">
            <action name="kb_rebound"><rebind input="kb1_mwheel_down"/></action>
            <action name="js_rebound"><rebind input="js1_button9"/></action>
          </actionmap>
        </ActionMaps>"#;
        let profile = parse_actionmaps(user_xml).unwrap();

        let resolved = resolve_bindings(&maps, &profile);
        let tokens = |action: &str| {
            let mut t: Vec<&str> = resolved.iter().filter(|b| b.action == action).map(|b| b.token.as_str()).collect();
            t.sort();
            t
        };

        assert_eq!(tokens("both"), vec!["js1_button1", "kb1_mouse1", "kb1_w"]);
        assert!(resolved.iter().filter(|b| b.action == "both").all(|b| b.is_default));
        assert!(resolved
            .iter()
            .filter(|b| b.action == "both" && b.token.starts_with("kb1_"))
            .all(|b| b.device_kind == DeviceKind::Keyboard && b.device.as_deref() == Some("Keyboard/Mouse")));
        // The same token in both slots is one binding, not two.
        assert_eq!(tokens("same"), vec!["kb1_mwheel_up"]);
        assert_eq!(tokens("kb_rebound"), vec!["kb1_mwheel_down"]);
        assert_eq!(tokens("js_rebound"), vec!["js1_button9", "kb1_e", "kb1_mouse3"]);

        let index = BindingIndex::build(&maps, &profile);
        assert_eq!(index.resolve("kb1_mouse1").len(), 1);
        assert!(index.resolve("kb1_mouse2").is_empty());
        assert_eq!(index.resolve("kb1_mouse3").len(), 1);
    }

    #[test]
    fn defaults_and_touched_state_are_per_device_kind() {
        // Every action ships all three defaults; the user rebinds exactly one
        // device per action. Only that device's default must give way.
        let profile_xml = r#"<profile>
          <actionmap name="m" UILabel="@m">
            <action name="js_rebound" joystick="button1" keyboard="a" gamepad="x"/>
            <action name="kb_rebound" joystick="button2" keyboard="b" gamepad="y"/>
            <action name="gp_rebound" joystick="button3" keyboard="c" gamepad="back"/>
            <action name="gp_unbound" joystick="button4" keyboard="d" gamepad="start"/>
            <action name="combo_default" keyboard="lalt+x">
              <gamepad activationMode="tap" input="shoulderl+thumbl_left"/>
            </action>
          </actionmap>
        </profile>"#;
        let maps = parse_default_profile(profile_xml, &HashMap::new()).unwrap();

        let user_xml = r#"<ActionMaps>
          <actionmap name="m">
            <action name="js_rebound"><rebind input="js2_button9"/></action>
            <action name="kb_rebound"><rebind input="kb1_lctrl+e"/></action>
            <action name="gp_rebound"><rebind input="gp1_shoulderl+y"/></action>
            <action name="gp_unbound"><rebind input="gp1_ "/></action>
          </actionmap>
        </ActionMaps>"#;
        let profile = parse_actionmaps(user_xml).unwrap();

        let resolved = resolve_bindings(&maps, &profile);
        let tokens = |action: &str| {
            let mut t: Vec<&str> = resolved.iter().filter(|b| b.action == action).map(|b| b.token.as_str()).collect();
            t.sort_unstable();
            t
        };

        // The rebound device loses its default, the other two keep theirs.
        assert_eq!(tokens("js_rebound"), vec!["gp1_x", "js2_button9", "kb1_a"]);
        assert_eq!(tokens("kb_rebound"), vec!["gp1_y", "js1_button2", "kb1_lctrl+e"]);
        assert_eq!(tokens("gp_rebound"), vec!["gp1_shoulderl+y", "js1_button3", "kb1_c"]);
        // A blank gamepad rebind unbinds only the gamepad default.
        assert_eq!(tokens("gp_unbound"), vec!["js1_button4", "kb1_d"]);
        // Defaults with modifiers keep them inside the token.
        assert_eq!(tokens("combo_default"), vec!["gp1_shoulderl+thumbl_left", "kb1_lalt+x"]);

        // The live index agrees, and tags the kind on every hit.
        let index = BindingIndex::build(&maps, &profile);
        let kb = index.resolve("kb1_lctrl+e");
        assert_eq!(kb.len(), 1);
        assert_eq!(kb[0].device_kind, DeviceKind::Keyboard);
        assert!(!kb[0].is_default);

        let gp = index.resolve("gp1_shoulderl+thumbl_left");
        assert_eq!(gp.len(), 1);
        assert_eq!(gp[0].device_kind, DeviceKind::Gamepad);
        assert!(gp[0].is_default);

        assert!(index.resolve("gp1_start").is_empty()); // blank rebind
        assert!(index.resolve("js1_button1").is_empty()); // moved to js2
        assert_eq!(index.resolve("js2_button9")[0].device_kind, DeviceKind::Joystick);
    }

    #[test]
    fn clash_analysis_ignores_keyboard_and_gamepad_devices() {
        let xml = r#"<ActionMaps>
          <options type="joystick" instance="1" Product=" VKB L {0201231D-0000-0000-0000-504944564944}"/>
        </ActionMaps>"#;
        let profile = parse_actionmaps(xml).unwrap();
        let mut pad = dev("{028E045E-0000-0000-0000-504944564944}", "Gamepad");
        pad.kind = DeviceKind::Gamepad;
        let mut keyboard = DeviceInfo { kind: DeviceKind::Keyboard, ..DeviceInfo::default() };
        keyboard.sc_name = Some("Keyboard".into());
        let devs = [dev(VKB_L, "VKB L"), pad, keyboard];

        let log = crate::gamelog::parse(concat!(
            "<t> - Connected joystick0:  VKB L {0201231D-0000-0000-0000-504944564944}\n",
            "<t> - Connected xinput0: Gamepad\n",
        ))
        .unwrap();
        let report = analyze_clash(&profile, &devs, Ok(&log));

        assert!(report.gamepad_seen);
        assert!(!report.has_clash);
        assert_eq!(report.connected.len(), 1); // only the stick has a slot
        assert!(report.unseen.is_empty()); // pad and keyboard are not "unseen"

        // No xinput line -> SC did not see a pad at its last start.
        let log = log_of(&[("VKB L", VKB_L)]);
        assert!(!analyze_clash(&profile, &devs, Ok(&log)).gamepad_seen);
    }
}
