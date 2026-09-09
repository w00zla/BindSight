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
use crate::scdata::{is_joystick_rebind, parse_js_binding, ActionMap, UserProfile};

/// An action a token is bound to, with the context (actionmap) it applies in.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BoundAction {
    pub actionmap: String,
    pub action: String,
    pub label: Option<String>,
    /// Comes from `defaultProfile.xml`'s joystick default (always on `js1`),
    /// not from a user rebind.
    pub is_default: bool,
}

/// Instance SC applies `defaultProfile.xml`'s unnumbered joystick defaults to
/// (`joystick="button1"` means `js1_button1`) — confirmed by the user.
const DEFAULT_INSTANCE: u32 = 1;

/// Full SC token for an unnumbered joystick default from `defaultProfile.xml`.
pub fn default_token(token: &str) -> String {
    format!("js{DEFAULT_INSTANCE}_{token}")
}

/// Actions whose joystick binding the user touched in `actionmaps.xml` — any
/// rebind with a `js` input, including a blank one (`js1_ `) that unbinds the
/// shipped default. For those the default no longer applies. A keyboard/mouse/
/// gamepad-only rebind does not count: rebinds are per device.
fn joystick_touched(profile: &UserProfile) -> HashSet<(&str, &str)> {
    profile
        .rebinds
        .iter()
        .filter(|r| is_joystick_rebind(&r.input))
        .map(|r| (r.actionmap.as_str(), r.action.as_str()))
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
    pub fn build(maps: &[ActionMap], profile: &UserProfile) -> Self {
        // (actionmap, action) -> resolved label
        let mut label_of: HashMap<(&str, &str), Option<String>> = HashMap::new();
        for map in maps {
            for action in &map.actions {
                label_of.insert((map.name.as_str(), action.name.as_str()), action.label.clone());
            }
        }

        let mut by_token: HashMap<String, Vec<BoundAction>> = HashMap::new();
        for rebind in &profile.rebinds {
            let Some((instance, token)) = parse_js_binding(&rebind.input) else {
                continue;
            };
            let full_token = format!("js{instance}_{token}");
            let label = label_of
                .get(&(rebind.actionmap.as_str(), rebind.action.as_str()))
                .cloned()
                .flatten();
            by_token.entry(full_token).or_default().push(BoundAction {
                actionmap: rebind.actionmap.clone(),
                action: rebind.action.clone(),
                label,
                is_default: false,
            });
        }

        // Shipped defaults (always js1) for actions the user never touched.
        let touched = joystick_touched(profile);
        for map in maps {
            for action in &map.actions {
                let Some(token) = &action.joystick_default else { continue };
                if touched.contains(&(map.name.as_str(), action.name.as_str())) {
                    continue;
                }
                by_token.entry(default_token(token)).or_default().push(BoundAction {
                    actionmap: map.name.clone(),
                    action: action.name.clone(),
                    label: action.label.clone(),
                    is_default: true,
                });
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

/// One user joystick binding, resolved to a device and a label for display.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResolvedBinding {
    pub token: String,
    /// Device name recorded for this binding's `jsN` instance (from `<options>`).
    pub device: Option<String>,
    /// SC Product GUID of that instance, to match against connected devices.
    pub device_guid: Option<String>,
    pub actionmap: String,
    pub action: String,
    pub label: Option<String>,
    /// A shipped default from `defaultProfile.xml` (on `js1`), not a user rebind.
    pub is_default: bool,
}

/// Flatten the user's joystick rebinds into resolved bindings: each real (bound)
/// joystick input with its SC token, the device it sits on, and the action's
/// label. Non-joystick or unbound rebinds are skipped. Shipped `js1` defaults
/// are appended for every action the user never touched (see
/// [`joystick_touched`]).
pub fn resolve_bindings(maps: &[ActionMap], profile: &UserProfile) -> Vec<ResolvedBinding> {
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

    let mut out = Vec::new();
    for rebind in &profile.rebinds {
        let Some((instance, token)) = parse_js_binding(&rebind.input) else {
            continue;
        };
        let device = device_of.get(&instance);
        out.push(ResolvedBinding {
            token: format!("js{instance}_{token}"),
            device: device.map(|d| d.0.to_string()),
            device_guid: device.and_then(|d| d.1.map(String::from)),
            actionmap: rebind.actionmap.clone(),
            action: rebind.action.clone(),
            label: label_of
                .get(&(rebind.actionmap.as_str(), rebind.action.as_str()))
                .cloned()
                .flatten(),
            is_default: false,
        });
    }

    let touched = joystick_touched(profile);
    let default_device = device_of.get(&DEFAULT_INSTANCE);
    for map in maps {
        for action in &map.actions {
            let Some(token) = &action.joystick_default else { continue };
            if touched.contains(&(map.name.as_str(), action.name.as_str())) {
                continue;
            }
            out.push(ResolvedBinding {
                token: default_token(token),
                device: default_device.map(|d| d.0.to_string()),
                device_guid: default_device.and_then(|d| d.1.map(String::from)),
                actionmap: map.name.clone(),
                action: action.name.clone(),
                label: action.label.clone(),
                is_default: true,
            });
        }
    }
    out
}

/// One connected joystick's SC instance status: the `jsN` SC would assign it
/// right now (its 1-based rank in SC's enumeration order, derived per platform —
/// see [`ClashReport::order_verified`]) versus the `jsN` recorded for its GUID
/// in the saved `<options>` block.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SlotStatus {
    /// `jsN` SC assigns now: 1-based rank in the derived SC order. Only a best
    /// guess when the report's `order_verified` is false.
    pub effective_instance: u32,
    /// `jsN` recorded for this device's GUID in `<options>`, or `None` if the
    /// device is not in the saved profile at all.
    pub stored_instance: Option<u32>,
    pub sc_product_guid: Option<String>,
    pub name: Option<String>,
    /// The device is in the saved profile but under a different `jsN` than it now
    /// gets — every binding on its slot lands on the wrong stick.
    pub clash: bool,
    /// Whether SDL sees the device right now. With a `Game.log` source a slot
    /// can be in SC's last-start list yet unplugged since — SC will renumber on
    /// its next start.
    pub connected_now: bool,
}

/// A saved `<options>` joystick slot whose device is not in SC's device list
/// (not seen at the last game start, or not connected when deriving from SDL).
/// Its bindings dangle, and every device enumerated after it shifts down a slot.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MissingSlot {
    pub stored_instance: u32,
    pub name: String,
    pub sc_product_guid: Option<String>,
}

/// Where SC's device order came from.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OrderSource {
    /// SC's own enumeration from `Game.log` (last game start); `timestamp` is
    /// the raw log time of that enumeration.
    GameLog { timestamp: Option<String> },
    /// Derived from SDL's order by a per-platform rule — the fallback when no
    /// usable `Game.log` exists; `log_error` says why (`None` only when no
    /// profile is loaded and the log was never consulted).
    SdlDerived { log_error: Option<GameLogError> },
}

