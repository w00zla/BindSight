//! Parsing of Star Citizen's generic (per-install, not per-user) config data:
//! the action master list from `defaultProfile.xml` and the human-readable
//! labels from `global.ini`. User rebinds (`actionmaps.xml`) are layered on
//! top in a later step.
//!
//! `defaultProfile.xml` groups bindable actions under `<actionmap>` elements:
//!
//! ```xml
//! <actionmap name="seat_general" UILabel="@ui_CGSeatGeneral">
//!   <action name="v_eject" UILabel="@ui_CIEject" UIDescription="@ui_CIEjectDesc" joystick=" " .../>
//! </actionmap>
//! ```
//!
//! The `UILabel`/`UIDescription` values are `@ui_*` keys resolved against
//! `global.ini`, whose entries are `ui_*=text` (the `@` is dropped). The
//! `UICategory` (SC's options-menu category) is deliberately not carried over.
//!
//! Defaults come in two forms, and SC uses both in the same file: as an
//! attribute on `<action>` (`keyboard="lalt+x" gamepad=" " joystick="button1"`)
//! and as a child element (`<gamepad activationMode="hold" input="triggerl_btn"/>`).
//! The child wins over the attribute of the same kind; a blank value means
//! "no default".

use std::collections::HashMap;

use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use serde::{Deserialize, Serialize};

/// The kind of input device a token, binding or connected device belongs to.
/// SC knows exactly one keyboard (`kb1`) and one gamepad (`gp1`); only
/// joysticks are numbered and take part in the `jsN` order/clash/resort logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    #[default]
    Joystick,
    Keyboard,
    Gamepad,
}

impl DeviceKind {
    /// The token prefix without the instance number (`js`, `kb`, `gp`).
    pub fn token_prefix(self) -> &'static str {
        match self {
            DeviceKind::Joystick => "js",
            DeviceKind::Keyboard => "kb",
            DeviceKind::Gamepad => "gp",
        }
    }
}

/// A single bindable action with its default bindings per device kind and the
/// resolved human-readable label/description.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Action {
    pub name: String,
    pub label: Option<String>,
    pub description: Option<String>,
    /// Default joystick binding from `defaultProfile.xml`, or `None` if unbound.
    pub joystick_default: Option<String>,
    /// Default keyboard binding (prefix-less, modifiers included: `lalt+x`).
    pub keyboard_default: Option<String>,
    /// Default gamepad binding (prefix-less, modifiers included: `shoulderl+y`).
    pub gamepad_default: Option<String>,
    /// Default mouse binding (`mouse1`, `mwheel_up`, `maxis_x`). SC binds the
    /// mouse under the keyboard prefix (`kb1_mouse1`), so this is a second
    /// keyboard-kind default, not a device kind of its own.
    #[serde(default)]
    pub mouse_default: Option<String>,
}

/// A group of actions (SC's `<actionmap>`), with its resolved label.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionMap {
    pub name: String,
    pub label: Option<String>,
    pub actions: Vec<Action>,
}

/// Parse `global.ini` (`key=value`, UTF-8 BOM, CRLF) into a lookup table.
/// Values may themselves contain `=`, so only the first `=` splits. Keys are
/// lowercased because SC references them with inconsistent casing (e.g.
/// `@ui_CIboost` vs the `ui_CIBoost` entry) — see [`resolve`].
pub fn parse_localization(text: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in text.lines() {
        // The BOM sits at the very start of the file, i.e. the first line.
        let line = line.strip_prefix('\u{feff}').unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        // Many keys exist only with a ",P" (PC platform) suffix, while the
        // UILabel references the bare key. Alias the suffixed key to the bare
        // one so it still resolves; a real bare entry always wins.
        if let Some(base) = key.strip_suffix(",p") {
            map.entry(base.to_string()).or_insert_with(|| value.to_string());
        }
        map.insert(key, value.to_string());
    }
    map
}

