//! Bridge between live joystick input and SC's bound actions: build the SC
//! input token for a physical input, and look up what that token is bound to.
//!
//! Flow at runtime: a live event carries the device's SDL GUID -> derive its SC
//! Product GUID -> [`instance_for_guid`] gives the `jsN` number -> [`button_token`]
//! / [`hat_token`] builds the token -> [`BindingIndex::resolve`] returns the
//! bound action(s).

use std::collections::HashMap;

use serde::Serialize;

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