impl Default for OrderSource {
    fn default() -> Self {
        Self::SdlDerived { log_error: None }
    }
}

/// A device SDL sees that SC did not list at its last start: either hidden from
/// SC by Wine, or plugged in after SC started. The log alone can't tell which.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UnseenDevice {
    pub name: Option<String>,
    pub sc_product_guid: Option<String>,
}

/// Result of comparing SC's saved instance→device map against SC's actual
/// device order, to surface the SC "device order" binding-switch bug.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ClashReport {
    /// Joysticks in SC order (from `source`), each with effective vs stored `jsN`.
    pub connected: Vec<SlotStatus>,
    /// Saved occupied slots not in SC's device list (they cause the shift).
    pub missing: Vec<MissingSlot>,
    /// SDL-visible devices absent from SC's list. Only filled for a `Game.log`
    /// source; informational, never a clash by itself.
    pub unseen: Vec<UnseenDevice>,
    pub source: OrderSource,
    /// Whether the SC order is trustworthy: always for `Game.log`; for the SDL
    /// fallback only where the per-platform rule is verified. When false,
    /// `effective_instance` is a guess and no rank clash is asserted — only
    /// `missing` contributes to `has_clash`.
    pub order_verified: bool,
    pub has_clash: bool,
}

/// SC's device order derived from SDL's, plus whether that derivation is
/// verified on this platform. This is only the fallback when no usable
/// `Game.log` exists — SDL's enumeration order is NOT SC's, measured on the
/// real setup (`temp/joyenumtest`):
///
/// - **Windows**: SC enumerates in exactly the reverse of SDL's order (one
///   sample, four devices, exact mirror).
/// - **Linux/Wine**: no usable relation. SC's order stayed put across a reboot
///   that reshuffled the evdev (SDL) order, and Wine hides devices SC never
///   sees (e.g. a Keychron K2 HE keyboard). The identity mapping here is only a
///   placeholder so the slots exist; the order is unknown, flagged unverified,
///   and no rank clash is ever asserted from it.
fn sc_order_from_sdl(devices: &[DeviceInfo]) -> (Vec<&DeviceInfo>, bool) {
    if cfg!(target_os = "windows") {
        (devices.iter().rev().collect(), true)
    } else {
        (devices.iter().collect(), false)
    }
}