/// Parse `defaultProfile.xml` into the action master list, resolving labels
/// against `loc`. Only actions inside an `<actionmap>` are collected (grouped
/// definitions like `<actiongroup>` are ignored).
pub fn parse_default_profile(xml: &str, loc: &HashMap<String, String>) -> Result<Vec<ActionMap>, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut maps: Vec<ActionMap> = Vec::new();
    let mut in_actionmap = false;
    // Inside an `<action>` element: only there do `<keyboard>`/`<gamepad>`/
    // `<joystick>` children carry a default. `<CustomisationUIHeader>` uses the
    // same element names for something else entirely.
    let mut in_action = false;

    loop {
        match reader.read_event().map_err(|e| format!("XML error: {e}"))? {
            Event::Start(e) if e.name().as_ref() == b"actionmap" => {
                maps.push(actionmap_from(&e, loc));
                in_actionmap = true;
                in_action = false;
            }
            Event::Empty(e) if e.name().as_ref() == b"actionmap" => {
                maps.push(actionmap_from(&e, loc));
            }
            Event::Start(e) if in_actionmap && e.name().as_ref() == b"action" => {
                if let Some(map) = maps.last_mut() {
                    map.actions.push(action_from(&e, loc));
                }
                in_action = true;
            }
            Event::Empty(e) if in_actionmap && e.name().as_ref() == b"action" => {
                if let Some(map) = maps.last_mut() {
                    map.actions.push(action_from(&e, loc));
                }
            }
            Event::End(e) if e.name().as_ref() == b"action" => {
                in_action = false;
            }
            Event::Start(e) | Event::Empty(e) if in_action => {
                if let Some(action) = maps.last_mut().and_then(|m| m.actions.last_mut()) {
                    apply_default_child(&e, action);
                }
            }
            Event::End(e) if e.name().as_ref() == b"actionmap" => {
                in_actionmap = false;
                in_action = false;
            }
            Event::Eof => break,
            _ => {}
        }
    }

    Ok(maps)
}

/// Apply a `<keyboard|mouse|gamepad|joystick input="…"/>` child of an `<action>`:
/// it overrides the attribute of the same kind, blank input included (that is
/// how SC unbinds a default it otherwise inherits).
fn apply_default_child(e: &BytesStart, action: &mut Action) {
    let field = match e.name().as_ref() {
        b"keyboard" => &mut action.keyboard_default,
        b"mouse" => &mut action.mouse_default,
        b"gamepad" => &mut action.gamepad_default,
        b"joystick" => &mut action.joystick_default,
        _ => return,
    };
    *field = binding_value(attributes(e).get("input"));
}

/// A raw default/binding value: trimmed, with blank meaning "unbound".
fn binding_value(raw: Option<&String>) -> Option<String> {
    raw.map(|s| s.trim()).filter(|s| !s.is_empty()).map(str::to_string)
}

fn actionmap_from(e: &BytesStart, loc: &HashMap<String, String>) -> ActionMap {
    let attrs = attributes(e);
    ActionMap {
        name: attrs.get("name").cloned().unwrap_or_default(),
        label: resolve(attrs.get("UILabel"), loc),
        actions: Vec::new(),
    }
}

fn action_from(e: &BytesStart, loc: &HashMap<String, String>) -> Action {
    let attrs = attributes(e);
    Action {
        name: attrs.get("name").cloned().unwrap_or_default(),
        label: resolve(attrs.get("UILabel"), loc),
        description: resolve(attrs.get("UIDescription"), loc),
        joystick_default: binding_value(attrs.get("joystick")),
        keyboard_default: binding_value(attrs.get("keyboard")),
        gamepad_default: binding_value(attrs.get("gamepad")),
        mouse_default: binding_value(attrs.get("mouse")),
    }
}

fn attributes(e: &BytesStart) -> HashMap<String, String> {
    attribute_list(e).into_iter().collect()
}

