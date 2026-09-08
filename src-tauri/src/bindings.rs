//! Bridge between live joystick input and SC's bound actions: build the SC
//! input token for a physical input, and look up what that token is bound to.
//!
//! Flow at runtime: a live event carries the device's SDL GUID -> derive its SC
//! Product GUID -> [`instance_for_guid`] gives the `jsN` number -> [`button_token`]
//! / [`hat_token`] builds the token -> [`BindingIndex::resolve`] returns the
//! bound action(s).

use std::collections::HashMap;

use serde::Serialize;

use crate::input::DeviceInfo;
use crate::scdata::{parse_js_binding, ActionMap, UserProfile};

/// An action a token is bound to, with the context (actionmap) it applies in.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BoundAction {
    pub actionmap: String,
    pub action: String,
    pub label: Option<String>,
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
            });
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
}

/// Flatten the user's joystick rebinds into resolved bindings: each real (bound)
/// joystick input with its SC token, the device it sits on, and the action's
/// label. Non-joystick or unbound rebinds are skipped.
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
        });
    }
    out
}

/// One connected joystick's SC instance status: the `jsN` SC would assign it
/// right now (its 1-based rank in enumeration order) versus the `jsN` recorded
/// for its GUID in the saved `<options>` block.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SlotStatus {
    /// `jsN` SC assigns now: 1-based rank among connected joysticks (enum order).
    pub effective_instance: u32,
    /// `jsN` recorded for this device's GUID in `<options>`, or `None` if the
    /// device is not in the saved profile at all.
    pub stored_instance: Option<u32>,
    pub sc_product_guid: Option<String>,
    pub name: Option<String>,
    /// The device is in the saved profile but under a different `jsN` than it now
    /// gets — every binding on its slot lands on the wrong stick.
    pub clash: bool,
}

/// A saved `<options>` joystick slot whose device is not connected right now.
/// Its bindings dangle, and every device enumerated after it shifts down a slot.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MissingSlot {
    pub stored_instance: u32,
    pub name: String,
    pub sc_product_guid: Option<String>,
}

/// Result of comparing SC's saved instance→device map against the devices
/// connected now, to surface the SC "device order" binding-switch bug.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ClashReport {
    /// Connected joysticks in SC enum order, each with effective vs stored `jsN`.
    pub connected: Vec<SlotStatus>,
    /// Saved occupied slots whose device is not connected (they cause the shift).
    pub missing: Vec<MissingSlot>,
    pub has_clash: bool,
}

/// Compare the saved `<options>` order against the live device order. SC assigns
/// `jsN` purely by start-time enumeration order and ignores name/GUID (a known
/// SC bug, validated). So a connected device whose live rank differs from its
/// saved `jsN` — or a saved device that is now missing — means its bindings land
/// on the wrong stick. `devices` must be in SC/SDL enumeration order.
pub fn analyze_clash(profile: &UserProfile, devices: &[DeviceInfo]) -> ClashReport {
    let mut connected = Vec::with_capacity(devices.len());
    let mut has_clash = false;

    for (rank, dev) in devices.iter().enumerate() {
        let effective_instance = rank as u32 + 1;
        let stored_instance = dev
            .sc_product_guid
            .as_deref()
            .and_then(|g| instance_for_guid(profile, g));
        let clash = matches!(stored_instance, Some(i) if i != effective_instance);
        has_clash |= clash;
        connected.push(SlotStatus {
            effective_instance,
            stored_instance,
            sc_product_guid: dev.sc_product_guid.clone(),
            name: dev.sc_name.clone(),
            clash,
        });
    }

    let mut missing = Vec::new();
    for js in &profile.joysticks {
        let Some(guid) = js.product_guid.as_deref() else {
            continue; // empty slot
        };
        let connected_now = devices.iter().any(|d| {
            d.sc_product_guid
                .as_deref()
                .is_some_and(|g| g.eq_ignore_ascii_case(guid))
        });
        if !connected_now {
            missing.push(MissingSlot {
                stored_instance: js.instance,
                name: js.product_name.clone(),
                sc_product_guid: js.product_guid.clone(),
            });
        }
    }
    has_clash |= !missing.is_empty();

    ClashReport { connected, missing, has_clash }
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

    #[test]
    fn detects_device_order_clash() {
        // Saved order: js1 = Keychron, js2 = VKB L, js3 = VKB R.
        let xml = r#"<ActionMaps>
          <options type="joystick" instance="1" Product="Keychron K2 HE  {0E213434-0000-0000-0000-504944564944}"/>
          <options type="joystick" instance="2" Product=" VKB L {0201231D-0000-0000-0000-504944564944}"/>
          <options type="joystick" instance="3" Product=" VKB R {0200231D-0000-0000-0000-504944564944}"/>
        </ActionMaps>"#;
        let profile = parse_user_profile(xml).unwrap();

        let keychron = "{0E213434-0000-0000-0000-504944564944}";
        let vkb_l = "{0201231D-0000-0000-0000-504944564944}";
        let vkb_r = "{0200231D-0000-0000-0000-504944564944}";

        // All three connected in the saved order -> no clash.
        let all = [dev(keychron, "Keychron"), dev(vkb_l, "VKB L"), dev(vkb_r, "VKB R")];
        let report = analyze_clash(&profile, &all);
        assert!(!report.has_clash);
        assert!(report.missing.is_empty());
        assert!(report.connected.iter().all(|s| !s.clash));

        // Keychron unplugged: the sticks behind it shift down a slot.
        let shifted = [dev(vkb_l, "VKB L"), dev(vkb_r, "VKB R")];
        let report = analyze_clash(&profile, &shifted);
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
    fn unknown_device_is_not_a_clash() {
        // A connected device absent from the saved profile carries no stored jsN,
        // so it is flagged "not saved" rather than a hard clash. The saved stick
        // stays connected at its slot, so nothing shifts.
        let xml = r#"<ActionMaps>
          <options type="joystick" instance="1" Product=" VKB L {0201231D-0000-0000-0000-504944564944}"/>
        </ActionMaps>"#;
        let profile = parse_user_profile(xml).unwrap();
        let report = analyze_clash(
            &profile,
            &[
                dev("{0201231D-0000-0000-0000-504944564944}", "VKB L"),
                dev("{DEAD0000-0000-0000-0000-504944564944}", "New Stick"),
            ],
        );
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
}