/// Case-insensitive SC Product GUID equality; `None` never matches.
fn guid_eq(a: Option<&str>, b: Option<&str>) -> bool {
    matches!((a, b), (Some(a), Some(b)) if a.eq_ignore_ascii_case(b))
}

/// Drop devices the user declared invisible to SC ("SC doesn't see this
/// device", e.g. a keyboard Wine hides from the game). They then count as
/// unplugged for the analysis. Matched by SC Product GUID, case-insensitively.
pub fn without_ignored(devices: &[DeviceInfo], ignored: &[String]) -> Vec<DeviceInfo> {
    devices
        .iter()
        .filter(|d| !ignored.iter().any(|g| guid_eq(Some(g), d.sc_product_guid.as_deref())))
        .cloned()
        .collect()
}

/// A joystick slot in SC's order, from whichever source.
struct OrderedSlot {
    instance: u32,
    name: Option<String>,
    guid: Option<String>,
    connected_now: bool,
}

/// Compare the saved `<options>` order against SC's actual device order. SC
/// assigns `jsN` purely by enumeration position and ignores name/GUID
/// (validated by the user: `pp_resortdevices` is only ever needed because of
/// this). So a device whose SC rank differs from its saved `jsN` — or a saved
/// device SC does not list — means its bindings land on the wrong stick.
///
/// `log` (SC's own enumeration from `Game.log`) is the primary order source;
/// without it the order is derived from `devices` (SDL order) per platform.
pub fn analyze_clash(
    profile: &UserProfile,
    devices: &[DeviceInfo],
    log: Result<&LogEnumeration, GameLogError>,
) -> ClashReport {
    let log = match log {
        Ok(log) => log,
        Err(err) => {
            let (sc_order, order_verified) = sc_order_from_sdl(devices);
            let mut report = analyze_clash_with(profile, &sc_order, order_verified);
            report.source = OrderSource::SdlDerived { log_error: Some(err) };
            return report;
        }
    };

    let slots: Vec<OrderedSlot> = log
        .joysticks
        .iter()
        .map(|j| OrderedSlot {
            instance: j.instance,
            name: Some(j.product_name.clone()),
            guid: j.product_guid.clone(),
            connected_now: devices
                .iter()
                .any(|d| guid_eq(d.sc_product_guid.as_deref(), j.product_guid.as_deref())),
        })
        .collect();
    let mut report = build_report(profile, &slots, true);
    report.unseen = devices
        .iter()
        .filter(|d| !slots.iter().any(|s| guid_eq(s.guid.as_deref(), d.sc_product_guid.as_deref())))
        .map(|d| UnseenDevice { name: d.sc_name.clone(), sc_product_guid: d.sc_product_guid.clone() })
        .collect();
    report.source = OrderSource::GameLog { timestamp: log.timestamp.clone() };
    report
}

/// SDL-derived path: `sc_order` is already in SC's enumeration order, so rank
/// is the instance and every device is connected by definition.
fn analyze_clash_with(profile: &UserProfile, sc_order: &[&DeviceInfo], order_verified: bool) -> ClashReport {
    let slots: Vec<OrderedSlot> = sc_order
        .iter()
        .enumerate()
        .map(|(rank, d)| OrderedSlot {
            instance: rank as u32 + 1,
            name: d.sc_name.clone(),
            guid: d.sc_product_guid.clone(),
            connected_now: true,
        })
        .collect();
    build_report(profile, &slots, order_verified)
}