/// The element's attributes in file order.
fn attribute_list(e: &BytesStart) -> Vec<(String, String)> {
    e.attributes()
        .flatten()
        .map(|attr| {
            let key = String::from_utf8_lossy(attr.key.as_ref()).into_owned();
            let value = attr.unescape_value().map(|v| v.into_owned()).unwrap_or_default();
            (key, value)
        })
        .collect()
}

/// Resolve an `@ui_*` key against the localization table. Returns `None` for a
/// missing/blank key or one that has no entry.
fn resolve(key: Option<&String>, loc: &HashMap<String, String>) -> Option<String> {
    let key = key.map(|k| k.trim()).filter(|k| !k.is_empty())?;
    let key = key.strip_prefix('@').unwrap_or(key);
    // Case-insensitive: the localization table stores keys lowercased.
    loc.get(&key.to_ascii_lowercase()).cloned()
}

/// The token prefix a `<device name="...">` in `keybinding_localization.xml`
/// contributes to, or `None` for a device we do not know. `joystickN` ->
/// `jsN_`, `keyboard` and `mouse` -> `kb1_` (SC binds the mouse under the
/// keyboard prefix: `kb1_mouse1`), `control_pad` -> `gp1_`.
fn token_prefix_for_device(name: &str) -> Option<String> {
    match name {
        "keyboard" | "mouse" => Some("kb1_".to_string()),
        "control_pad" => Some("gp1_".to_string()),
        _ => name
            .strip_prefix("joystick")
            .and_then(|n| n.parse::<u32>().ok())
            .map(|i| format!("js{i}_")),
    }
}