/// The source-independent core: rank clashes are only asserted when
/// `order_verified`; missing-slot detection is by GUID presence and holds
/// regardless.
fn build_report(profile: &UserProfile, slots: &[OrderedSlot], order_verified: bool) -> ClashReport {
    let mut connected = Vec::with_capacity(slots.len());
    let mut has_clash = false;

    for slot in slots {
        let stored_instance = slot.guid.as_deref().and_then(|g| instance_for_guid(profile, g));
        // A rank mismatch only means something if the order is real.
        let clash = order_verified && matches!(stored_instance, Some(i) if i != slot.instance);
        has_clash |= clash;
        connected.push(SlotStatus {
            effective_instance: slot.instance,
            stored_instance,
            sc_product_guid: slot.guid.clone(),
            name: slot.name.clone(),
            clash,
            connected_now: slot.connected_now,
        });
    }

    let mut missing = Vec::new();
    for js in &profile.joysticks {
        let Some(guid) = js.product_guid.as_deref() else {
            continue; // empty slot
        };
        let listed = slots.iter().any(|s| guid_eq(s.guid.as_deref(), Some(guid)));
        if !listed {
            missing.push(MissingSlot {
                stored_instance: js.instance,
                name: js.product_name.clone(),
                sc_product_guid: js.product_guid.clone(),
            });
        }
    }
    has_clash |= !missing.is_empty();

    ClashReport { connected, missing, order_verified, has_clash, ..Default::default() }
}