/// Parse `keybinding_localization.xml` into display labels for input tokens,
/// keyed by the full SC token: `"js1_button1"` -> `"Button 1"`, `"js2_button1"`
/// -> `"Button 1 (Input 2)"`, `"kb1_a"` -> `"A"`, `"gp1_dpad_up"` -> `"D-Pad Up"`.
/// SC's per-instance joystick strings differ (js1 has no suffix, js2+ carry
/// "(Input N)"), so each `joystickN` device is kept separate rather than
/// treated as canonical; keyboard and gamepad exist exactly once. The
/// `localizationString` is taken from the XML (hat directions use camelCase
/// keys like `hat1Up`, so it must not be rebuilt) and resolved against `loc`.
/// The `mouse` device lands under `kb1_` too (`kb1_mouse1`, `kb1_mwheel_up`).
pub fn parse_token_labels(xml: &str, loc: &HashMap<String, String>) -> HashMap<String, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut labels = HashMap::new();
    // The token prefix while inside a `<device>` we bind.
    let mut prefix: Option<String> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) if e.name().as_ref() == b"device" => {
                prefix = attributes(&e).get("name").and_then(|n| token_prefix_for_device(n));
            }
            Ok(Event::End(e)) if e.name().as_ref() == b"device" => {
                prefix = None;
            }
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if prefix.is_some() && e.name().as_ref() == b"Key" => {
                let attrs = attributes(&e);
                if let (Some(name), Some(label)) =
                    (attrs.get("name"), resolve(attrs.get("localizationString"), loc))
                {
                    labels.insert(format!("{}{name}", prefix.as_deref().unwrap()), label);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    labels
}

/// A joystick device as SC's user config knows it: its `jsN` instance number
/// plus the stored product name and GUID from the `<options>` block.
#[derive(Debug, Clone, Serialize)]
pub struct JoystickDevice {
    pub instance: u32,
    pub product_name: String,
    pub product_guid: Option<String>,
}

/// One user rebind: which action (identified by its actionmap + action name)
/// got which raw input string.
#[derive(Debug, Clone, Serialize)]
pub struct Rebind {
    pub actionmap: String,
    pub action: String,
    pub input: String,
    /// The element's other attributes in file order (`activationMode`,
    /// `multiTap`, …) — part of the binding, carried along by an apply.
    pub attrs: Vec<(String, String)>,
}

/// The user's `actionmaps.xml`: the joystick instance→device map and every
/// rebind. Layered on top of the [`ActionMap`] master list.
#[derive(Debug, Clone, Serialize)]
pub struct ActionMapsFile {
    pub joysticks: Vec<JoystickDevice>,
    pub rebinds: Vec<Rebind>,
}

/// Parse the user's `actionmaps.xml`.
pub fn parse_actionmaps(xml: &str) -> Result<ActionMapsFile, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut joysticks = Vec::new();
    let mut rebinds = Vec::new();
    let mut cur_map = String::new();
    let mut cur_action = String::new();

    loop {
        match reader.read_event().map_err(|e| format!("XML error: {e}"))? {
            Event::Start(e) | Event::Empty(e) if e.name().as_ref() == b"options" => {
                let attrs = attributes(&e);
                if attrs.get("type").map(String::as_str) == Some("joystick") {
                    if let Some(device) = joystick_device_from(&attrs) {
                        joysticks.push(device);
                    }
                }
            }
            Event::Start(e) if e.name().as_ref() == b"actionmap" => {
                cur_map = attributes(&e).get("name").cloned().unwrap_or_default();
            }
            Event::Start(e) if e.name().as_ref() == b"action" => {
                cur_action = attributes(&e).get("name").cloned().unwrap_or_default();
            }
            Event::Start(e) | Event::Empty(e) if e.name().as_ref() == b"rebind" => {
                let attrs = attribute_list(&e);
                if let Some(input) = attrs.iter().find(|(k, _)| k == "input").map(|(_, v)| v.clone()) {
                    rebinds.push(Rebind {
                        actionmap: cur_map.clone(),
                        action: cur_action.clone(),
                        input,
                        attrs: attrs.into_iter().filter(|(k, _)| k != "input").collect(),
                    });
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    Ok(ActionMapsFile { joysticks, rebinds })
}

fn joystick_device_from(attrs: &HashMap<String, String>) -> Option<JoystickDevice> {
    let instance: u32 = attrs.get("instance")?.parse().ok()?;
    // Empty joystick slots have no Product attribute; skip them.
    let product = attrs.get("Product")?;
    let (product_name, product_guid) = split_product(product);
    Some(JoystickDevice { instance, product_name, product_guid })
}

/// Split an SC `Product` string into its name and `{GUID}` parts, e.g.
/// `" VKBsim Gladiator EVO  L    {0201231D-...}"` -> `("VKBsim Gladiator EVO  L", Some("{0201231D-...}"))`.
/// The same format appears in `Game.log`'s device lines (see `gamelog`).
pub(crate) fn split_product(product: &str) -> (String, Option<String>) {
    if let Some(open) = product.rfind('{') {
        if product.trim_end().ends_with('}') {
            return (product[..open].trim().to_string(), Some(product[open..].trim().to_string()));
        }
    }
    (product.trim().to_string(), None)
}

/// Whether a raw rebind input targets a joystick at all — a real token
/// (`js1_button6`, `lctrl+js1_x`) or a blank one (`js2_ `) that explicitly
/// unbinds the shipped default. Unlike [`parse_js_binding`], blank counts.
pub fn is_joystick_rebind(input: &str) -> bool {
    input.rsplit('+').next().is_some_and(|main| main.trim_start().starts_with("js"))
}

/// Parse a raw rebind input into its joystick `(instance, token)`, or `None`
/// if it is not a bound joystick input. Handles a modifier prefix
/// (`lctrl+js1_button1`) by taking the part after the last `+`, and treats a
/// whitespace-only token (`js2_ `) as unbound.
pub fn parse_js_binding(input: &str) -> Option<(u32, String)> {
    let main = input.rsplit('+').next()?;
    let rest = main.strip_prefix("js")?;
    let (num, token) = rest.split_once('_')?;
    let instance: u32 = num.parse().ok()?;
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    Some((instance, token.to_string()))
}

/// The device a raw rebind input targets, and its token when it is really
/// bound. See [`parse_rebind`].
#[derive(Debug, Clone, PartialEq)]
pub struct RebindTarget {
    pub kind: DeviceKind,
    /// SC instance: `jsN` for joysticks, always 1 for keyboard and gamepad.
    pub instance: u32,
    /// The token without its device prefix (`button6`, `lalt+x`), or `None`
    /// for a blank rebind (`js1_ `, `kb1_ `) that deliberately unbinds the
    /// shipped default.
    pub token: Option<String>,
}

impl RebindTarget {
    /// The full SC token (`js2_button9`, `kb1_lalt+x`), or `None` when blank.
    pub fn full_token(&self) -> Option<String> {
        let token = self.token.as_deref()?;
        Some(format!("{}{}_{token}", self.kind.token_prefix(), self.instance))
    }
}

/// Classify a raw rebind input from `actionmaps.xml`.
///
/// Keyboard and gamepad tokens carry their modifiers *inside* the token, after
/// the single prefix (`kb1_lalt+x`, `gp1_shoulderl+y`), while a joystick token
/// puts the modifier in front of the prefix (`lctrl+js1_button1`) — hence the
/// two different parses. Returns `None` for anything else (no prefix, empty).
pub fn parse_rebind(input: &str) -> Option<RebindTarget> {
    let trimmed = input.trim_start();
    for kind in [DeviceKind::Keyboard, DeviceKind::Gamepad] {
        if let Some(token) = trimmed.strip_prefix(&format!("{}1_", kind.token_prefix())) {
            let token = token.trim();
            return Some(RebindTarget {
                kind,
                instance: 1,
                token: (!token.is_empty()).then(|| token.to_string()),
            });
        }
    }
    if !is_joystick_rebind(input) {
        return None;
    }
    // Blank joystick rebinds still name their instance (`js2_ `).
    let main = input.rsplit('+').next()?;
    let (num, token) = main.trim_start().strip_prefix("js")?.split_once('_')?;
    let instance: u32 = num.parse().ok()?;
    let token = token.trim();
    Some(RebindTarget { kind: DeviceKind::Joystick, instance, token: (!token.is_empty()).then(|| token.to_string()) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_actionmaps_and_resolves_labels() {
        let xml = r#"<profile>
          <actiongroup action="v_attack"><action name="v_attack_all"/></actiongroup>
          <actionmap name="seat_general" UILabel="@ui_grp">
            <action name="v_eject" UILabel="@ui_eject" UIDescription="@ui_eject_desc" joystick=" "/>
            <action name="v_look" UILabel="@ui_look" joystick="js1_button3"/>
          </actionmap>
        </profile>"#;

        let mut loc = HashMap::new();
        loc.insert("ui_grp".to_string(), "Seat General".to_string());
        loc.insert("ui_eject".to_string(), "Eject".to_string());
        loc.insert("ui_eject_desc".to_string(), "Eject from seat".to_string());
        // ui_look intentionally absent -> unresolved

        let maps = parse_default_profile(xml, &loc).unwrap();

        // the <actiongroup> and its action are ignored
        assert_eq!(maps.len(), 1);
        let m = &maps[0];
        assert_eq!(m.name, "seat_general");
        assert_eq!(m.label.as_deref(), Some("Seat General"));
        assert_eq!(m.actions.len(), 2);

        assert_eq!(m.actions[0].name, "v_eject");
        assert_eq!(m.actions[0].label.as_deref(), Some("Eject"));
        assert_eq!(m.actions[0].description.as_deref(), Some("Eject from seat"));
        assert_eq!(m.actions[0].joystick_default, None); // " " -> unbound

        assert_eq!(m.actions[1].label, None); // key not in loc
        assert_eq!(m.actions[1].joystick_default.as_deref(), Some("js1_button3"));
    }

    #[test]
    fn parses_localization_with_bom_and_equals_in_value() {
        let ini = "\u{feff}ui_a=Hello\r\nui_b=x=y=z\r\nbroken line\r\n";
        let loc = parse_localization(ini);
        assert_eq!(loc.get("ui_a").map(String::as_str), Some("Hello"));
        assert_eq!(loc.get("ui_b").map(String::as_str), Some("x=y=z"));
        assert_eq!(loc.get("broken line"), None);
    }

    #[test]
    fn aliases_pc_platform_suffix() {
        // global.ini only has "ui_v_master_mode_cycle,P"; the action references
        // it without the suffix. A real bare entry wins over the alias.
        let loc = parse_localization("ui_v_master_mode_cycle,P=Cycle Master Mode\r\nui_x,P=alias\r\nui_x=real\r\n");
        assert_eq!(loc.get("ui_v_master_mode_cycle").map(String::as_str), Some("Cycle Master Mode"));
        assert_eq!(loc.get("ui_x").map(String::as_str), Some("real"));

        let xml = r#"<profile><actionmap name="m">
            <action name="v_master_mode_cycle" UILabel="@ui_v_master_mode_cycle"/>
        </actionmap></profile>"#;
        let maps = parse_default_profile(xml, &loc).unwrap();
        assert_eq!(maps[0].actions[0].label.as_deref(), Some("Cycle Master Mode"));
    }

    #[test]
    fn parses_token_labels_per_instance_and_device_kind() {
        // js1 has no suffix; js2+ carry "(Input N)". Axis/hat keys differ from
        // the bind token name (hat1_up -> hat1Up). The keyboard and the
        // control_pad exist once each (kb1_/gp1_); the mouse shares kb1_.
        let loc = parse_localization(concat!(
            "input_key_joystick1_button1=Button 1\r\n",
            "input_key_joystick2_button1=Button 1 (Input 2)\r\n",
            "input_key_joystick1_hat1Up=Up (Hat 1)\r\n",
            "input_key_keyboard_a=A\r\n",
            "input_key_controlPad_dpadUp=D-Pad Up\r\n",
            "input_key_mouse_mouse1=Mouse 1\r\n",
        ));
        let xml = r#"<LocalizedKeys>
          <device name="joystick1">
            <Key name="button1" localizationString="@input_key_joystick1_button1"/>
            <Key name="hat1_up" localizationString="@input_key_joystick1_hat1Up"/>
          </device>
          <device name="joystick2">
            <Key name="button1" localizationString="@input_key_joystick2_button1"/>
          </device>
          <device name="keyboard">
            <Key name="a" localizationString="@input_key_keyboard_a"/>
          </device>
          <device name="control_pad">
            <Key name="dpad_up" localizationString="@input_key_controlPad_dpadUp"/>
          </device>
          <device name="mouse">
            <Key name="mouse1" localizationString="@input_key_mouse_mouse1"/>
          </device>
        </LocalizedKeys>"#;

        let tokens = parse_token_labels(xml, &loc);
        assert_eq!(tokens.get("js1_button1").map(String::as_str), Some("Button 1"));
        assert_eq!(tokens.get("js2_button1").map(String::as_str), Some("Button 1 (Input 2)"));
        assert_eq!(tokens.get("js1_hat1_up").map(String::as_str), Some("Up (Hat 1)"));
        assert_eq!(tokens.get("kb1_a").map(String::as_str), Some("A"));
        assert_eq!(tokens.get("gp1_dpad_up").map(String::as_str), Some("D-Pad Up"));
        assert_eq!(tokens.get("kb1_mouse1").map(String::as_str), Some("Mouse 1"));
        assert_eq!(tokens.len(), 6);
    }

    #[test]
    fn reads_defaults_from_attributes_and_child_elements() {
        // SC uses both forms; a child wins over the attribute of the same kind,
        // a blank value means unbound, and the same element names outside an
        // <action> (the UI header) must not be mistaken for defaults.
        let xml = r#"<profile>
          <CustomisationUIHeader>
            <keyboard label="@x" image="KeyboardDefault"/>
            <gamepad label="@x" image="GamePadDefault"/>
            <joystick label="@x" image="JoystickDefault"/>
          </CustomisationUIHeader>
          <actionmap name="m">
            <action name="attrs_only" keyboard="lalt+x" mouse="mouse1" gamepad="a" joystick="button1"/>
            <action name="blank_attrs" keyboard=" " mouse="" gamepad=" " joystick=" "/>
            <action name="child_wins" keyboard="q" mouse="mouse2" gamepad="a">
              <keyboard activationMode="double_tap" input="e"/>
              <mouse activationMode="tap" input="mwheel_up"/>
              <gamepad activationMode="tap" input="shoulderl+thumbl_left"/>
            </action>
            <action name="child_unbinds" gamepad="a">
              <gamepad activationMode="delayed_press_medium" input=" "/>
            </action>
            <action name="joystick_child" gamepad="b">
              <joystick activationMode="press" input=" "/>
              <states><state name="s"/></states>
            </action>
          </actionmap>
        </profile>"#;

        let maps = parse_default_profile(xml, &HashMap::new()).unwrap();
        let find = |name: &str| maps[0].actions.iter().find(|a| a.name == name).unwrap();

        let a = find("attrs_only");
        assert_eq!(a.keyboard_default.as_deref(), Some("lalt+x"));
        assert_eq!(a.mouse_default.as_deref(), Some("mouse1"));
        assert_eq!(a.gamepad_default.as_deref(), Some("a"));
        assert_eq!(a.joystick_default.as_deref(), Some("button1"));

        let b = find("blank_attrs");
        assert_eq!((&b.keyboard_default, &b.mouse_default, &b.gamepad_default, &b.joystick_default), (&None, &None, &None, &None));

        let c = find("child_wins");
        assert_eq!(c.keyboard_default.as_deref(), Some("e"));
        assert_eq!(c.mouse_default.as_deref(), Some("mwheel_up"));
        assert_eq!(c.gamepad_default.as_deref(), Some("shoulderl+thumbl_left"));

        // A blank child unbinds what the attribute set.
        assert_eq!(find("child_unbinds").gamepad_default, None);

        let e = find("joystick_child");
        assert_eq!(e.joystick_default, None);
        assert_eq!(e.gamepad_default.as_deref(), Some("b"));

        // The UI header elements produced no action of their own.
        assert_eq!(maps[0].actions.len(), 5);
    }

    #[test]
    fn classifies_rebinds_by_device_kind() {
        let target = |input: &str| parse_rebind(input).unwrap();

        // Joystick: modifier in front of the prefix, instance from the token.
        let js = target("lctrl+js2_button9");
        assert_eq!((js.kind, js.instance), (DeviceKind::Joystick, 2));
        assert_eq!(js.token.as_deref(), Some("button9"));
        assert_eq!(js.full_token().as_deref(), Some("js2_button9"));

        // Keyboard/gamepad: single prefix, modifiers stay inside the token.
        let kb = target("kb1_lalt+x");
        assert_eq!((kb.kind, kb.instance), (DeviceKind::Keyboard, 1));
        assert_eq!(kb.token.as_deref(), Some("lalt+x"));
        assert_eq!(kb.full_token().as_deref(), Some("kb1_lalt+x"));

        let gp = target("gp1_shoulderl+y");
        assert_eq!((gp.kind, gp.instance), (DeviceKind::Gamepad, 1));
        assert_eq!(gp.full_token().as_deref(), Some("gp1_shoulderl+y"));

        // Blank = deliberately unbound, but the device is still named.
        for input in ["js1_ ", "kb1_ ", "gp1_ "] {
            let t = target(input);
            assert_eq!(t.token, None, "{input}");
            assert_eq!(t.full_token(), None, "{input}");
        }

        // Not a device we bind.
        assert_eq!(parse_rebind("mouse1"), None);
        assert_eq!(parse_rebind(" "), None);
        assert_eq!(parse_rebind(""), None);
    }

    #[test]
    fn resolves_labels_case_insensitively() {
        // global.ini has "ui_CIBoost"; defaultProfile references "@ui_CIboost".
        let loc = parse_localization("ui_CIBoost=Boost\r\n");
        let xml = r#"<profile><actionmap name="m">
            <action name="v_boost" UILabel="@ui_CIboost"/>
        </actionmap></profile>"#;
        let maps = parse_default_profile(xml, &loc).unwrap();
        assert_eq!(maps[0].actions[0].label.as_deref(), Some("Boost"));
    }

    #[test]
    fn parses_js_bindings() {
        assert_eq!(parse_js_binding("js1_button9"), Some((1, "button9".to_string())));
        assert_eq!(parse_js_binding("js2_hat1_up"), Some((2, "hat1_up".to_string())));
        assert_eq!(parse_js_binding("lctrl+js1_button1"), Some((1, "button1".to_string())));
        assert_eq!(parse_js_binding("js2_ "), None); // device-tagged but unbound
        assert_eq!(parse_js_binding("kb1_insert"), None); // not a joystick
        assert_eq!(parse_js_binding(" "), None);
    }

    #[test]
    fn detects_joystick_rebinds_including_blank_ones() {
        assert!(is_joystick_rebind("js1_button6"));
        assert!(is_joystick_rebind("lctrl+js1_x"));
        assert!(is_joystick_rebind("js2_ ")); // blank = deliberately unbound
        assert!(!is_joystick_rebind("kb1_insert"));
        assert!(!is_joystick_rebind(" "));
    }

    #[test]
    fn splits_product_string() {
        let (name, guid) = split_product(" VKBsim Gladiator EVO  L    {0201231D-0000-0000-0000-504944564944}");
        assert_eq!(name, "VKBsim Gladiator EVO  L"); // trimmed ends, inner spaces kept
        assert_eq!(guid.as_deref(), Some("{0201231D-0000-0000-0000-504944564944}"));

        let (name, guid) = split_product("Controller (Gamepad)");
        assert_eq!(name, "Controller (Gamepad)");
        assert_eq!(guid, None);
    }

    #[test]
    fn parses_actionmaps() {
        let xml = r#"<ActionMaps>
          <options type="keyboard" instance="1" Product="Wine Keyboard  {6F1D2B61-...}"/>
          <options type="joystick" instance="1" Product=" VKB L {0201231D-...}"/>
          <options type="joystick" instance="2" Product=" VKB R {0200231D-...}"/>
          <options type="joystick" instance="3"/>
          <actionmap name="seat_general">
            <action name="v_eject"><rebind input="js2_ "/></action>
            <action name="v_toggle_flight_mode"><rebind input="js1_button6"/></action>
          </actionmap>
        </ActionMaps>"#;

        let profile = parse_actionmaps(xml).unwrap();

        // only the two joystick options with a Product; the empty slot is skipped
        assert_eq!(profile.joysticks.len(), 2);
        assert_eq!(profile.joysticks[0].instance, 1);
        assert_eq!(profile.joysticks[0].product_guid.as_deref(), Some("{0201231D-...}"));

        assert_eq!(profile.rebinds.len(), 2);
        let flight = profile
            .rebinds
            .iter()
            .find(|r| r.action == "v_toggle_flight_mode")
            .unwrap();
        assert_eq!(flight.actionmap, "seat_general");
        assert_eq!(parse_js_binding(&flight.input), Some((1, "button6".to_string())));
    }
}