/// Find the SC `jsN` instance for a device by its SC Product GUID.
pub fn instance_for_guid(profile: &UserProfile, sc_product_guid: &str) -> Option<u32> {
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
    use crate::scdata::{parse_default_profile, parse_user_profile};

    /// A minimal connected device for clash tests; only GUID/name/order matter.
    fn dev(guid: &str, name: &str) -> DeviceInfo {
        DeviceInfo {
            index: 0,
            sc_name: Some(name.to_string()),
            sdl_name: name.to_string(),
            sdl_guid: String::new(),
            sc_product_guid: Some(guid.to_string()),
            num_buttons: 0,
            num_axes: 0,
            num_hats: 0,
        }
    }

    // Real product GUIDs from the measured setup (temp/joyenumtest).
    const KEYCHRON_K2HE: &str = "{0E213434-0000-0000-0000-504944564944}";
    const KEYCHRON_LINK: &str = "{D0303434-0000-0000-0000-504944564944}";
    const VKB_L: &str = "{0201231D-0000-0000-0000-504944564944}";
    const VKB_R: &str = "{0200231D-0000-0000-0000-504944564944}";

    #[test]
    fn detects_device_order_clash_when_order_is_verified() {
        // Saved order: js1 = Keychron, js2 = VKB L, js3 = VKB R.
        let xml = r#"<ActionMaps>
          <options type="joystick" instance="1" Product="Keychron K2 HE  {0E213434-0000-0000-0000-504944564944}"/>
          <options type="joystick" instance="2" Product=" VKB L {0201231D-0000-0000-0000-504944564944}"/>
          <options type="joystick" instance="3" Product=" VKB R {0200231D-0000-0000-0000-504944564944}"/>
        </ActionMaps>"#;
        let profile = parse_user_profile(xml).unwrap();

        // All three connected in the saved SC order -> no clash.
        let all = [dev(KEYCHRON_K2HE, "Keychron"), dev(VKB_L, "VKB L"), dev(VKB_R, "VKB R")];
        let report = analyze_clash_with(&profile, &all.iter().collect::<Vec<_>>(), true);
        assert!(report.order_verified);
        assert!(!report.has_clash);
        assert!(report.missing.is_empty());
        assert!(report.connected.iter().all(|s| !s.clash));

        // Keychron unplugged: the sticks behind it shift down a slot.
        let shifted = [dev(VKB_L, "VKB L"), dev(VKB_R, "VKB R")];
        let report = analyze_clash_with(&profile, &shifted.iter().collect::<Vec<_>>(), true);
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
    }

    #[test]
    fn windows_sc_order_is_reverse_of_sdl() {
        // Measured on Windows: SDL enumerates [R, Link, K2 HE, L] while SC's
        // fresh actionmaps.xml writes js1=L js2=K2HE js3=Link js4=R — the exact
        // reverse. Reversing SDL's order must therefore yield zero clashes.
        let xml = r#"<ActionMaps>
          <options type="joystick" instance="1" Product=" VKBsim Gladiator EVO  L    {0201231D-0000-0000-0000-504944564944}"/>
          <options type="joystick" instance="2" Product="Keychron K2 HE  {0E213434-0000-0000-0000-504944564944}"/>
          <options type="joystick" instance="3" Product="Keychron Link   {D0303434-0000-0000-0000-504944564944}"/>
          <options type="joystick" instance="4" Product=" VKBsim Gladiator EVO  R    {0200231D-0000-0000-0000-504944564944}"/>
        </ActionMaps>"#;
        let profile = parse_user_profile(xml).unwrap();

        let sdl = [dev(VKB_R, "VKB R"), dev(KEYCHRON_LINK, "Link"), dev(KEYCHRON_K2HE, "K2 HE"), dev(VKB_L, "VKB L")];
        let sc_order: Vec<&DeviceInfo> = sdl.iter().rev().collect();
        let report = analyze_clash_with(&profile, &sc_order, true);
        assert!(!report.has_clash);
        assert!(report.missing.is_empty());
        let ranks: Vec<(u32, Option<u32>)> =
            report.connected.iter().map(|s| (s.effective_instance, s.stored_instance)).collect();
        assert_eq!(ranks, vec![(1, Some(1)), (2, Some(2)), (3, Some(3)), (4, Some(4))]);

        // Sanity: the un-reversed SDL order would have flagged every stick.
        let wrong = analyze_clash_with(&profile, &sdl.iter().collect::<Vec<_>>(), true);
        assert!(wrong.connected.iter().all(|s| s.clash));
    }

    #[test]
    fn unverified_order_asserts_no_rank_clash_but_still_reports_missing() {
        // Linux/Wine: the derived order is a guess, so a rank mismatch must not
        // be reported as a clash — but a saved device that is absent is a fact.
        let xml = r#"<ActionMaps>
          <options type="joystick" instance="1" Product="Keychron K2 HE  {0E213434-0000-0000-0000-504944564944}"/>
          <options type="joystick" instance="2" Product=" VKB L {0201231D-0000-0000-0000-504944564944}"/>
        </ActionMaps>"#;
        let profile = parse_user_profile(xml).unwrap();

        // VKB L alone at rank 1 while saved as js2: a mismatch, but unverified.
        let only_l = [dev(VKB_L, "VKB L")];
        let report = analyze_clash_with(&profile, &only_l.iter().collect::<Vec<_>>(), false);
        assert!(!report.order_verified);
        assert!(!report.connected[0].clash);
        assert_eq!(report.connected[0].effective_instance, 1); // guess still exposed
        assert_eq!(report.connected[0].stored_instance, Some(2));
        // ...yet has_clash is true purely because the Keychron slot is missing.
        assert!(report.has_clash);
        assert_eq!(report.missing.len(), 1);
        assert_eq!(report.missing[0].stored_instance, 1);
    }

    /// A "Game.log not found" error, for the fallback tests.
    fn no_log() -> GameLogError {
        GameLogError::NotFound { path: "Game.log".into(), reason: "missing".into() }
    }

    #[test]
    fn public_entry_applies_platform_order_model() {
        let profile = parse_user_profile("<ActionMaps/>").unwrap();
        let devs = [dev(VKB_R, "R"), dev(VKB_L, "L")];
        let report = analyze_clash(&profile, &devs, Err(no_log()));
        assert!(matches!(report.source, OrderSource::SdlDerived { .. }));
        assert_eq!(report.order_verified, cfg!(target_os = "windows"));
        // On Windows rank 1 is the *last* SDL device; elsewhere the first.
        let first = report.connected[0].sc_product_guid.as_deref();
        if cfg!(target_os = "windows") {
            assert_eq!(first, Some(VKB_L));
        } else {
            assert_eq!(first, Some(VKB_R));
        }
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
    fn game_log_is_the_order_source_and_is_verified_everywhere() {
        // Saved options match the log -> no clash, on any platform.
        let xml = r#"<ActionMaps>
          <options type="joystick" instance="1" Product=" VKBsim Gladiator EVO  R    {0200231D-0000-0000-0000-504944564944}"/>
          <options type="joystick" instance="2" Product=" VKBsim Gladiator EVO  L    {0201231D-0000-0000-0000-504944564944}"/>
        </ActionMaps>"#;
        let profile = parse_user_profile(xml).unwrap();
        // SDL sees the K2 HE too (Linux), in an order that is NOT SC's.
        let sdl = [dev(KEYCHRON_K2HE, "K2 HE"), dev(VKB_R, "VKB R"), dev(VKB_L, "VKB L")];
        let log = linux_log();
        let report = analyze_clash(&profile, &sdl, Ok(&log));

        assert_eq!(
            report.source,
            OrderSource::GameLog { timestamp: Some("2026-09-08T21:06:00.789Z".into()) }
        );
        assert!(report.order_verified);
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
        let profile = parse_user_profile(xml).unwrap();
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
        let profile = parse_user_profile("<ActionMaps/>").unwrap();
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
    fn missing_game_log_falls_back_to_sdl_and_carries_the_reason() {
        let profile = parse_user_profile("<ActionMaps/>").unwrap();
        let err = GameLogError::NoDeviceLines { path: "x/Game.log".into() };
        let report = analyze_clash(&profile, &[dev(VKB_R, "R")], Err(err.clone()));
        assert_eq!(report.source, OrderSource::SdlDerived { log_error: Some(err) });
        assert_eq!(report.connected.len(), 1); // the SDL fallback still numbers devices
    }

    #[test]
    fn ignored_devices_are_dropped_by_guid_case_insensitively() {
        let devs = [dev(KEYCHRON_K2HE, "K2 HE"), dev(VKB_R, "R")];
        let ignored = vec![KEYCHRON_K2HE.to_ascii_lowercase()];
        let kept = without_ignored(&devs, &ignored);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].sc_product_guid.as_deref(), Some(VKB_R));
        // Nothing ignored -> untouched.
        assert_eq!(without_ignored(&devs, &[]).len(), 2);
    }

    #[test]
    fn unknown_device_is_not_a_clash() {
        // A connected device absent from the saved profile carries no stored jsN,
        // so it is flagged "not saved" rather than a hard clash. The saved stick
        // stays connected at its slot, so nothing shifts.
        let xml = r#"<ActionMaps>
          <options type="joystick" instance="1" Product=" VKB L {0201231D-0000-0000-0000-504944564944}"/>
        </ActionMaps>"#;
        let profile = parse_user_profile(xml).unwrap();
        let devs = [dev(VKB_L, "VKB L"), dev("{DEAD0000-0000-0000-0000-504944564944}", "New Stick")];
        let report = analyze_clash_with(&profile, &devs.iter().collect::<Vec<_>>(), true);
        assert!(!report.has_clash);
        assert_eq!(report.connected[1].stored_instance, None);
        assert!(!report.connected[1].clash);
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
        let profile = parse_user_profile(user_xml).unwrap();

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
        let profile = parse_user_profile(user_xml).unwrap();

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

        // A keyboard-only rebind leaves the joystick default in place.
        let k = find("kb_only");
        assert_eq!(k.len(), 1);
        assert_eq!(k[0].token, "js1_button2");
        assert!(k[0].is_default);

        assert!(find("no_default").is_empty());

        // The live index sees the same defaults.
        let index = BindingIndex::build(&maps, &profile);
        let hit = index.resolve("js1_button1");
        assert_eq!(hit.len(), 1);
        assert!(hit[0].is_default);
        assert!(index.resolve("js1_x").is_empty()); // replaced by the js2_rotz rebind
        assert!(index.resolve("js1_y").is_empty()); // unbound
    }
}
