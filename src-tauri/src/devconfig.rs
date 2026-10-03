//! Device settings — what the game's inversion / sensitivity-curve screens,
//! its deadzone / saturation sliders and its mouse / gamepad sensitivity
//! write: read from and written to the user's `actionmaps.xml` and
//! `attributes.xml`.
//!
//! `actionmaps.xml` keeps two kinds of settings inside `<ActionProfiles>`:
//!
//! - **per slot**, as children of `<options type="…" instance="N">`: one
//!   element per option-tree node (`scdata::OptionTree`), named after it,
//!   holding only what deviates from the shipped tree — `invert` before
//!   `exponent`, or a `<nonlinearity_curve>` of `<point in out/>` instead of
//!   the exponent. The game writes the children in tree order (pre-order).
//! - **per device**, as `<deviceoptions name="<raw Product>">` with one
//!   `<option input="x" deadzone="…"/>` per value (deadzone and saturation
//!   are separate elements). The gamepad is `Controller (Gamepad)`, the mouse
//!   `Mouse` with literal `@pause_Options…` inputs (smoothing is stored in a
//!   `saturation` attribute). Duplicates occur: reading takes the last one
//!   (an assumption, the game's own pick is unknown), writing updates every
//!   one — what the game did on save.
//!
//! `attributes.xml` (`<Attributes><Attr name value/>…`, sorted by name) holds
//! every game setting; only the [`ManagedAttribute`] entries are touched.
//!
//! The GUI works in the game's display units; this module converts (see the
//! `*_SCALE` constants and [`ManagedAttribute`]). The `actionmaps.xml` floats
//! are written like the game writes them (`%.8g` of the `f32`), the
//! `attributes.xml` ones as `%g` ([`format_actionmaps_float`],
//! [`format_attributes_float`]).
//!
//! Writing is textual, like `rebind.rs`: only the elements a change touches
//! are rewritten, everything else stays byte for byte. Every change is
//! validated against the option tree and the file first, and the rewrite is
//! parsed again and compared with the intent (the parsed file with the
//! changes applied to its model) before it is returned.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Mutex;

use log::{error, info, warn};
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::rebind::{indent_before, layout_of, Layout};
use crate::scdata::{self, DeviceKind, OptionNode, OptionTree};
use crate::xmltext::{attr, find_attr, insert_attr, mask_markup, remove_attr, set_attr, tag_end};
use crate::{backups, config, diff, gamefile, AppData, LoadStatus};

/// The `<deviceoptions>` name of the gamepad.
pub const GAMEPAD_DEVICE: &str = "Controller (Gamepad)";
/// The `<deviceoptions>` name of the mouse.
pub const MOUSE_DEVICE: &str = "Mouse";
/// The joystick axes with a deadzone / saturation in the game's screen. Not
/// in the game data; the labels are `ui_DeadzoneJoystick<Input>` /
/// `ui_SaturationJoystick<Input>`.
pub const JOYSTICK_AXES: [&str; 6] = ["x", "y", "z", "rotx", "roty", "rotz"];
/// The axis part of the game's English labels, per [`JOYSTICK_AXES`] entry
/// (`Deadzone Joystick X Axis`, `Saturation Joystick Z Rotation`).
const JOYSTICK_AXES_ENGLISH: [&str; 6] = ["X Axis", "Y Axis", "Z Axis", "X Rotation", "Y Rotation", "Z Rotation"];
/// The gamepad sticks with a deadzone (no saturation); labels
/// `ui_DeadzoneXI<Input>`.
pub const GAMEPAD_AXES: [&str; 2] = ["thumbl", "thumbr"];
/// The stick part of the game's English labels, per [`GAMEPAD_AXES`] entry
/// (`Deadzone Gamepad Thumb Left`).
const GAMEPAD_AXES_ENGLISH: [&str; 2] = ["Thumb Left", "Thumb Right"];
const MOUSE_ACCELERATION_INPUT: &str = "@pause_OptionsMouseAcceleration";
const MOUSE_SMOOTHING_INPUT: &str = "@pause_OptionsMouseSmoothing";

/// Display -> stored factors (stored = display × factor).
const JOYSTICK_SCALE: f64 = 0.99;
const GAMEPAD_SCALE: f64 = 0.899;
const ACCELERATION_SCALE: f64 = 0.1;
const SMOOTHING_SCALE: f64 = 0.9;

/// Most points a custom curve may have.
const MAX_POINTS: usize = 64;
/// Longest device name accepted from a change.
const MAX_DEVICE_NAME: usize = 256;

// ---------------------------------------------------------------------------
// Floats
// ---------------------------------------------------------------------------

/// C's `printf("%.<precision>g", value)`: `precision` significant digits,
/// fixed notation while the decimal exponent is in `-4..precision`,
/// scientific otherwise, trailing zeros (and a trailing point) stripped.
fn format_g(value: f64, precision: usize) -> String {
    let p = precision.max(1) as i32;
    if value == 0.0 || !value.is_finite() {
        return if value == 0.0 { "0".to_string() } else { value.to_string() };
    }
    // The exponent after rounding to `p` digits decides the style, as in C.
    let sci = format!("{:.*e}", (p - 1) as usize, value);
    let (mantissa, exp) = sci.split_once('e').unwrap_or((sci.as_str(), "0"));
    let x: i32 = exp.parse().unwrap_or(0);
    if (-4..p).contains(&x) {
        strip_zeros(&format!("{:.*}", (p - 1 - x) as usize, value))
    } else {
        format!("{}e{}{:02}", strip_zeros(mantissa), if x < 0 { '-' } else { '+' }, x.abs())
    }
}

fn strip_zeros(s: &str) -> String {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s.to_string()
    }
}

/// A float as the game writes it into `actionmaps.xml`: `%.8g` of the `f32`
/// (`0.7` -> `0.69999999`, `3` -> `3`).
pub fn format_actionmaps_float(value: f32) -> String {
    format_g(value as f64, 8)
}

/// A float as the game writes it into `attributes.xml`: `%g`, six
/// significant digits (`8.888888…` -> `8.88889`).
pub fn format_attributes_float(value: f64) -> String {
    format_g(value, 6)
}

/// `value` rounded to `1 / per_unit` steps.
fn round_step(value: f64, per_unit: f64) -> f64 {
    (value * per_unit).round() / per_unit
}

/// A stored `actionmaps.xml` value for a display value in 0.01 steps: the
/// exact product rounded to `f32` once, then `%.8g` — this reproduces the
/// game's `0.15 × 0.99` -> `0.1485`, `0.92 × 0.99` -> `0.91079998`.
fn stored_scaled(display: f64, scale: f64) -> String {
    format_actionmaps_float((round_step(display, 100.0) * scale) as f32)
}

/// A stored text back in display units, rounded to the GUI's 0.01 step.
fn display_scaled(stored: &str, scale: f64) -> Option<f32> {
    let v: f64 = stored.trim().parse().ok().filter(|v: &f64| v.is_finite())?;
    Some(round_step(v / scale, 100.0) as f32)
}

// ---------------------------------------------------------------------------
// The file model (stored values, raw text)
// ---------------------------------------------------------------------------

/// The device settings of an `actionmaps.xml` as stored — every value its
/// raw attribute text, every element in file order. What the writer edits
/// and its check compares; [`view`] turns it into display values.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DeviceConfig {
    pub options: Vec<OptionsEntry>,
    pub device_options: Vec<DeviceOptionsEntry>,
}

/// One `<options type instance [Product]>` element.
#[derive(Debug, Clone, PartialEq)]
pub struct OptionsEntry {
    pub kind: DeviceKind,
    pub instance: u32,
    pub product: Option<String>,
    pub nodes: Vec<NodeEntry>,
}

/// One child of an `<options>` element: a node's own settings.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeEntry {
    pub name: String,
    /// Every attribute in file order (`invert`, `exponent`).
    pub attrs: Vec<(String, String)>,
    /// The `<nonlinearity_curve>` points (`in`, `out`), when it has one (the
    /// last one with points, should there be several). An empty list when
    /// its curve elements hold no point: read as no custom curve.
    pub points: Option<Vec<(String, String)>>,
    /// Content other than `<nonlinearity_curve>`: child elements, comments,
    /// text, CDATA (none in the game's files; counted so such an element is
    /// never taken for empty — a user's comment keeps its element).
    pub other_children: usize,
    /// Content of the `<nonlinearity_curve>` elements other than `<point>`,
    /// counted the same way: a curve element holding some stays when its
    /// points go.
    pub curve_other: usize,
}

/// One `<deviceoptions name>` element.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceOptionsEntry {
    pub name: String,
    pub entries: Vec<OptionEntry>,
    /// Content other than `<option>` elements, counted like
    /// [`NodeEntry::other_children`].
    pub other_children: usize,
}

/// One `<option input …>` of a `<deviceoptions>` element.
#[derive(Debug, Clone, PartialEq)]
pub struct OptionEntry {
    pub input: String,
    /// The other attributes in file order (`deadzone`, `saturation`, …).
    pub attrs: Vec<(String, String)>,
}

fn kind_attr(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Joystick => "joystick",
        DeviceKind::Keyboard => "keyboard",
        DeviceKind::Gamepad => "gamepad",
    }
}

fn kind_from_attr(value: &str) -> Option<DeviceKind> {
    [DeviceKind::Joystick, DeviceKind::Keyboard, DeviceKind::Gamepad].into_iter().find(|k| kind_attr(*k) == value)
}

/// What an open element is, for the parse below.
#[derive(Clone, Copy, PartialEq)]
enum Ctx {
    Options,
    Node,
    Curve,
    DeviceOptions,
    Other,
}

/// Parse the device settings of an `actionmaps.xml` (the live file and the
/// game's exported profiles alike: the root must be `<ActionMaps>`). An
/// `<options>` element of an unknown `type` or without a numeric `instance`
/// is skipped. Never panics; malformed XML is an error.
pub fn parse_device_config(xml: &str) -> Result<DeviceConfig, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut config = DeviceConfig::default();
    // The open elements, outermost first.
    let mut stack: Vec<Ctx> = Vec::new();
    let mut root_seen = false;
    // The points of the curve element being read.
    let mut curve: Vec<(String, String)> = Vec::new();

    loop {
        let event = reader.read_event().map_err(|e| format!("XML error: {e}"))?;
        let (e, empty) = match &event {
            Event::Start(e) => (e, false),
            Event::Empty(e) => (e, true),
            Event::End(_) => {
                if stack.pop() == Some(Ctx::Curve) && !curve.is_empty() {
                    if let Some(node) = config.options.last_mut().and_then(|b| b.nodes.last_mut()) {
                        node.points = Some(std::mem::take(&mut curve));
                    }
                }
                continue;
            }
            Event::Eof => break,
            // Not an element, but content all the same: it keeps a node or
            // device element from being empty (whitespace is trimmed away).
            Event::Text(_) | Event::CData(_) | Event::Comment(_) | Event::PI(_) => {
                match stack.last() {
                    Some(Ctx::Node) => {
                        if let Some(node) = config.options.last_mut().and_then(|b| b.nodes.last_mut()) {
                            node.other_children += 1;
                        }
                    }
                    Some(Ctx::Curve) => {
                        if let Some(node) = config.options.last_mut().and_then(|b| b.nodes.last_mut()) {
                            node.curve_other += 1;
                        }
                    }
                    Some(Ctx::DeviceOptions) => {
                        if let Some(device) = config.device_options.last_mut() {
                            device.other_children += 1;
                        }
                    }
                    _ => {}
                }
                continue;
            }
            _ => continue,
        };
        let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
        if !root_seen {
            if name != "ActionMaps" {
                return Err(format!("not a bindings file (root element <{name}>, expected <ActionMaps>)"));
            }
            root_seen = true;
        }
        let attrs = scdata::attribute_list(e)?;
        let get = |k: &str| attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        let ctx = match stack.last().copied() {
            Some(Ctx::Options) => {
                if let Some(block) = config.options.last_mut() {
                    block.nodes.push(NodeEntry { name, attrs, points: None, other_children: 0, curve_other: 0 });
                }
                Ctx::Node
            }
            Some(Ctx::Node) => {
                let node = config.options.last_mut().and_then(|b| b.nodes.last_mut());
                if name == "nonlinearity_curve" {
                    if let Some(node) = node {
                        node.points.get_or_insert_with(Vec::new);
                    }
                    curve.clear();
                    Ctx::Curve
                } else {
                    if let Some(node) = node {
                        node.other_children += 1;
                    }
                    Ctx::Other
                }
            }
            Some(Ctx::Curve) => {
                if name == "point" {
                    curve.push((get("in").unwrap_or_default(), get("out").unwrap_or_default()));
                } else if let Some(node) = config.options.last_mut().and_then(|b| b.nodes.last_mut()) {
                    node.curve_other += 1;
                }
                Ctx::Other
            }
            Some(Ctx::DeviceOptions) => {
                if let Some(device) = config.device_options.last_mut() {
                    if name == "option" {
                        let input = get("input").unwrap_or_default();
                        let attrs = attrs.into_iter().filter(|(k, _)| k != "input").collect();
                        device.entries.push(OptionEntry { input, attrs });
                    } else {
                        device.other_children += 1;
                    }
                }
                Ctx::Other
            }
            _ if name == "options" => {
                let kind = get("type").and_then(|t| kind_from_attr(&t));
                let instance = get("instance").and_then(|i| i.parse::<u32>().ok());
                match (kind, instance) {
                    (Some(kind), Some(instance)) => {
                        config.options.push(OptionsEntry { kind, instance, product: get("Product"), nodes: Vec::new() });
                        Ctx::Options
                    }
                    _ => Ctx::Other,
                }
            }
            _ if name == "deviceoptions" => {
                config.device_options.push(DeviceOptionsEntry { name: get("name").unwrap_or_default(), entries: Vec::new(), other_children: 0 });
                Ctx::DeviceOptions
            }
            _ => Ctx::Other,
        };
        if !empty {
            stack.push(ctx);
        }
    }

    if !root_seen {
        return Err("not a bindings file (empty document)".into());
    }
    Ok(config)
}

/// The `<Attr name value/>` entries of an `attributes.xml` in file order
/// (the root must be `<Attributes>`; entries without a name are skipped).
fn attribute_entries(xml: &str) -> Result<Vec<(String, String)>, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut root_seen = false;
    loop {
        let event = reader.read_event().map_err(|e| format!("XML error: {e}"))?;
        let (e, empty) = match &event {
            Event::Start(e) => (e, false),
            Event::Empty(e) => (e, true),
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                continue;
            }
            Event::Eof => break,
            _ => continue,
        };
        if !root_seen {
            if e.name().as_ref() != b"Attributes" {
                return Err(format!(
                    "not a settings file (root element <{}>, expected <Attributes>)",
                    String::from_utf8_lossy(e.name().as_ref())
                ));
            }
            root_seen = true;
        } else if depth == 1 && e.name().as_ref() == b"Attr" {
            let attrs = scdata::attribute_list(e)?;
            let get = |k: &str| attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
            if let Some(name) = get("name") {
                out.push((name, get("value").unwrap_or_default()));
            }
        }
        if !empty {
            depth += 1;
        }
    }
    if !root_seen {
        return Err("not a settings file (empty document)".into());
    }
    Ok(out)
}

/// Parse `attributes.xml` into name -> raw value (a name listed twice: the
/// last one).
pub fn parse_attributes(xml: &str) -> Result<BTreeMap<String, String>, String> {
    Ok(attribute_entries(xml)?.into_iter().collect())
}

// ---------------------------------------------------------------------------
// The view (display values, for the GUI)
// ---------------------------------------------------------------------------

/// The device settings of one source, in the game's display units.
#[derive(Debug, Clone, Serialize)]
pub struct DeviceConfigView {
    /// Every `<options>` element of the file.
    pub options: Vec<OptionsBlock>,
    /// Every `<deviceoptions>` element except the mouse's (that one is
    /// [`MouseView::acceleration`] / [`MouseView::smoothing`]).
    pub device_options: Vec<DeviceOptionsView>,
    pub mouse: MouseView,
    /// `attributes.xml` `Sensitivity`.
    pub gamepad_sensitivity: Option<f32>,
    /// False for profiles and backups without an `attributes.xml`.
    pub has_attributes: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct OptionsBlock {
    pub kind: DeviceKind,
    pub instance: u32,
    /// The raw `Product` attribute.
    pub product: Option<String>,
    /// Node name -> its own settings (only nodes with one).
    pub values: BTreeMap<String, NodeValue>,
}

/// A node's own settings; `None` = not set (the tree default or the parent's
/// value applies).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeValue {
    pub invert: Option<bool>,
    pub exponent: Option<f32>,
    pub points: Option<Vec<[f32; 2]>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceOptionsView {
    /// The raw `name` attribute (`Controller (Gamepad)` or a joystick's
    /// Product string).
    pub name: String,
    /// Deadzone / saturation per input, in file order of first appearance.
    pub axes: Vec<AxisOptionView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AxisOptionView {
    pub input: String,
    pub deadzone: Option<f32>,
    pub saturation: Option<f32>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct MouseView {
    pub sensitivity: Option<f32>,
    pub ads_percent: Option<f32>,
    pub zoom_scaling_enabled: Option<bool>,
    pub zoom_scaling_percent: Option<f32>,
    pub acceleration: Option<f32>,
    pub smoothing: Option<f32>,
}

fn node_value(node: &NodeEntry) -> NodeValue {
    let get = |k: &str| node.attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.trim());
    let num = |s: &str| s.trim().parse::<f32>().ok().filter(|v| v.is_finite());
    let points = node.points.as_ref().map(|pts| {
        pts.iter().filter_map(|(i, o)| Some([num(i)?, num(o)?])).collect::<Vec<_>>()
    });
    NodeValue {
        invert: match get("invert") {
            Some("1") => Some(true),
            Some("0") => Some(false),
            _ => None,
        },
        // The slider's 0.1 grid plus the game's float noise (`2.1000001`):
        // two decimals keep the value and drop the noise.
        exponent: get("exponent").and_then(num).map(|v| round_step(v as f64, 100.0) as f32),
        points: points.filter(|p| !p.is_empty()),
    }
}

/// The last value of `attr` among the `<option input>` entries of every
/// `<deviceoptions name>` element.
fn last_option<'a>(config: &'a DeviceConfig, device: &str, input: &str, attr_name: &str) -> Option<&'a str> {
    config
        .device_options
        .iter()
        .filter(|d| d.name == device)
        .flat_map(|d| &d.entries)
        .filter(|e| e.input == input)
        .filter_map(|e| e.attrs.iter().find(|(k, _)| k == attr_name).map(|(_, v)| v.as_str()))
        .next_back()
}

/// Turn a parsed file (plus the `attributes.xml` entries, when there are
/// any) into display values.
pub fn view(config: &DeviceConfig, attributes: Option<&BTreeMap<String, String>>) -> DeviceConfigView {
    let options = config
        .options
        .iter()
        .map(|block| OptionsBlock {
            kind: block.kind,
            instance: block.instance,
            product: block.product.clone(),
            values: block
                .nodes
                .iter()
                .map(|n| (n.name.clone(), node_value(n)))
                .filter(|(_, v)| v.invert.is_some() || v.exponent.is_some() || v.points.is_some())
                .collect(),
        })
        .collect();
    let device_options = config
        .device_options
        .iter()
        .filter(|d| d.name != MOUSE_DEVICE)
        .map(|d| {
            let scale = if d.name == GAMEPAD_DEVICE { GAMEPAD_SCALE } else { JOYSTICK_SCALE };
            let mut axes: Vec<AxisOptionView> = Vec::new();
            for entry in &d.entries {
                for (k, v) in &entry.attrs {
                    let slot = match k.as_str() {
                        "deadzone" | "saturation" => k.as_str(),
                        _ => continue,
                    };
                    let i = match axes.iter().position(|a| a.input == entry.input) {
                        Some(i) => i,
                        None => {
                            axes.push(AxisOptionView { input: entry.input.clone(), deadzone: None, saturation: None });
                            axes.len() - 1
                        }
                    };
                    let value = display_scaled(v, scale);
                    if slot == "deadzone" {
                        axes[i].deadzone = value;
                    } else {
                        axes[i].saturation = value;
                    }
                }
            }
            DeviceOptionsView { name: d.name.clone(), axes }
        })
        .collect();
    let attribute = |setting: ManagedAttribute| attributes.and_then(|a| a.get(setting.attr_name())).and_then(|s| setting.display(s));
    let mouse = MouseView {
        sensitivity: attribute(ManagedAttribute::MouseSensitivity),
        ads_percent: attribute(ManagedAttribute::AdsPercent),
        zoom_scaling_enabled: attribute(ManagedAttribute::ZoomScalingEnabled).map(|v| v != 0.0),
        zoom_scaling_percent: attribute(ManagedAttribute::ZoomScalingPercent),
        acceleration: last_option(config, MOUSE_DEVICE, MOUSE_ACCELERATION_INPUT, "acceleration")
            .and_then(|v| display_scaled(v, ACCELERATION_SCALE)),
        smoothing: last_option(config, MOUSE_DEVICE, MOUSE_SMOOTHING_INPUT, "saturation").and_then(|v| display_scaled(v, SMOOTHING_SCALE)),
    };
    DeviceConfigView {
        options,
        device_options,
        mouse,
        gamepad_sensitivity: attribute(ManagedAttribute::GamepadSensitivity),
        has_attributes: attributes.is_some(),
    }
}

// ---------------------------------------------------------------------------
// Labels of the hard-coded rows
// ---------------------------------------------------------------------------

/// The game's labels for the rows that are not in the option tree (resolved
/// from the `global.ini` the game data was loaded with; cached with it).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConfigLabels {
    /// Input (`x` … `rotz`) -> `Deadzone Joystick X Axis`.
    pub joystick_deadzone: BTreeMap<String, String>,
    pub joystick_saturation: BTreeMap<String, String>,
    /// `thumbl` / `thumbr` -> `Deadzone Gamepad Thumb Left`.
    pub gamepad_deadzone: BTreeMap<String, String>,
    pub gamepad_sensitivity: String,
    pub mouse_sensitivity: String,
    pub mouse_ads_percent: String,
    pub mouse_zoom_scaling_enabled: String,
    pub mouse_zoom_scaling_percent: String,
    pub mouse_acceleration: String,
    pub mouse_smoothing: String,
}

/// Resolve the [`ConfigLabels`] against `loc`; a key that does not resolve
/// falls back to the game's English label.
pub fn config_labels(loc: &HashMap<String, String>) -> ConfigLabels {
    let label = |key: &str| scdata::resolve(Some(&key.to_string()), loc);
    let capitalized = |s: &str| {
        let mut c = s.chars();
        c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
    };
    let or_english = |key: &str, english: &str| label(key).unwrap_or_else(|| english.to_string());
    let per_input = |prefix: &str, english: &str, inputs: &[&str], inputs_english: &[&str]| -> BTreeMap<String, String> {
        inputs
            .iter()
            .zip(inputs_english)
            .map(|(i, e)| (i.to_string(), or_english(&format!("{prefix}{}", capitalized(i)), &format!("{english} {e}"))))
            .collect()
    };
    ConfigLabels {
        joystick_deadzone: per_input("ui_DeadzoneJoystick", "Deadzone Joystick", &JOYSTICK_AXES, &JOYSTICK_AXES_ENGLISH),
        joystick_saturation: per_input("ui_SaturationJoystick", "Saturation Joystick", &JOYSTICK_AXES, &JOYSTICK_AXES_ENGLISH),
        gamepad_deadzone: per_input("ui_DeadzoneXI", "Deadzone Gamepad", &GAMEPAD_AXES, &GAMEPAD_AXES_ENGLISH),
        gamepad_sensitivity: or_english("ui_GamePadSensitivity", "GamePad Sensitvity"),
        mouse_sensitivity: or_english("pause_OptionsMouseSensitivity", "Mouse Sensitivity"),
        mouse_ads_percent: or_english("pause_OptionsMouseADSSensitivity", "Mouse Sensitivity - ADS - %"),
        mouse_zoom_scaling_enabled: or_english(
            "pause_OptionsMouseADSSensitivityZoomMultiplierToggle",
            "Mouse Sensitivity - ADS - Zoom Scaling Enabled",
        ),
        mouse_zoom_scaling_percent: or_english("pause_OptionsMouseADSSensitivityZoomMultiplier", "Mouse Sensitivity - ADS - Zoom Scaling %"),
        mouse_acceleration: or_english("pause_OptionsMouseAcceleration", "Mouse Acceleration"),
        mouse_smoothing: or_english("pause_OptionsMouseSmoothing", "Mouse Smoothing"),
    }
}

// ---------------------------------------------------------------------------
// Changes
// ---------------------------------------------------------------------------

/// The `attributes.xml` settings BindSight edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedAttribute {
    /// "Mouse Sensitivity", display 1 – 100.
    MouseSensitivity,
    /// "Mouse Sensitivity - ADS - %", display 0 – 200 (range unverified).
    AdsPercent,
    /// "Mouse Sensitivity - ADS - Zoom Scaling Enabled", 0 / 1.
    ZoomScalingEnabled,
    /// "Mouse Sensitivity - ADS - Zoom Scaling %", display 0 – 200 (range
    /// unverified).
    ZoomScalingPercent,
    /// "GamePad Sensitvity", 0 – 2 in 0.01 steps (range unverified).
    GamepadSensitivity,
}

impl ManagedAttribute {
    pub fn attr_name(self) -> &'static str {
        match self {
            ManagedAttribute::MouseSensitivity => "MouseSensitivity",
            ManagedAttribute::AdsPercent => "ADSMouseSensitivity",
            ManagedAttribute::ZoomScalingEnabled => "ZoomSensitivityMultiplierToggle",
            ManagedAttribute::ZoomScalingPercent => "ZoomSensitivityMultiplier",
            ManagedAttribute::GamepadSensitivity => "Sensitivity",
        }
    }

    /// The stored text for a display value; out of range or not finite is
    /// an error.
    fn stored(self, display: f32) -> Result<String, String> {
        let v = display as f64;
        if !v.is_finite() {
            return Err(format!("{self:?}: not a number"));
        }
        let (rounded, min, max) = match self {
            ManagedAttribute::GamepadSensitivity => (round_step(v, 100.0), 0.0, 2.0),
            ManagedAttribute::MouseSensitivity => (v.round(), 1.0, 100.0),
            ManagedAttribute::ZoomScalingEnabled => (v.round(), 0.0, 1.0),
            ManagedAttribute::AdsPercent | ManagedAttribute::ZoomScalingPercent => (v.round(), 0.0, 200.0),
        };
        if !(min..=max).contains(&rounded) {
            return Err(format!("{self:?}: {display} is out of range"));
        }
        let stored = match self {
            ManagedAttribute::MouseSensitivity => 5.0 + (rounded - 1.0) * 35.0 / 99.0,
            ManagedAttribute::AdsPercent | ManagedAttribute::ZoomScalingPercent => rounded / 100.0,
            ManagedAttribute::ZoomScalingEnabled | ManagedAttribute::GamepadSensitivity => rounded,
        };
        Ok(format_attributes_float(stored))
    }

    /// A stored text in display units (`ZoomScalingEnabled`: 1 or 0).
    fn display(self, stored: &str) -> Option<f32> {
        let s: f64 = stored.trim().parse().ok().filter(|v: &f64| v.is_finite())?;
        let d = match self {
            ManagedAttribute::MouseSensitivity => (1.0 + (s - 5.0) * 99.0 / 35.0).round(),
            ManagedAttribute::AdsPercent | ManagedAttribute::ZoomScalingPercent => (s * 100.0).round(),
            ManagedAttribute::ZoomScalingEnabled => {
                if s != 0.0 {
                    1.0
                } else {
                    0.0
                }
            }
            ManagedAttribute::GamepadSensitivity => round_step(s, 100.0),
        };
        Some(d as f32)
    }
}

/// A sensitivity curve to set on a node.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CurveValue {
    /// 0.1 – 3.0, rounded to 0.1 (the game's slider).
    Exponent { value: f32 },
    /// A custom curve: `[in, out]` in 0..1, `[0, 0]` and `[1, 1]` included,
    /// at most [`MAX_POINTS`]; written sorted by `in`.
    Points { points: Vec<[f32; 2]> },
    /// Remove the node's own curve ("Set Default").
    Default,
}

/// One edit of the device settings, as the GUI sends it (display units).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConfigChange {
    /// A node's curve. On a node with children an exponent or custom curve
    /// removes every descendant's own curve (what the game does).
    Curve { kind: DeviceKind, instance: u32, node: String, value: CurveValue },
    /// A node's inversion; `None` removes it ("Set Default"). Set on a node
    /// with children (mining) it removes every descendant's own inversion.
    Invert { kind: DeviceKind, instance: u32, node: String, value: Option<bool> },
    /// `device` is the raw `<deviceoptions>` name; 0 – 1.
    Deadzone { device: String, input: String, value: f32 },
    /// Joysticks only; 0 – 1.
    Saturation { device: String, input: String, value: f32 },
    MouseAcceleration { value: f32 },
    MouseSmoothing { value: f32 },
    Attribute { setting: ManagedAttribute, value: f32 },
}

impl ConfigChange {
    fn is_attribute(&self) -> bool {
        matches!(self, ConfigChange::Attribute { .. })
    }

    /// One short line for the app log.
    fn describe(&self) -> String {
        let slot = |kind: &DeviceKind, instance: &u32| format!("{}{instance}", kind.token_prefix());
        match self {
            ConfigChange::Curve { kind, instance, node, value } => {
                let v = match value {
                    CurveValue::Exponent { value } => format!("exponent {value}"),
                    CurveValue::Points { points } => format!("{} points", points.len()),
                    CurveValue::Default => "default".to_string(),
                };
                format!("curve {}/{node}={v}", slot(kind, instance))
            }
            ConfigChange::Invert { kind, instance, node, value } => {
                let v = value.map_or("default", |b| if b { "on" } else { "off" });
                format!("invert {}/{node}={v}", slot(kind, instance))
            }
            ConfigChange::Deadzone { device, input, value } => format!("deadzone {}/{input}={value}", device.trim()),
            ConfigChange::Saturation { device, input, value } => format!("saturation {}/{input}={value}", device.trim()),
            ConfigChange::MouseAcceleration { value } => format!("mouse acceleration={value}"),
            ConfigChange::MouseSmoothing { value } => format!("mouse smoothing={value}"),
            ConfigChange::Attribute { setting, value } => format!("{}={value}", setting.attr_name()),
        }
    }
}

/// A low-level edit of one node element.
#[derive(Debug, Clone, PartialEq)]
enum NodeOp {
    SetExponent(String),
    SetPoints(Vec<(String, String)>),
    ClearCurve,
    SetInvert(String),
    ClearInvert,
}

impl NodeOp {
    fn is_clear(&self) -> bool {
        matches!(self, NodeOp::ClearCurve | NodeOp::ClearInvert)
    }
}

/// A low-level edit of the file, what a [`ConfigChange`] breaks down into.
#[derive(Debug, Clone, PartialEq)]
enum Edit {
    Node { kind: DeviceKind, instance: u32, node: String, op: NodeOp },
    DeviceOption { device: String, input: String, attr: &'static str, value: String },
    /// Remove `attr` from every `<option input>` of the device; an option
    /// left with nothing but its input goes.
    RemoveDeviceOption { device: String, input: String, attr: &'static str },
}

/// An element name as the game's trees use them.
fn is_identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        && s.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
}

/// A device name that can be written into an attribute as it is.
fn is_writable_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_DEVICE_NAME && !s.chars().any(|c| matches!(c, '"' | '<' | '>' | '&') || c.is_control())
}

/// Every node name below `node`, pre-order.
fn descendants(node: &OptionNode) -> Vec<String> {
    let mut out = Vec::new();
    for child in &node.children {
        out.push(child.name.clone());
        out.extend(descendants(child));
    }
    out
}

/// Node name -> pre-order position in the tree (the order the game writes
/// the children of an `<options>` element in).
fn tree_order(tree: &OptionTree) -> HashMap<String, usize> {
    fn walk(nodes: &[OptionNode], out: &mut HashMap<String, usize>) {
        for n in nodes {
            let next = out.len();
            out.entry(n.name.clone()).or_insert(next);
            walk(&n.children, out);
        }
    }
    let mut out = HashMap::new();
    walk(&tree.nodes, &mut out);
    out
}

fn find_tree_node<'a>(nodes: &'a [OptionNode], name: &str) -> Option<&'a OptionNode> {
    nodes.iter().find_map(|n| if n.name == name { Some(n) } else { find_tree_node(&n.children, name) })
}

/// Check that `<options type=kind instance>` exists exactly once in `current`.
fn require_slot(current: &DeviceConfig, kind: DeviceKind, instance: u32) -> Result<(), String> {
    match current.options.iter().filter(|o| o.kind == kind && o.instance == instance).count() {
        1 => Ok(()),
        0 => Err(format!("no {}{instance} settings in the bindings file", kind.token_prefix())),
        _ => Err(format!("more than one {}{instance} settings element in the bindings file", kind.token_prefix())),
    }
}

/// A display value in 0..1 (0.01 steps), as stored with `scale`.
fn unit_value(value: f32, scale: f64, what: &str) -> Result<String, String> {
    let v = value as f64;
    if !v.is_finite() || !(0.0..=1.0).contains(&round_step(v, 100.0)) {
        return Err(format!("{what}: {value} is out of range"));
    }
    Ok(stored_scaled(v, scale))
}

/// Validate one change against the trees and the file as it stands
/// (`current`, earlier changes of the batch applied) and break it down into
/// edits.
fn plan(change: &ConfigChange, trees: &[OptionTree], current: &DeviceConfig) -> Result<Vec<Edit>, String> {
    let node_of = |kind: DeviceKind, instance: u32, name: &str| -> Result<&OptionNode, String> {
        let tree = trees.iter().find(|t| t.kind == kind).ok_or_else(|| format!("no option tree for {kind:?}"))?;
        if !is_identifier(name) {
            return Err(format!("invalid setting {name:?}"));
        }
        let node = find_tree_node(&tree.nodes, name).ok_or_else(|| format!("unknown {kind:?} setting {name:?}"))?;
        require_slot(current, kind, instance)?;
        Ok(node)
    };
    let edit = |kind, instance, node: &str, op| Edit::Node { kind, instance, node: node.to_string(), op };
    match change {
        ConfigChange::Curve { kind, instance, node, value } => {
            let n = node_of(*kind, *instance, node)?;
            let op = match value {
                CurveValue::Default => return Ok(vec![edit(*kind, *instance, node, NodeOp::ClearCurve)]),
                CurveValue::Exponent { value } => {
                    let v = round_step(*value as f64, 10.0);
                    if !value.is_finite() || !(0.1 - 1e-9..=3.0 + 1e-9).contains(&v) {
                        return Err(format!("exponent {value} is out of range"));
                    }
                    NodeOp::SetExponent(format_actionmaps_float(v as f32))
                }
                CurveValue::Points { points } => NodeOp::SetPoints(curve_points(points)?),
            };
            if n.show_curve != 1 {
                return Err(format!("{node:?} has no curve"));
            }
            let mut edits = vec![edit(*kind, *instance, node, op)];
            edits.extend(descendants(n).iter().map(|d| edit(*kind, *instance, d, NodeOp::ClearCurve)));
            Ok(edits)
        }
        ConfigChange::Invert { kind, instance, node, value } => {
            let n = node_of(*kind, *instance, node)?;
            let Some(on) = value else {
                return Ok(vec![edit(*kind, *instance, node, NodeOp::ClearInvert)]);
            };
            if n.show_invert != 1 {
                return Err(format!("{node:?} has no inversion"));
            }
            let mut edits = vec![edit(*kind, *instance, node, NodeOp::SetInvert(if *on { "1" } else { "0" }.to_string()))];
            edits.extend(descendants(n).iter().map(|d| edit(*kind, *instance, d, NodeOp::ClearInvert)));
            Ok(edits)
        }
        ConfigChange::Deadzone { device, input, value } | ConfigChange::Saturation { device, input, value } => {
            let deadzone = matches!(change, ConfigChange::Deadzone { .. });
            if !is_writable_name(device) {
                return Err(format!("invalid device name {device:?}"));
            }
            let known = device == GAMEPAD_DEVICE
                || current.device_options.iter().any(|d| d.name == *device)
                || current.options.iter().any(|o| o.kind == DeviceKind::Joystick && o.product.as_deref() == Some(device.as_str()));
            if !known {
                return Err(format!("unknown device {:?}", device.trim()));
            }
            let (inputs, scale): (&[&str], f64) = match device.as_str() {
                MOUSE_DEVICE => return Err("the mouse has no deadzone or saturation".into()),
                GAMEPAD_DEVICE if !deadzone => return Err("the gamepad has no saturation".into()),
                GAMEPAD_DEVICE => (&GAMEPAD_AXES, GAMEPAD_SCALE),
                _ => (&JOYSTICK_AXES, JOYSTICK_SCALE),
            };
            if !inputs.contains(&input.as_str()) {
                return Err(format!("invalid input {input:?}"));
            }
            let what = if deadzone { "deadzone" } else { "saturation" };
            Ok(vec![Edit::DeviceOption { device: device.clone(), input: input.clone(), attr: what, value: unit_value(*value, scale, what)? }])
        }
        ConfigChange::MouseAcceleration { value } => Ok(vec![Edit::DeviceOption {
            device: MOUSE_DEVICE.into(),
            input: MOUSE_ACCELERATION_INPUT.into(),
            attr: "acceleration",
            value: unit_value(*value, ACCELERATION_SCALE, "mouse acceleration")?,
        }]),
        ConfigChange::MouseSmoothing { value } => Ok(vec![Edit::DeviceOption {
            device: MOUSE_DEVICE.into(),
            input: MOUSE_SMOOTHING_INPUT.into(),
            attr: "saturation",
            value: unit_value(*value, SMOOTHING_SCALE, "mouse smoothing")?,
        }]),
        ConfigChange::Attribute { .. } => Err("a game setting is not part of the bindings file".into()),
    }
}

/// Validate a custom curve and format it, sorted by `in` (stable, so points
/// with the same `in` keep their order).
fn curve_points(points: &[[f32; 2]]) -> Result<Vec<(String, String)>, String> {
    if points.len() < 2 || points.len() > MAX_POINTS {
        return Err(format!("a curve has 2 to {MAX_POINTS} points"));
    }
    if points.iter().flatten().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
        return Err("curve points must lie within 0..1".into());
    }
    if !points.contains(&[0.0, 0.0]) || !points.contains(&[1.0, 1.0]) {
        return Err("a curve runs from 0/0 to 1/1".into());
    }
    let mut sorted = points.to_vec();
    sorted.sort_by(|a, b| a[0].total_cmp(&b[0]));
    Ok(sorted.iter().map(|[i, o]| (format_actionmaps_float(*i), format_actionmaps_float(*o))).collect())
}

// ---------------------------------------------------------------------------
// The model side of an edit (what the rewrite must end up as)
// ---------------------------------------------------------------------------

fn set_or_push(attrs: &mut Vec<(String, String)>, name: &str, value: &str) {
    match attrs.iter_mut().find(|(k, _)| k == name) {
        Some(slot) => slot.1 = value.to_string(),
        None => attrs.push((name.to_string(), value.to_string())),
    }
}

fn apply_node_op(node: &mut NodeEntry, op: &NodeOp) {
    let remove = |attrs: &mut Vec<(String, String)>, name: &str| attrs.retain(|(k, _)| k != name);
    match op {
        NodeOp::SetExponent(v) => {
            set_or_push(&mut node.attrs, "exponent", v);
            clear_points(node);
        }
        NodeOp::SetPoints(p) => {
            remove(&mut node.attrs, "exponent");
            node.points = Some(p.clone());
        }
        NodeOp::ClearCurve => {
            remove(&mut node.attrs, "exponent");
            clear_points(node);
        }
        NodeOp::SetInvert(v) => {
            if let Some(slot) = node.attrs.iter_mut().find(|(k, _)| k == "invert") {
                slot.1 = v.clone();
            } else {
                // The game writes `invert` before `exponent`.
                let at = node.attrs.iter().position(|(k, _)| k == "exponent").unwrap_or(node.attrs.len());
                node.attrs.insert(at, ("invert".to_string(), v.clone()));
            }
        }
        NodeOp::ClearInvert => remove(&mut node.attrs, "invert"),
    }
}

/// The curve's points go; a curve element holding other content (a
/// comment) stays without them.
fn clear_points(node: &mut NodeEntry) {
    node.points = (node.curve_other > 0).then(Vec::new);
}

fn node_is_empty(node: &NodeEntry) -> bool {
    node.attrs.is_empty() && node.points.is_none() && node.other_children == 0
}

fn apply_edit_model(config: &mut DeviceConfig, edit: &Edit, orders: &HashMap<DeviceKind, HashMap<String, usize>>) {
    match edit {
        Edit::Node { kind, instance, node, op } => {
            let Some(block) = config.options.iter_mut().find(|o| o.kind == *kind && o.instance == *instance) else { return };
            match block.nodes.iter().position(|n| n.name == *node) {
                Some(i) => {
                    apply_node_op(&mut block.nodes[i], op);
                    if node_is_empty(&block.nodes[i]) {
                        block.nodes.remove(i);
                    }
                }
                None if op.is_clear() => {}
                None => {
                    let mut entry = NodeEntry { name: node.clone(), attrs: Vec::new(), points: None, other_children: 0, curve_other: 0 };
                    apply_node_op(&mut entry, op);
                    let at = orders
                        .get(kind)
                        .and_then(|order| insert_position(block.nodes.iter().map(|n| n.name.as_str()), order, node))
                        .unwrap_or(block.nodes.len());
                    block.nodes.insert(at, entry);
                }
            }
        }
        Edit::DeviceOption { device, input, attr: attr_name, value } => {
            let Some(d) = config.device_options.iter_mut().find(|d| d.name == *device) else {
                config.device_options.push(DeviceOptionsEntry {
                    name: device.clone(),
                    entries: vec![OptionEntry { input: input.clone(), attrs: vec![(attr_name.to_string(), value.clone())] }],
                    other_children: 0,
                });
                return;
            };
            let mut hit = false;
            for entry in d.entries.iter_mut().filter(|e| e.input == *input) {
                if let Some(slot) = entry.attrs.iter_mut().find(|(k, _)| k == attr_name) {
                    slot.1 = value.clone();
                    hit = true;
                }
            }
            if !hit {
                d.entries.push(OptionEntry { input: input.clone(), attrs: vec![(attr_name.to_string(), value.clone())] });
            }
        }
        Edit::RemoveDeviceOption { device, input, attr: attr_name } => {
            let Some(at) = config.device_options.iter().position(|d| d.name == *device) else { return };
            let d = &mut config.device_options[at];
            let mut hit = false;
            d.entries.retain_mut(|e| {
                if e.input == *input && e.attrs.iter().any(|(k, _)| k == attr_name) {
                    hit = true;
                    e.attrs.retain(|(k, _)| k != attr_name);
                    !e.attrs.is_empty()
                } else {
                    true
                }
            });
            // A device element the removal left empty goes.
            if hit && d.entries.is_empty() && d.other_children == 0 {
                config.device_options.remove(at);
            }
        }
    }
}

/// Where a new node goes among existing children (`names`, in order):
/// before the first one that comes later in the tree; `None` = at the end.
/// Children the tree does not know are passed over.
fn insert_position<'a>(names: impl Iterator<Item = &'a str>, order: &HashMap<String, usize>, new: &str) -> Option<usize> {
    let mine = *order.get(new)?;
    names.enumerate().find(|(_, n)| order.get(*n).is_some_and(|&i| i > mine)).map(|(i, _)| i)
}

// ---------------------------------------------------------------------------
// The text side of an edit
// ---------------------------------------------------------------------------

/// One element in the text (byte offsets).
#[derive(Debug, Clone)]
struct Span {
    name: String,
    start: usize,
    /// The index of the start tag's `>`.
    head_end: usize,
    /// Start of the end tag (`== end` for a self-closing element).
    close_start: usize,
    end: usize,
    self_closing: bool,
}

impl Span {
    fn head<'a>(&self, xml: &'a str) -> &'a str {
        &xml[self.start..=self.head_end]
    }

    /// The text between the tags; empty for a self-closing element.
    fn inner<'a>(&self, xml: &'a str) -> &'a str {
        if self.self_closing {
            ""
        } else {
            &xml[self.head_end + 1..self.close_start]
        }
    }
}

fn tag_name(head: &str) -> String {
    head[1..].split(|c: char| c.is_whitespace() || c == '/' || c == '>').next().unwrap_or("").to_string()
}

/// The elements directly inside `masked[from..to]` (see `mask_markup`),
/// nested ones skipped, end tags matched by depth.
fn children(masked: &str, from: usize, to: usize) -> Result<Vec<Span>, String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    // The depth-0 element being read: name, start, head end.
    let mut open: Option<(String, usize, usize)> = None;
    let mut pos = from;
    while let Some(rel) = masked.get(pos..to).and_then(|s| s.find('<')) {
        let start = pos + rel;
        let end = tag_end(masked, start, to).ok_or("unterminated tag")?;
        let head = &masked[start..=end];
        pos = end + 1;
        if head.starts_with("</") {
            depth = depth.checked_sub(1).ok_or("unbalanced end tag")?;
            if depth == 0 {
                if let Some((name, s, he)) = open.take() {
                    out.push(Span { name, start: s, head_end: he, close_start: start, end: end + 1, self_closing: false });
                }
            }
            continue;
        }
        // A DOCTYPE or the like (comments, CDATA and PIs are masked).
        if head.starts_with("<!") || head.starts_with("<?") {
            continue;
        }
        let self_closing = head.ends_with("/>");
        if depth == 0 {
            if self_closing {
                out.push(Span { name: tag_name(head), start, head_end: end, close_start: end + 1, end: end + 1, self_closing: true });
            } else {
                open = Some((tag_name(head), start, end));
                depth = 1;
            }
        } else if !self_closing {
            depth += 1;
        }
    }
    if depth != 0 {
        return Err("unclosed element".into());
    }
    Ok(out)
}

/// Every `<tag …>` element anywhere in the text (tags that never nest:
/// `options`, `deviceoptions`, `Attr`, the root).
fn find_all(xml: &str, masked: &str, tag: &str) -> Result<Vec<Span>, String> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(rel) = masked[pos..].find(&open) {
        let start = pos + rel;
        let after = masked[start + open.len()..].chars().next();
        if !matches!(after, Some(c) if c.is_whitespace() || c == '/' || c == '>') {
            pos = start + open.len();
            continue;
        }
        let head_end = tag_end(masked, start, masked.len()).ok_or_else(|| format!("unterminated <{tag}> tag"))?;
        let self_closing = xml[start..=head_end].ends_with("/>");
        let (close_start, end) = if self_closing {
            (head_end + 1, head_end + 1)
        } else {
            let c = masked[head_end..].find(&close).ok_or_else(|| format!("<{tag}> without {close}"))? + head_end;
            (c, c + close.len())
        };
        out.push(Span { name: tag.to_string(), start, head_end, close_start, end, self_closing });
        pos = end;
    }
    Ok(out)
}

/// Remove `s[start..end]` with its indentation and the line break after it.
fn remove_line(s: &mut String, start: usize, end: usize, eol: &str) {
    let line_start = start - indent_before(s, start).len();
    let line_end = if s[end..].starts_with(eol) { end + eol.len() } else { end };
    s.replace_range(line_start..line_end, "");
}

/// Whether a tag head has any attribute.
fn has_attributes(head: &str) -> bool {
    let rest = head[1..].trim_start_matches(|c: char| !(c.is_whitespace() || c == '/' || c == '>'));
    rest.chars().any(|c| !(c.is_whitespace() || c == '/' || c == '>'))
}

/// A tag head without its closing `>` or `/>`, trailing whitespace dropped.
fn open_part(head: &str) -> &str {
    head.strip_suffix("/>").or_else(|| head.strip_suffix('>')).unwrap_or(head).trim_end()
}

/// The head with ` name="value"` inserted before the attribute `before`
/// (the game's attribute order), or appended when there is none.
fn insert_attr_before(head: &str, name: &str, value: &str, before: &str) -> String {
    let Some(s) = find_attr(head, before) else {
        return insert_attr(head, name, value);
    };
    let bytes = head.as_bytes();
    let mut i = s.value_start - 1;
    while i > 0 && (bytes[i - 1].is_ascii_whitespace() || bytes[i - 1] == b'=') {
        i -= 1;
    }
    match i.checked_sub(before.len()).filter(|&at| &head[at..i] == before) {
        Some(at) => format!("{}{name}=\"{value}\" {}", &head[..at], &head[at..]),
        None => insert_attr(head, name, value),
    }
}

/// A `<nonlinearity_curve>` block whose first line sits at `indent`.
fn curve_text(points: &[(String, String)], indent: &str, layout: &Layout) -> String {
    let (eol, unit) = (layout.eol, layout.unit);
    let mut out = format!("<nonlinearity_curve>{eol}");
    for (i, o) in points {
        out.push_str(&format!("{indent}{unit}<point in=\"{i}\" out=\"{o}\"/>{eol}"));
    }
    out.push_str(&format!("{indent}</nonlinearity_curve>"));
    out
}

/// The `<options type instance>` element of a slot (exactly one).
fn find_slot(xml: &str, masked: &str, kind: DeviceKind, instance: u32) -> Result<Span, String> {
    let mut found = find_all(xml, masked, "options")?.into_iter().filter(|b| {
        let head = b.head(xml);
        attr(head, "type") == Some(kind_attr(kind)) && attr(head, "instance").and_then(|i| i.parse::<u32>().ok()) == Some(instance)
    });
    let first = found.next().ok_or_else(|| format!("no {}{instance} settings in the bindings file", kind.token_prefix()))?;
    if found.next().is_some() {
        return Err(format!("more than one {}{instance} settings element in the bindings file", kind.token_prefix()));
    }
    Ok(first)
}

/// Insert `el` as a new child of `parent` (a self-closing one is opened):
/// before the line of `before` if given, else after the last child, else as
/// the only one.
fn insert_child(xml: &mut String, parent: &Span, kids: &[Span], before: Option<&Span>, el: &str, layout: &Layout) {
    let (eol, unit) = (layout.eol, layout.unit);
    let pi = indent_before(xml, parent.start).to_string();
    let ci = kids.first().map(|k| indent_before(xml, k.start).to_string()).unwrap_or_else(|| format!("{pi}{unit}"));
    if let Some(next) = before {
        let line_start = next.start - indent_before(xml, next.start).len();
        xml.insert_str(line_start, &format!("{ci}{el}{eol}"));
    } else if let Some(last) = kids.last() {
        xml.insert_str(last.end, &format!("{eol}{ci}{el}"));
    } else if parent.self_closing {
        let open = open_part(parent.head(xml)).to_string();
        let text = format!("{open}>{eol}{ci}{el}{eol}{pi}</{}>", parent.name);
        xml.replace_range(parent.start..parent.end, &text);
    } else if parent.inner(xml).trim().is_empty() {
        xml.replace_range(parent.head_end + 1..parent.close_start, &format!("{eol}{ci}{el}{eol}{pi}"));
    } else {
        let indent = indent_before(xml, parent.close_start).to_string();
        xml.insert_str(parent.close_start, &format!("{unit}{el}{eol}{indent}"));
    }
}

/// Apply one node edit to the text.
fn edit_node(xml: &str, kind: DeviceKind, instance: u32, name: &str, op: &NodeOp, order: &HashMap<String, usize>) -> Result<String, String> {
    let layout = layout_of(xml);
    let (eol, unit) = (layout.eol, layout.unit);
    let masked = mask_markup(xml);
    let block = find_slot(xml, &masked, kind, instance)?;
    let kids = if block.self_closing { Vec::new() } else { children(&masked, block.head_end + 1, block.close_start)? };
    let mut found = kids.iter().filter(|c| c.name == name);
    let target = found.next().cloned();
    if found.next().is_some() {
        return Err(format!("more than one {name:?} element in the {}{instance} settings", kind.token_prefix()));
    }
    let mut out = xml.to_string();
    match target {
        None => {
            let ci = kids.first().map(|k| indent_before(xml, k.start).to_string()).unwrap_or_else(|| format!("{}{unit}", indent_before(xml, block.start)));
            let el = match op {
                NodeOp::SetExponent(v) => format!("<{name} exponent=\"{v}\"/>"),
                NodeOp::SetInvert(v) => format!("<{name} invert=\"{v}\"/>"),
                NodeOp::SetPoints(p) => {
                    let curve = curve_text(p, &format!("{ci}{unit}"), &layout);
                    format!("<{name}>{eol}{ci}{unit}{curve}{eol}{ci}</{name}>")
                }
                // Nothing to remove.
                NodeOp::ClearCurve | NodeOp::ClearInvert => return Ok(out),
            };
            let before = insert_position(kids.iter().map(|k| k.name.as_str()), order, name).map(|i| &kids[i]);
            insert_child(&mut out, &block, &kids, before, &el, &layout);
        }
        Some(c) => match rebuild_node(xml, &c, op, &layout)? {
            Some(text) => out.replace_range(c.start..c.end, &text),
            None => remove_line(&mut out, c.start, c.end, eol),
        },
    }
    // An `<options>` element left without children is written self-closing,
    // the way the game writes it.
    let masked = mask_markup(&out);
    let block = find_slot(&out, &masked, kind, instance)?;
    if !block.self_closing && block.inner(&out).trim().is_empty() {
        let text = format!("{}/>", open_part(block.head(&out)));
        out.replace_range(block.start..block.end, &text);
    }
    Ok(out)
}

/// The new text of an existing node element after `op`; `None` when it is
/// left empty (no attributes, nothing inside) and goes.
fn rebuild_node(xml: &str, c: &Span, op: &NodeOp, layout: &Layout) -> Result<Option<String>, String> {
    let indent = indent_before(xml, c.start).to_string();
    // Edited in its open form `<name …>`.
    let mut head = format!("{}>", open_part(c.head(xml)));
    let mut inner = c.inner(xml).to_string();
    let drop_attr = |head: &str, name: &str| remove_attr(head, name).unwrap_or_else(|| head.to_string());
    match op {
        NodeOp::SetExponent(v) => {
            head = set_attr(&head, "exponent", v).unwrap_or_else(|| insert_attr(&head, "exponent", v));
            inner = set_curve(&inner, None, &indent, layout)?;
        }
        NodeOp::SetPoints(p) => {
            head = drop_attr(&head, "exponent");
            inner = set_curve(&inner, Some(p), &indent, layout)?;
        }
        NodeOp::ClearCurve => {
            head = drop_attr(&head, "exponent");
            inner = set_curve(&inner, None, &indent, layout)?;
        }
        NodeOp::SetInvert(v) => {
            head = set_attr(&head, "invert", v).unwrap_or_else(|| insert_attr_before(&head, "invert", v, "exponent"));
        }
        NodeOp::ClearInvert => head = drop_attr(&head, "invert"),
    }
    let empty_inside = inner.trim().is_empty();
    if empty_inside && !has_attributes(&head) {
        return Ok(None);
    }
    if empty_inside {
        return Ok(Some(format!("{}/>", open_part(&head))));
    }
    Ok(Some(format!("{head}{inner}</{}>", c.name)))
}

/// A node element's content with the points of its `<nonlinearity_curve>`
/// children removed, `points` put into the first one (or into a new one
/// appended when there is none). A curve element left without content goes;
/// one holding something else (a comment) stays with it. `indent` is the
/// node element's own indentation.
fn set_curve(inner: &str, points: Option<&[(String, String)]>, indent: &str, layout: &Layout) -> Result<String, String> {
    let (eol, unit) = (layout.eol, layout.unit);
    let masked = mask_markup(inner);
    let curves: Vec<Span> = children(&masked, 0, inner.len())?.into_iter().filter(|k| k.name == "nonlinearity_curve").collect();
    let mut out = inner.to_string();
    // Back to front, so the earlier offsets stay valid.
    for (n, c) in curves.iter().enumerate().rev() {
        let fill = if n == 0 { points } else { None };
        match rebuild_curve(inner, c, fill, layout)? {
            Some(text) => out.replace_range(c.start..c.end, &text),
            None => remove_line(&mut out, c.start, c.end, eol),
        }
    }
    let Some(points) = points else { return Ok(out) };
    if curves.is_empty() {
        let ci = format!("{indent}{unit}");
        let curve = curve_text(points, &ci, layout);
        if out.trim().is_empty() {
            out = format!("{eol}{ci}{curve}{eol}{indent}");
        } else {
            let p = out.trim_end().len();
            out.insert_str(p, &format!("{eol}{ci}{curve}"));
        }
    }
    Ok(out)
}

/// The new text of a `<nonlinearity_curve>` element `c` of `inner`: its
/// `<point>` children removed, `points` in the first one's place (else
/// before the end tag). `None` when nothing is left inside and no points
/// are to go in; a curve holding nothing else is written anew.
fn rebuild_curve(inner: &str, c: &Span, points: Option<&[(String, String)]>, layout: &Layout) -> Result<Option<String>, String> {
    let (eol, unit) = (layout.eol, layout.unit);
    let ci = indent_before(inner, c.start).to_string();
    let mut body = c.inner(inner).to_string();
    let masked = mask_markup(&body);
    let olds: Vec<Span> = children(&masked, 0, body.len())?.into_iter().filter(|k| k.name == "point").collect();
    let first_line = olds.first().map(|p| p.start - indent_before(&body, p.start).len());
    for p in olds.iter().rev() {
        remove_line(&mut body, p.start, p.end, eol);
    }
    if body.trim().is_empty() {
        return Ok(points.map(|p| curve_text(p, &ci, layout)));
    }
    if let Some(points) = points {
        let lines: String = points.iter().map(|(i, o)| format!("{ci}{unit}<point in=\"{i}\" out=\"{o}\"/>{eol}")).collect();
        match first_line {
            Some(at) => body.insert_str(at, &lines),
            None => {
                let at = body.trim_end().len();
                let lines = lines.strip_suffix(eol).unwrap_or(&lines);
                body.insert_str(at, &format!("{eol}{lines}"));
            }
        }
    }
    Ok(Some(format!("{}{body}</{}>", c.head(inner), c.name)))
}

/// Apply one `<deviceoptions>` edit to the text.
fn edit_device_option(xml: &str, device: &str, input: &str, attr_name: &str, value: &str) -> Result<String, String> {
    let layout = layout_of(xml);
    let (eol, unit) = (layout.eol, layout.unit);
    let masked = mask_markup(xml);
    let all = find_all(xml, &masked, "deviceoptions")?;
    let mut matching = all.iter().filter(|d| attr(d.head(xml), "name") == Some(device));
    let target = matching.next();
    if matching.next().is_some() {
        return Err(format!("more than one device settings element for {:?}", device.trim()));
    }
    let option = format!("<option input=\"{input}\" {attr_name}=\"{value}\"/>");
    let mut out = xml.to_string();
    let Some(d) = target else {
        // A new element after the last one of its kind, else before the
        // first `<options>`.
        if let Some(last) = all.last() {
            let di = indent_before(xml, last.start).to_string();
            let el = format!("<deviceoptions name=\"{device}\">{eol}{di}{unit}{option}{eol}{di}</deviceoptions>");
            out.insert_str(last.end, &format!("{eol}{di}{el}"));
        } else {
            let first = find_all(xml, &masked, "options")?.into_iter().next().ok_or("no place for device settings in the bindings file")?;
            let di = indent_before(xml, first.start).to_string();
            let el = format!("<deviceoptions name=\"{device}\">{eol}{di}{unit}{option}{eol}{di}</deviceoptions>");
            out.insert_str(first.start - di.len(), &format!("{di}{el}{eol}"));
        }
        return Ok(out);
    };
    let kids = if d.self_closing { Vec::new() } else { children(&masked, d.head_end + 1, d.close_start)? };
    let options: Vec<&Span> = kids.iter().filter(|k| k.name == "option").collect();
    let targets: Vec<&&Span> = options
        .iter()
        .filter(|o| {
            let head = o.head(xml);
            attr(head, "input") == Some(input) && find_attr(head, attr_name).is_some()
        })
        .collect();
    if targets.is_empty() {
        let after = options.last().copied().or(kids.last());
        match after {
            Some(last) => {
                let oi = indent_before(xml, last.start).to_string();
                out.insert_str(last.end, &format!("{eol}{oi}{option}"));
            }
            None => insert_child(&mut out, d, &[], None, &option, &layout),
        }
        return Ok(out);
    }
    for o in targets.iter().rev() {
        let head = o.head(xml);
        let new_head = set_attr(head, attr_name, value).ok_or("unreadable device setting")?;
        out.replace_range(o.start..=o.head_end, &new_head);
    }
    Ok(out)
}

/// Remove `attr_name` from every `<option input>` of the device's
/// `<deviceoptions>` element; an option left with nothing but its input
/// goes, and so does a device element left empty (with its line) — the game
/// was never seen writing a self-closing one, a missing one it handles. No
/// such element or option: nothing to do.
fn remove_device_option(xml: &str, device: &str, input: &str, attr_name: &str) -> Result<String, String> {
    let layout = layout_of(xml);
    let masked = mask_markup(xml);
    let all = find_all(xml, &masked, "deviceoptions")?;
    let mut matching = all.iter().filter(|d| attr(d.head(xml), "name") == Some(device));
    let Some(d) = matching.next() else { return Ok(xml.to_string()) };
    if matching.next().is_some() {
        return Err(format!("more than one device settings element for {:?}", device.trim()));
    }
    let kids = if d.self_closing { Vec::new() } else { children(&masked, d.head_end + 1, d.close_start)? };
    let targets: Vec<&Span> = kids
        .iter()
        .filter(|k| k.name == "option" && attr(k.head(xml), "input") == Some(input) && find_attr(k.head(xml), attr_name).is_some())
        .collect();
    if targets.is_empty() {
        return Ok(xml.to_string());
    }
    let mut out = xml.to_string();
    for o in targets.iter().rev() {
        let new_head = remove_attr(o.head(xml), attr_name).ok_or("unreadable device setting")?;
        let only_input = remove_attr(&new_head, "input").is_some_and(|h| !has_attributes(&h));
        if only_input && o.inner(xml).trim().is_empty() {
            remove_line(&mut out, o.start, o.end, layout.eol);
        } else {
            out.replace_range(o.start..=o.head_end, &new_head);
        }
    }
    // A device element left with nothing inside goes; one that still holds
    // anything (a comment too) stays.
    let masked = mask_markup(&out);
    let d = find_all(&out, &masked, "deviceoptions")?
        .into_iter()
        .find(|d| attr(d.head(&out), "name") == Some(device))
        .ok_or("device settings element lost")?;
    if !d.self_closing && d.inner(&out).trim().is_empty() {
        remove_line(&mut out, d.start, d.end, layout.eol);
    }
    Ok(out)
}

/// Apply the `actionmaps.xml` part of `changes` (everything but
/// [`ConfigChange::Attribute`]) to `xml` and return the new text. Every
/// change is validated first ([`plan`]: node names from `trees`, inputs from
/// the fixed lists, numbers in range, slots and devices from the file);
/// the result is parsed once more and must equal the parsed original with
/// the changes applied, its rebinds and joystick map untouched.
pub fn apply_config(xml: &str, changes: &[ConfigChange], trees: &[OptionTree]) -> Result<String, String> {
    let changes: Vec<&ConfigChange> = changes.iter().filter(|c| !c.is_attribute()).collect();
    if changes.is_empty() {
        return Err("nothing to change".into());
    }
    let before = parse_device_config(xml)?;
    let orders = tree_orders(trees);
    let mut expected = before.clone();
    let mut out = xml.to_string();
    for change in changes {
        for edit in plan(change, trees, &expected)? {
            out = edit_text(&out, &edit, &orders)?;
            apply_edit_model(&mut expected, &edit, &orders);
        }
    }
    check_rewrite(xml, &out, &expected)?;
    Ok(out)
}

/// Every tree's pre-order, by kind.
fn tree_orders(trees: &[OptionTree]) -> HashMap<DeviceKind, HashMap<String, usize>> {
    trees.iter().map(|t| (t.kind, tree_order(t))).collect()
}

/// Apply one edit to the text.
fn edit_text(xml: &str, edit: &Edit, orders: &HashMap<DeviceKind, HashMap<String, usize>>) -> Result<String, String> {
    match edit {
        Edit::Node { kind, instance, node, op } => {
            let order = orders.get(kind).ok_or_else(|| format!("no option tree for {kind:?}"))?;
            edit_node(xml, *kind, *instance, node, op, order)
        }
        Edit::DeviceOption { device, input, attr, value } => edit_device_option(xml, device, input, attr, value),
        Edit::RemoveDeviceOption { device, input, attr } => remove_device_option(xml, device, input, attr),
    }
}

/// The rewrite `out` of `xml` must parse to `expected`, its rebinds and
/// joystick map untouched.
fn check_rewrite(xml: &str, out: &str, expected: &DeviceConfig) -> Result<(), String> {
    let after = parse_device_config(out).map_err(|e| format!("rewrite produced unreadable XML: {e}"))?;
    if after != *expected {
        return Err("rewrite check failed: the device settings are not what was asked".into());
    }
    verify_bindings_untouched(xml, out)
}

/// One rebind as the check compares it: actionmap, action, input, attributes.
type RebindRow = (String, String, String, Vec<(String, String)>);

/// The rebinds and the joystick device map must survive a settings rewrite
/// unchanged.
fn verify_bindings_untouched(before: &str, after: &str) -> Result<(), String> {
    let (b, a) = (scdata::parse_actionmaps(before)?, scdata::parse_actionmaps(after).map_err(|e| format!("rewrite produced unreadable XML: {e}"))?);
    let rebinds = |f: &scdata::ActionMapsFile| -> Vec<RebindRow> {
        f.rebinds.iter().map(|r| (r.actionmap.clone(), r.action.clone(), r.input.clone(), r.attrs.clone())).collect()
    };
    let devices = |f: &scdata::ActionMapsFile| -> Vec<(u32, String)> { f.joysticks.iter().map(|j| (j.instance, j.product.clone())).collect() };
    if rebinds(&b) != rebinds(&a) || devices(&b) != devices(&a) {
        return Err("rewrite check failed: the bindings changed".into());
    }
    Ok(())
}

/// Apply the [`ConfigChange::Attribute`] part of `changes` to the text of an
/// `attributes.xml`: every `<Attr>` of a managed name gets the new value, a
/// missing one is inserted at its sorted position (byte order of the name);
/// nothing else changes. Checked against the intent like [`apply_config`].
pub fn apply_attributes(xml: &str, changes: &[ConfigChange]) -> Result<String, String> {
    let changes: Vec<(ManagedAttribute, String)> = changes
        .iter()
        .filter_map(|c| match c {
            ConfigChange::Attribute { setting, value } => Some(setting.stored(*value).map(|v| (*setting, v))),
            _ => None,
        })
        .collect::<Result<_, _>>()?;
    if changes.is_empty() {
        return Err("nothing to change".into());
    }
    let mut expected = attribute_entries(xml)?;
    let mut out = xml.to_string();
    for (setting, value) in &changes {
        let name = setting.attr_name();
        out = set_attribute_text(&out, name, value)?;
        if expected.iter().any(|(n, _)| n == name) {
            for entry in expected.iter_mut().filter(|(n, _)| n == name) {
                entry.1 = value.clone();
            }
        } else {
            let at = expected.iter().position(|(n, _)| n.as_str() > name).unwrap_or(expected.len());
            expected.insert(at, (name.to_string(), value.clone()));
        }
    }
    let after = attribute_entries(&out).map_err(|e| format!("rewrite produced unreadable XML: {e}"))?;
    if after != expected {
        return Err("rewrite check failed: the game settings are not what was asked".into());
    }
    Ok(out)
}

fn set_attribute_text(xml: &str, name: &str, value: &str) -> Result<String, String> {
    let layout = layout_of(xml);
    let eol = layout.eol;
    let masked = mask_markup(xml);
    let tags = find_all(xml, &masked, "Attr")?;
    let same: Vec<&Span> = tags.iter().filter(|t| attr(t.head(xml), "name") == Some(name)).collect();
    let mut out = xml.to_string();
    if !same.is_empty() {
        for t in same.iter().rev() {
            let head = t.head(xml);
            let new_head = set_attr(head, "value", value).unwrap_or_else(|| insert_attr(head, "value", value));
            out.replace_range(t.start..=t.head_end, &new_head);
        }
        return Ok(out);
    }
    let el = format!("<Attr name=\"{name}\" value=\"{value}\"/>");
    if let Some(next) = tags.iter().find(|t| attr(t.head(xml), "name").is_some_and(|n| n > name)) {
        let indent = indent_before(xml, next.start).to_string();
        out.insert_str(next.start - indent.len(), &format!("{indent}{el}{eol}"));
    } else if let Some(last) = tags.last() {
        let indent = indent_before(xml, last.start).to_string();
        out.insert_str(last.end, &format!("{eol}{indent}{el}"));
    } else {
        let root = find_all(xml, &masked, "Attributes")?.into_iter().next().ok_or("no <Attributes> element")?;
        insert_child(&mut out, &root, &[], None, &el, &layout);
    }
    Ok(out)
}

fn remove_attribute_text(xml: &str, name: &str) -> Result<String, String> {
    let eol = layout_of(xml).eol;
    let masked = mask_markup(xml);
    let mut out = xml.to_string();
    for t in find_all(xml, &masked, "Attr")?.iter().rev().filter(|t| attr(t.head(xml), "name") == Some(name)) {
        remove_line(&mut out, t.start, t.end, eol);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Apply a source's settings (`apply_source`)
// ---------------------------------------------------------------------------

/// One device of an apply: the source's slot and the live slot its
/// settings land on (joysticks may move, like their bindings).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SlotPair {
    pub kind: DeviceKind,
    pub source: u32,
    pub target: u32,
}

/// The `attributes.xml` settings that belong to a device: the mouse's four
/// for the keyboard, the sensitivity for the gamepad.
pub fn managed_attributes(kind: DeviceKind) -> &'static [ManagedAttribute] {
    match kind {
        DeviceKind::Keyboard => &[
            ManagedAttribute::MouseSensitivity,
            ManagedAttribute::AdsPercent,
            ManagedAttribute::ZoomScalingEnabled,
            ManagedAttribute::ZoomScalingPercent,
        ],
        DeviceKind::Gamepad => &[ManagedAttribute::GamepadSensitivity],
        DeviceKind::Joystick => &[],
    }
}

/// The `<deviceoptions>` values a device kind owns, as `(input, attribute)`.
fn device_option_slots(kind: DeviceKind) -> Vec<(&'static str, &'static str)> {
    match kind {
        DeviceKind::Joystick => JOYSTICK_AXES.iter().flat_map(|i| [(*i, "deadzone"), (*i, "saturation")]).collect(),
        DeviceKind::Gamepad => GAMEPAD_AXES.iter().map(|i| (*i, "deadzone")).collect(),
        DeviceKind::Keyboard => vec![(MOUSE_ACCELERATION_INPUT, "acceleration"), (MOUSE_SMOOTHING_INPUT, "saturation")],
    }
}

/// A stored number taken over from another file as it is written there:
/// a plain decimal literal (nothing that needs escaping), finite, within
/// `min..=max`.
fn stored_number(text: &str, min: f64, max: f64, what: &str) -> Result<String, String> {
    let t = text.trim();
    let plain = !t.is_empty() && t.len() <= 32 && t.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'));
    match t.parse::<f64>() {
        Ok(v) if plain && v.is_finite() && (min..=max).contains(&v) => Ok(t.to_string()),
        _ => Err(format!("{what}: unusable value {text:?} in the source")),
    }
}

/// A node's settings as the apply compares them: `invert`, `exponent`, the
/// custom curve (raw texts).
type NodeState = (Option<String>, Option<String>, Option<Vec<(String, String)>>);

fn node_state(node: Option<&NodeEntry>) -> NodeState {
    let Some(n) = node else { return (None, None, None) };
    let get = |k: &str| n.attrs.iter().find(|(name, _)| name == k).map(|(_, v)| v.clone());
    // Curve elements without points are no custom curve.
    (get("invert"), get("exponent"), n.points.clone().filter(|p| !p.is_empty()))
}

/// The node edits that make the live slot `pair.target` hold exactly the
/// source slot's settings: the source's values written, live ones the source
/// lacks removed. A source without that slot has no settings for it.
fn slot_edits(live: &DeviceConfig, source: &DeviceConfig, pair: SlotPair) -> Result<Vec<Edit>, String> {
    let empty = Vec::new();
    let src_nodes = source.options.iter().rev().find(|o| o.kind == pair.kind && o.instance == pair.source).map_or(&empty, |o| &o.nodes);
    let live_slot = live.options.iter().find(|o| o.kind == pair.kind && o.instance == pair.target);
    let slot = format!("{}{}", pair.kind.token_prefix(), pair.target);
    let Some(live_slot) = live_slot else {
        let has_values = src_nodes.iter().any(|n| !n.attrs.is_empty() || n.points.as_ref().is_some_and(|p| !p.is_empty()));
        return if has_values { Err(format!("no {slot} settings in the bindings file")) } else { Ok(Vec::new()) };
    };
    let mut names: Vec<&str> = Vec::new();
    for n in src_nodes.iter().chain(&live_slot.nodes) {
        if !names.contains(&n.name.as_str()) {
            names.push(&n.name);
        }
    }
    let mut edits = Vec::new();
    for name in names {
        if !is_identifier(name) {
            return Err(format!("invalid setting {name:?} in the source"));
        }
        let src = node_state(src_nodes.iter().rev().find(|n| n.name == name));
        let now = node_state(live_slot.nodes.iter().find(|n| n.name == name));
        let what = format!("{slot}/{name}");
        let mut push = |op| edits.push(Edit::Node { kind: pair.kind, instance: pair.target, node: name.to_string(), op });
        match src.0 {
            Some(v) => {
                if v != "0" && v != "1" {
                    return Err(format!("{what}: unusable inversion {v:?} in the source"));
                }
                if now.0.as_deref() != Some(v.as_str()) {
                    push(NodeOp::SetInvert(v));
                }
            }
            None if now.0.is_some() => push(NodeOp::ClearInvert),
            None => {}
        }
        // A custom curve takes precedence over an exponent next to it (the
        // game writes one or the other).
        match (src.2, src.1) {
            (Some(points), _) => {
                let points = points
                    .iter()
                    .map(|(i, o)| Ok((stored_number(i, 0.0, 1.0, &what)?, stored_number(o, 0.0, 1.0, &what)?)))
                    .collect::<Result<Vec<_>, String>>()?;
                if now.2.as_ref() != Some(&points) || now.1.is_some() {
                    push(NodeOp::SetPoints(points));
                }
            }
            (None, Some(exponent)) => {
                // The game's slider range, with room for its float noise.
                let exponent = stored_number(&exponent, 0.1 - 1e-4, 3.0 + 1e-4, &what)?;
                if now.1.as_ref() != Some(&exponent) || now.2.is_some() {
                    push(NodeOp::SetExponent(exponent));
                }
            }
            (None, None) if now.1.is_some() || now.2.is_some() => push(NodeOp::ClearCurve),
            (None, None) => {}
        }
    }
    Ok(edits)
}

/// The `<deviceoptions>` edits that make the live device of `pair.target`
/// hold exactly the source device's deadzones / saturations (joystick: the
/// Product of each slot's `<options>`; gamepad and mouse: their fixed
/// names). Live values the source lacks are removed — unlike the game's own
/// import, which merges. Values outside the kind's fixed inputs stay.
fn device_edits(live: &DeviceConfig, source: &DeviceConfig, pair: SlotPair) -> Result<Vec<Edit>, String> {
    let product = |c: &DeviceConfig, instance: u32| {
        c.options.iter().find(|o| o.kind == DeviceKind::Joystick && o.instance == instance).and_then(|o| o.product.clone())
    };
    let (src_name, live_name) = match pair.kind {
        DeviceKind::Joystick => (product(source, pair.source), product(live, pair.target)),
        DeviceKind::Gamepad => (Some(GAMEPAD_DEVICE.to_string()), Some(GAMEPAD_DEVICE.to_string())),
        DeviceKind::Keyboard => (Some(MOUSE_DEVICE.to_string()), Some(MOUSE_DEVICE.to_string())),
    };
    let slots = device_option_slots(pair.kind);
    let wanted: Vec<Option<&str>> =
        slots.iter().map(|(input, a)| src_name.as_deref().and_then(|n| last_option(source, n, input, a))).collect();
    let Some(live_name) = live_name else {
        return if wanted.iter().any(Option::is_some) {
            Err(format!("no {}{} device in the bindings file", pair.kind.token_prefix(), pair.target))
        } else {
            Ok(Vec::new())
        };
    };
    if !is_writable_name(&live_name) {
        return Err(format!("invalid device name {:?}", live_name.trim()));
    }
    // The removals after the writes: a device element emptied on the way
    // would go and come back elsewhere.
    let (mut edits, mut removals) = (Vec::new(), Vec::new());
    for ((input, attr_name), wanted) in slots.iter().zip(wanted) {
        let now: Vec<&str> = live
            .device_options
            .iter()
            .filter(|d| d.name == live_name)
            .flat_map(|d| &d.entries)
            .filter(|e| e.input == *input)
            .filter_map(|e| e.attrs.iter().find(|(k, _)| k == attr_name).map(|(_, v)| v.as_str()))
            .collect();
        match wanted {
            Some(v) => {
                let v = stored_number(v, 0.0, 1.0, &format!("{}/{input}", live_name.trim()))?;
                // Compared as read (the last duplicate); a write updates every one.
                if now.last() != Some(&v.as_str()) {
                    edits.push(Edit::DeviceOption { device: live_name.clone(), input: input.to_string(), attr: attr_name, value: v });
                }
            }
            None if !now.is_empty() => removals.push(Edit::RemoveDeviceOption { device: live_name.clone(), input: input.to_string(), attr: attr_name }),
            None => {}
        }
    }
    edits.extend(removals);
    Ok(edits)
}

/// Make the live `actionmaps.xml` hold the `source` file's settings for
/// `pairs`: per slot the `<options>` children (exactly the source's, see
/// [`slot_edits`]), per device the `<deviceoptions>` values
/// ([`device_edits`]). Values are taken over as the source stores them
/// (validated, never escaped); the rewrite is checked like
/// [`apply_config`]. Returns the text unchanged when nothing differs.
pub fn apply_settings(xml: &str, source: &DeviceConfig, pairs: &[SlotPair], trees: &[OptionTree]) -> Result<String, String> {
    let before = parse_device_config(xml)?;
    let mut edits = Vec::new();
    for pair in pairs {
        edits.extend(slot_edits(&before, source, *pair)?);
        edits.extend(device_edits(&before, source, *pair)?);
    }
    if edits.is_empty() {
        return Ok(xml.to_string());
    }
    let orders = tree_orders(trees);
    let mut expected = before;
    let mut out = xml.to_string();
    for edit in &edits {
        out = edit_text(&out, edit, &orders)?;
        apply_edit_model(&mut expected, edit, &orders);
    }
    check_rewrite(xml, &out, &expected)?;
    Ok(out)
}

/// Make the managed `settings` of an `attributes.xml` exactly the source's
/// (`source`: a backup's parsed `attributes.xml`): its stored value written,
/// a setting it lacks removed; nothing else changes. Checked like
/// [`apply_attributes`]. Returns the text unchanged when nothing differs.
pub fn copy_attributes(xml: &str, source: &BTreeMap<String, String>, settings: &[ManagedAttribute]) -> Result<String, String> {
    let mut expected = attribute_entries(xml)?;
    let mut out = xml.to_string();
    for setting in settings {
        let name = setting.attr_name();
        let now: Vec<&str> = expected.iter().filter(|(n, _)| n == name).map(|(_, v)| v.as_str()).collect();
        match source.get(name) {
            Some(v) => {
                let v = stored_number(v, f64::MIN, f64::MAX, name)?;
                // Compared as read (the last duplicate); a write updates every one.
                if now.last() == Some(&v.as_str()) {
                    continue;
                }
                out = set_attribute_text(&out, name, &v)?;
                if now.is_empty() {
                    let at = expected.iter().position(|(n, _)| n.as_str() > name).unwrap_or(expected.len());
                    expected.insert(at, (name.to_string(), v));
                } else {
                    for entry in expected.iter_mut().filter(|(n, _)| n == name) {
                        entry.1 = v.clone();
                    }
                }
            }
            None if !now.is_empty() => {
                out = remove_attribute_text(&out, name)?;
                expected.retain(|(n, _)| n != name);
            }
            None => {}
        }
    }
    if out == xml {
        return Ok(out);
    }
    let after = attribute_entries(&out).map_err(|e| format!("rewrite produced unreadable XML: {e}"))?;
    if after != expected {
        return Err("rewrite check failed: the game settings are not what was asked".into());
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// The Monitor's resting zones and the Axis Test's report
// ---------------------------------------------------------------------------

/// The hardware id (`input::DeviceInfo::hardware_id`) a `<deviceoptions>`
/// name belongs to: the Product GUID of a joystick's raw Product string
/// (upper case, like the id), `gamepad` for the gamepad; `None` for the
/// mouse and a name without a GUID.
pub fn hardware_id_of(device: &str) -> Option<String> {
    match device {
        GAMEPAD_DEVICE => Some(crate::input::GAMEPAD_HARDWARE_ID.to_string()),
        MOUSE_DEVICE => None,
        _ => scdata::split_product(device).1.map(|g| g.to_ascii_uppercase()),
    }
}

/// The configured deadzone / saturation per device and axis, as stored
/// (the last duplicate wins, as when reading), for the input thread's
/// resting zones: joysticks by Product GUID with their six axes, the
/// gamepad's two sticks (deadzone only). Unreadable values are left out.
pub fn axis_zones(config: &DeviceConfig) -> crate::input::ZoneMap {
    let mut out = crate::input::ZoneMap::new();
    for d in &config.device_options {
        let Some(id) = hardware_id_of(&d.name) else { continue };
        let kind = if d.name == GAMEPAD_DEVICE { DeviceKind::Gamepad } else { DeviceKind::Joystick };
        let slots = device_option_slots(kind);
        for e in &d.entries {
            for (k, v) in &e.attrs {
                if !slots.contains(&(e.input.as_str(), k.as_str())) {
                    continue;
                }
                let Some(v) = v.trim().parse::<f32>().ok().filter(|v| v.is_finite()) else { continue };
                let zone = out.entry(id.clone()).or_default().entry(e.input.clone()).or_default();
                if k == "deadzone" {
                    zone.deadzone = Some(v);
                } else {
                    zone.saturation = Some(v);
                }
            }
        }
    }
    out
}

/// The device settings as the files hold them, for the Axis Test report:
/// every `<deviceoptions>` and `<options>` element of `actionmaps` verbatim
/// (file order, indented by four), then the managed `<Attr>` lines of
/// `attributes` verbatim — `None` = no such file, an error = unreadable.
pub fn config_text(actionmaps: &str, attributes: Option<Result<String, String>>) -> Result<String, String> {
    let indent = |text: &str| text.lines().map(|l| format!("    {l}")).collect::<Vec<_>>();
    let masked = mask_markup(actionmaps);
    let mut spans = find_all(actionmaps, &masked, "deviceoptions")?;
    spans.extend(find_all(actionmaps, &masked, "options")?);
    spans.sort_by_key(|s| s.start);
    let mut lines = vec!["bindings file".to_string()];
    if spans.is_empty() {
        lines.push("    none".to_string());
    }
    for span in &spans {
        let from = span.start - indent_before(actionmaps, span.start).len();
        lines.extend(indent(&actionmaps[from..span.end]));
    }
    lines.push("settings file".to_string());
    match attributes {
        None => lines.push("    not found".to_string()),
        Some(Err(e)) => lines.push(format!("    error: {e}")),
        Some(Ok(xml)) => {
            let managed: Vec<&str> = managed_attributes(DeviceKind::Keyboard)
                .iter()
                .chain(managed_attributes(DeviceKind::Gamepad))
                .map(|m| m.attr_name())
                .collect();
            let masked = mask_markup(&xml);
            match find_all(&xml, &masked, "Attr") {
                Ok(tags) => {
                    let ours: Vec<&Span> = tags.iter().filter(|t| attr(t.head(&xml), "name").is_some_and(|n| managed.contains(&n))).collect();
                    if ours.is_empty() {
                        lines.push("    none".to_string());
                    }
                    for t in ours {
                        lines.extend(indent(&xml[t.start..t.end]));
                    }
                }
                Err(e) => lines.push(format!("    error: {e}")),
            }
        }
    }
    Ok(lines.join("\n") + "\n")
}

// ---------------------------------------------------------------------------
// Tauri commands
//
// Crate-visible: `generate_handler!` in lib.rs is the only caller.
// ---------------------------------------------------------------------------

/// The option trees of the loaded game data.
#[tauri::command]
pub(crate) fn get_option_trees(data: State<Mutex<AppData>>) -> Vec<OptionTree> {
    data.lock().unwrap().sc.data.options.clone()
}

/// The game's labels for the rows outside the option trees.
#[tauri::command]
pub(crate) fn get_config_labels(data: State<Mutex<AppData>>) -> ConfigLabels {
    data.lock().unwrap().sc.data.config_labels.clone()
}

/// The device settings of a source in display units: `Current` reads the
/// live `actionmaps.xml` and `attributes.xml`, a profile its file (profiles
/// carry no game settings), a backup its copies (an older backup has no
/// `attributes.xml`). An `attributes.xml` that cannot be read leaves the
/// game settings out (logged), the rest still shows. The files are read
/// outside the lock.
#[tauri::command]
pub(crate) fn get_device_config(source: diff::Source, app: AppHandle, data: State<Mutex<AppData>>) -> Result<DeviceConfigView, String> {
    let (base, loaded) = {
        let data = data.lock().unwrap();
        (data.config.base_path().to_string(), data.bindings_file.is_some())
    };
    let base = base.as_str();
    let (xml, attributes_path): (String, Option<PathBuf>) = match &source {
        diff::Source::Current => {
            if !loaded {
                return Err("No bindings loaded".into());
            }
            let path = config::actionmaps_path(base);
            let xml = std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
            (xml, Some(config::attributes_path(base)))
        }
        diff::Source::Profile { .. } => (diff::source_xml(&source, base, &backups::backups_root(&app)?)?, None),
        diff::Source::Backup { id } => {
            let root = backups::backups_root(&app)?;
            (diff::source_xml(&source, base, &root)?, backups::attributes_path_of(&root, id)?)
        }
    };
    let parsed = parse_device_config(&xml)?;
    let attributes = attributes_path.filter(|p| p.is_file()).and_then(|p| {
        match std::fs::read_to_string(&p).map_err(|e| e.to_string()).and_then(|text| parse_attributes(&text)) {
            Ok(map) => Some(map),
            Err(e) => {
                warn!("game settings not read: {}: {e}", p.display());
                None
            }
        }
    });
    Ok(view(&parsed, attributes.as_ref()))
}

/// The live device settings verbatim, for the Axis Test report (see
/// [`config_text`]). The files are read outside the lock.
#[tauri::command]
pub(crate) fn device_config_text(data: State<Mutex<AppData>>) -> Result<String, String> {
    let base = data.lock().unwrap().config.base_path().to_string();
    let path = config::actionmaps_path(&base);
    let xml = std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let attributes = config::attributes_path(&base);
    let attributes = attributes.is_file().then(|| std::fs::read_to_string(&attributes).map_err(|e| e.to_string()));
    config_text(&xml, attributes)
}

/// Write device settings into the live files — what the game's options
/// screens do, applied from outside. The game must not be running (it would
/// overwrite the files on exit). While auto-backups are on, one backup of
/// both files is taken first (reason "before config"); then whichever file
/// changed is written atomically. Reloads the bindings afterwards and
/// returns the load status, like `save_rebinds`.
#[tauri::command]
pub(crate) fn save_device_config(changes: Vec<ConfigChange>, app: AppHandle, data: State<Mutex<AppData>>) -> Result<LoadStatus, String> {
    let mut data = data.lock().unwrap();
    if data.bindings_file.is_none() {
        return Err("No bindings loaded".into());
    }
    if changes.is_empty() {
        return Err("Nothing to save".into());
    }
    let base = data.config.base_path().to_string();
    let actionmaps = config::actionmaps_path(&base);
    let attributes = config::attributes_path(&base);
    let refused = |what: &str, e: String| {
        error!("device settings rewrite of {what} refused ({} change(s)): {e}", changes.len());
        e
    };

    let new_actionmaps = if changes.iter().any(|c| !c.is_attribute()) {
        let xml = std::fs::read_to_string(&actionmaps).map_err(|e| format!("read {}: {e}", actionmaps.display()))?;
        let out = apply_config(&xml, &changes, &data.sc.data.options).map_err(|e| refused("actionmaps.xml", e))?;
        (out != xml).then_some(out)
    } else {
        None
    };
    let new_attributes = if changes.iter().any(ConfigChange::is_attribute) {
        if !attributes.is_file() {
            return Err(refused("attributes.xml", format!("{}: not found", attributes.display())));
        }
        let xml = std::fs::read_to_string(&attributes).map_err(|e| format!("read {}: {e}", attributes.display()))?;
        let out = apply_attributes(&xml, &changes).map_err(|e| refused("attributes.xml", e))?;
        (out != xml).then_some(out)
    } else {
        None
    };

    let summary: Vec<String> = changes.iter().map(ConfigChange::describe).collect();
    if new_actionmaps.is_none() && new_attributes.is_none() {
        info!("device settings unchanged, nothing written: {}", summary.join(", "));
        return Ok(crate::reload_bindings(&mut data));
    }
    let version = data.sc.version.as_ref().map(|v| v.label.as_str());
    let root = backups::backups_root(&app)?;
    let backup = gamefile::replace_live_config(
        &root,
        &actionmaps,
        new_actionmaps.as_deref(),
        new_attributes.as_deref(),
        "before config",
        data.config.auto_backup,
        version,
        &data.sc.data.actions,
    )?;
    info!("device settings written: {} ({})", summary.join(", "), gamefile::backup_label(&backup));
    Ok(crate::reload_bindings(&mut data))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small synthetic `defaultProfile.xml` with the game's tree shape.
    const PROFILE: &str = r#"<profile>
  <optiontree type="keyboard" name="root" UIShowInvert="-1">
    <optiongroup name="master" UILabel="@ui_master" UIShowCurve="0" UIShowInvert="0">
      <optiongroup name="mouse_curves" UILabel="@ui_mouse_curves" UIShowCurve="-1" UIShowInvert="0">
        <optiongroup name="inversion" UILabel="@ui_inv" UIShowInvert="-1">
          <optiongroup name="fps" UILabel="@ui_fps" UIShowCurve="1" UIShowInvert="-1">
            <optiongroup name="fps_view_pitch" UILabel="@ui_pitch" UIShowCurve="1" UIShowInvert="1"/>
            <optiongroup name="fps_view_yaw" UILabel="@ui_yaw" UIShowCurve="1" UIShowInvert="1"/>
          </optiongroup>
        </optiongroup>
      </optiongroup>
    </optiongroup>
  </optiontree>
  <optiontree type="gamepad" name="root">
    <optiongroup name="master" UIShowCurve="0" UIShowInvert="0">
      <optiongroup name="thumbstick_curves" UIShowCurve="-1" UIShowInvert="0">
        <optiongroup name="inversion" UIShowInvert="-1">
          <optiongroup name="fps_view" UIShowCurve="1" UIShowInvert="-1">
            <optiongroup name="fps_view_pitch" UIShowCurve="1" UIShowInvert="1"/>
          </optiongroup>
        </optiongroup>
      </optiongroup>
    </optiongroup>
  </optiontree>
  <optiontree type="joystick" instances="8" name="root">
    <optiongroup name="master" UIShowCurve="0" UIShowInvert="0">
      <optiongroup name="joystick_curves" UIShowCurve="-1" UIShowInvert="0">
        <optiongroup name="inversion" UIShowCurve="0" UIShowInvert="-1">
          <optiongroup name="flight" UIShowCurve="-1" UIShowInvert="-1">
            <optiongroup name="flight_move" UIShowCurve="-1" UIShowInvert="-1">
              <optiongroup name="flight_move_pitch" UIShowCurve="1" UIShowInvert="1"/>
              <optiongroup name="flight_move_yaw" UIShowCurve="1" UIShowInvert="1"/>
              <optiongroup name="flight_move_roll" UIShowCurve="1" UIShowInvert="1"/>
            </optiongroup>
            <optiongroup name="flight_view" exponent="2.5" UIShowCurve="1" UIShowInvert="-1">
              <nonlinearity_curve reset="1"/>
              <optiongroup name="flight_view_pitch" UIShowCurve="1" UIShowInvert="1"/>
              <optiongroup name="flight_view_yaw" UIShowCurve="1" UIShowInvert="1"/>
            </optiongroup>
            <optiongroup name="flight_zoom" UIShowInvert="1"/>
          </optiongroup>
          <optiongroup name="mining" UIShowCurve="1" UIShowInvert="1">
            <optiongroup name="mining_throttle" invert="1" UIShowCurve="1" UIShowInvert="1"/>
          </optiongroup>
        </optiongroup>
      </optiongroup>
    </optiongroup>
  </optiontree>
</profile>"#;

    const STICK_L: &str = " VKB L    {0201231D-0000-0000-0000-504944564944}";
    const STICK_R: &str = " VKB R    {0200231D-0000-0000-0000-504944564944}";

    // Shaped like the game's own output (LF here, one-space indent).
    const XML: &str = concat!(
        "<ActionMaps>\n",
        " <ActionProfiles version=\"1\" optionsVersion=\"2\" rebindVersion=\"2\" profileName=\"default\">\n",
        "  <deviceoptions name=\" VKB L    {0201231D-0000-0000-0000-504944564944}\">\n",
        "   <option input=\"x\" deadzone=\"0.2277\"/>\n",
        "   <option input=\"y\" saturation=\"0.91079998\"/>\n",
        "   <option input=\"y\" saturation=\"0.49500001\"/>\n",
        "  </deviceoptions>\n",
        "  <deviceoptions name=\"Controller (Gamepad)\">\n",
        "   <option input=\"thumbl\" deadzone=\"0.58434999\"/>\n",
        "  </deviceoptions>\n",
        "  <deviceoptions name=\"Mouse\">\n",
        "   <option input=\"@pause_OptionsMouseAcceleration\" acceleration=\"0.077\"/>\n",
        "   <option input=\"@pause_OptionsMouseSmoothing\" saturation=\"0.29699999\"/>\n",
        "  </deviceoptions>\n",
        "  <options type=\"keyboard\" instance=\"1\" Product=\"Keyboard  {6F1D2B61-D5A0-11CF-BFC7-444553540000}\">\n",
        "   <fps>\n",
        "    <nonlinearity_curve>\n",
        "     <point in=\"0\" out=\"0\"/>\n",
        "     <point in=\"0.5\" out=\"0.2\"/>\n",
        "     <point in=\"1\" out=\"1\"/>\n",
        "    </nonlinearity_curve>\n",
        "   </fps>\n",
        "  </options>\n",
        "  <options type=\"gamepad\" instance=\"1\" Product=\"Controller (Gamepad)\"/>\n",
        "  <options type=\"joystick\" instance=\"1\" Product=\" VKB L    {0201231D-0000-0000-0000-504944564944}\">\n",
        "   <flight_move_yaw exponent=\"2\"/>\n",
        "   <flight_move_roll invert=\"1\"/>\n",
        "   <flight_view exponent=\"0.69999999\"/>\n",
        "   <flight_view_pitch invert=\"1\" exponent=\"2\"/>\n",
        "   <flight_view_yaw exponent=\"2\"/>\n",
        "   <mining_throttle invert=\"0\"/>\n",
        "  </options>\n",
        "  <options type=\"joystick\" instance=\"2\" Product=\" VKB R    {0200231D-0000-0000-0000-504944564944}\"/>\n",
        "  <modifiers />\n",
        "  <actionmap name=\"spaceship_general\">\n",
        "   <action name=\"v_eject\">\n",
        "    <rebind input=\"js1_button1\"/>\n",
        "   </action>\n",
        "  </actionmap>\n",
        " </ActionProfiles>\n",
        "</ActionMaps>\n",
    );

    const ATTRS: &str = concat!(
        "<Attributes Version=\"35\">\n",
        " <Attr name=\"AutoZoom\" value=\"1\"/>\n",
        " <Attr name=\"MouseSensitivity\" value=\"6.06061\"/>\n",
        " <Attr name=\"Sensitivity\" value=\"1\"/>\n",
        " <Attr name=\"SysSpec\" value=\"3\"/>\n",
        "</Attributes>\n",
    );

    fn trees() -> Vec<OptionTree> {
        scdata::parse_option_trees(PROFILE, &HashMap::new()).unwrap()
    }

    fn apply(xml: &str, changes: &[ConfigChange]) -> Result<String, String> {
        apply_config(xml, changes, &trees())
    }

    fn curve(kind: DeviceKind, instance: u32, node: &str, value: CurveValue) -> ConfigChange {
        ConfigChange::Curve { kind, instance, node: node.into(), value }
    }

    fn exponent(instance: u32, node: &str, value: f32) -> ConfigChange {
        curve(DeviceKind::Joystick, instance, node, CurveValue::Exponent { value })
    }

    fn invert(instance: u32, node: &str, value: Option<bool>) -> ConfigChange {
        ConfigChange::Invert { kind: DeviceKind::Joystick, instance, node: node.into(), value }
    }

    fn deadzone(device: &str, input: &str, value: f32) -> ConfigChange {
        ConfigChange::Deadzone { device: device.into(), input: input.into(), value }
    }

    fn saturation(device: &str, input: &str, value: f32) -> ConfigChange {
        ConfigChange::Saturation { device: device.into(), input: input.into(), value }
    }

    fn attribute(setting: ManagedAttribute, value: f32) -> ConfigChange {
        ConfigChange::Attribute { setting, value }
    }

    // --- floats -------------------------------------------------------------

    #[test]
    fn actionmaps_floats_are_percent_8g_of_the_f32() {
        assert_eq!(format_actionmaps_float(0.7), "0.69999999");
        assert_eq!(format_actionmaps_float(0.025118863), "0.025118863");
        assert_eq!(format_actionmaps_float(3.0), "3");
        assert_eq!(format_actionmaps_float(0.1), "0.1");
        assert_eq!(format_actionmaps_float(0.0), "0");
        assert_eq!(format_actionmaps_float(1.0), "1");
        // Outside fixed notation, C's exponent form.
        assert_eq!(format_g(0.00001234, 8), "1.234e-05");
        assert_eq!(format_g(123_456_789.0, 8), "1.2345679e+08");
        assert_eq!(format_g(0.0001, 6), "0.0001");
    }

    #[test]
    fn attributes_floats_are_percent_g() {
        assert_eq!(format_attributes_float(5.0 + 11.0 * 35.0 / 99.0), "8.88889");
        assert_eq!(format_attributes_float(5.0 + 3.0 * 35.0 / 99.0), "6.06061");
        assert_eq!(format_attributes_float(5.0 + 17.0 * 35.0 / 99.0), "11.0101");
        assert_eq!(format_attributes_float(40.0), "40");
        assert_eq!(format_attributes_float(0.5), "0.5");
        assert_eq!(format_attributes_float(1.2), "1.2");
    }

    #[test]
    fn display_and_stored_values_convert_like_the_game() {
        // Joystick deadzone / saturation: × 0.99.
        assert_eq!(stored_scaled(0.15, JOYSTICK_SCALE), "0.1485");
        assert_eq!(stored_scaled(0.14, JOYSTICK_SCALE), "0.13860001");
        assert_eq!(stored_scaled(0.92, JOYSTICK_SCALE), "0.91079998");
        // Gamepad deadzone: × 0.899.
        assert_eq!(stored_scaled(0.65, GAMEPAD_SCALE), "0.58434999");
        assert_eq!(stored_scaled(1.0, GAMEPAD_SCALE), "0.89899999");
        assert_eq!(stored_scaled(0.5, GAMEPAD_SCALE), "0.44949999");
        // Mouse acceleration × 0.1, smoothing × 0.9.
        assert_eq!(stored_scaled(0.77, ACCELERATION_SCALE), "0.077");
        assert_eq!(stored_scaled(0.5, ACCELERATION_SCALE), "0.050000001");
        assert_eq!(stored_scaled(0.33, SMOOTHING_SCALE), "0.29699999");
        assert_eq!(stored_scaled(0.5, SMOOTHING_SCALE), "0.44999999");
        // And back, on the GUI's 0.01 grid.
        for (stored, scale, display) in [
            ("0.1485", JOYSTICK_SCALE, 0.15),
            ("0.13860001", JOYSTICK_SCALE, 0.14),
            ("0.91079998", JOYSTICK_SCALE, 0.92),
            ("0.2277", JOYSTICK_SCALE, 0.23),
            ("0.58434999", GAMEPAD_SCALE, 0.65),
            ("0.89899999", GAMEPAD_SCALE, 1.0),
            ("0.077", ACCELERATION_SCALE, 0.77),
            ("0.050000001", ACCELERATION_SCALE, 0.5),
            ("0.29699999", SMOOTHING_SCALE, 0.33),
        ] {
            assert_eq!(display_scaled(stored, scale), Some(display), "{stored}");
        }
        assert_eq!(display_scaled("x", JOYSTICK_SCALE), None);
        assert_eq!(display_scaled("inf", JOYSTICK_SCALE), None);
    }

    #[test]
    fn managed_attributes_convert_both_ways() {
        use ManagedAttribute::*;
        for (display, stored) in [(4.0, "6.06061"), (12.0, "8.88889"), (18.0, "11.0101"), (100.0, "40"), (1.0, "5")] {
            assert_eq!(MouseSensitivity.stored(display).unwrap(), stored);
            assert_eq!(MouseSensitivity.display(stored), Some(display));
        }
        assert_eq!(AdsPercent.stored(50.0).unwrap(), "0.5");
        assert_eq!(AdsPercent.display("0.5"), Some(50.0));
        assert_eq!(ZoomScalingPercent.stored(75.0).unwrap(), "0.75");
        assert_eq!(ZoomScalingEnabled.stored(1.0).unwrap(), "1");
        assert_eq!(ZoomScalingEnabled.stored(0.0).unwrap(), "0");
        assert_eq!(GamepadSensitivity.stored(1.2).unwrap(), "1.2");
        assert_eq!(GamepadSensitivity.display("1.2"), Some(1.2));
        for (setting, bad) in [
            (MouseSensitivity, 0.0),
            (MouseSensitivity, 101.0),
            (AdsPercent, 201.0),
            (AdsPercent, -1.0),
            (ZoomScalingEnabled, 2.0),
            (GamepadSensitivity, 2.01),
            (GamepadSensitivity, f32::NAN),
            (ZoomScalingPercent, f32::INFINITY),
        ] {
            assert!(setting.stored(bad).is_err(), "{setting:?} {bad}");
        }
    }

    // --- reading ------------------------------------------------------------

    #[test]
    fn reads_every_setting_in_display_units() {
        let config = parse_device_config(XML).unwrap();
        let attrs = parse_attributes(ATTRS).unwrap();
        let v = view(&config, Some(&attrs));

        assert_eq!(v.options.len(), 4);
        let kb = &v.options[0];
        assert_eq!((kb.kind, kb.instance), (DeviceKind::Keyboard, 1));
        assert_eq!(kb.values["fps"].points, Some(vec![[0.0, 0.0], [0.5, 0.2], [1.0, 1.0]]));
        assert_eq!(kb.values["fps"].exponent, None);
        let js1 = &v.options[2];
        assert_eq!(js1.product.as_deref(), Some(STICK_L));
        assert_eq!(js1.values["flight_move_roll"], NodeValue { invert: Some(true), exponent: None, points: None });
        assert_eq!(js1.values["flight_view"].exponent, Some(0.7));
        assert_eq!(js1.values["flight_view_pitch"], NodeValue { invert: Some(true), exponent: Some(2.0), points: None });
        assert_eq!(js1.values["mining_throttle"].invert, Some(false));
        assert!(v.options[1].values.is_empty() && v.options[3].values.is_empty());

        // The mouse is reported in `mouse`, not as a device.
        assert_eq!(v.device_options.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), vec![STICK_L, GAMEPAD_DEVICE]);
        let stick = &v.device_options[0];
        assert_eq!(stick.axes.len(), 2);
        assert_eq!((stick.axes[0].input.as_str(), stick.axes[0].deadzone, stick.axes[0].saturation), ("x", Some(0.23), None));
        // A duplicate: the last one wins.
        assert_eq!((stick.axes[1].input.as_str(), stick.axes[1].deadzone, stick.axes[1].saturation), ("y", None, Some(0.5)));
        assert_eq!(v.device_options[1].axes[0].deadzone, Some(0.65));

        assert_eq!(v.mouse.acceleration, Some(0.77));
        assert_eq!(v.mouse.smoothing, Some(0.33));
        assert_eq!(v.mouse.sensitivity, Some(4.0));
        assert_eq!((v.mouse.ads_percent, v.mouse.zoom_scaling_enabled, v.mouse.zoom_scaling_percent), (None, None, None));
        assert_eq!(v.gamepad_sensitivity, Some(1.0));
        assert!(v.has_attributes);

        // Without an attributes.xml (a profile, an older backup).
        let v = view(&config, None);
        assert!(!v.has_attributes);
        assert_eq!((v.mouse.sensitivity, v.gamepad_sensitivity), (None, None));
        assert_eq!(v.mouse.acceleration, Some(0.77));
    }

    #[test]
    fn reading_tolerates_what_the_game_or_a_user_may_write() {
        let odd = "\u{feff}<?xml version=\"1.0\"?>\r\n<!-- <options type=\"joystick\" instance=\"9\"/> -->\r\n<ActionMaps>\r\n\t<options type='joystick' instance='1' Product='A &amp; B'>\r\n\t\t<fps_view invert='1'><![CDATA[x]]></fps_view>\r\n\t\t<odd/>\r\n\t</options>\r\n\t<options type=\"mouse\" instance=\"1\"><x invert=\"1\"/></options>\r\n\t<options type=\"joystick\" instance=\"x\"/>\r\n\t<deviceoptions name=\"Mouse\"/>\r\n</ActionMaps>\r\n".to_string();
        let config = parse_device_config(&odd).unwrap();
        assert_eq!(config.options.len(), 1, "unknown type and a non-numeric instance are skipped");
        assert_eq!(config.options[0].product.as_deref(), Some("A & B"));
        assert_eq!(config.options[0].nodes.len(), 2);
        assert_eq!(view(&config, None).options[0].values.len(), 1, "an element without a value is no value");
        assert_eq!(config.device_options.len(), 1);

        assert!(parse_device_config("<Attributes/>").unwrap_err().contains("not a bindings file"));
        assert!(parse_device_config("").unwrap_err().contains("not a bindings file"));
        assert!(parse_device_config("<ActionMaps><options a=b/></ActionMaps>").is_err());
    }

    #[test]
    fn parsing_and_writing_never_panic_on_a_truncated_file() {
        let changes = [exponent(1, "flight_move_pitch", 1.5), deadzone(STICK_R, "x", 0.1), invert(1, "flight_move_roll", None)];
        for cut in (0..XML.len()).step_by(7) {
            let part = &XML[..cut];
            let _ = parse_device_config(part);
            let _ = apply(part, &changes);
            let _ = attribute_entries(&ATTRS[..cut.min(ATTRS.len())]);
            let _ = apply_attributes(&ATTRS[..cut.min(ATTRS.len())], &[attribute(ManagedAttribute::AdsPercent, 50.0)]);
        }
        for junk in ["<", "<ActionMaps", "<ActionMaps><options", "<ActionMaps><options type=\"joystick\" instance=\"1\"><a></options></ActionMaps>", "<!-- <ActionMaps> -->", "</ActionMaps>"] {
            let _ = parse_device_config(junk);
            let _ = apply(junk, &changes);
        }
    }

    #[test]
    fn attributes_parse_with_the_last_duplicate_winning() {
        let xml = "<Attributes Version=\"35\"><Attr name=\"A\" value=\"1\"/><Attr name=\"A\" value=\"2\"/><Attr value=\"x\"/></Attributes>";
        let map = parse_attributes(xml).unwrap();
        assert_eq!(map.len(), 1);
        assert_eq!(map["A"], "2");
        assert!(parse_attributes("<ActionMaps/>").unwrap_err().contains("not a settings file"));
        assert!(parse_attributes("").is_err());
        assert!(parse_attributes("<Attributes><Attr name=\"A\" value=\"1\"></Attributes>").is_err());
    }

    // --- curves -------------------------------------------------------------

    #[test]
    fn sets_an_exponent_on_an_existing_node() {
        let out = apply(XML, &[exponent(1, "flight_move_yaw", 0.7)]).unwrap();
        assert_eq!(out, XML.replace("<flight_move_yaw exponent=\"2\"/>", "<flight_move_yaw exponent=\"0.69999999\"/>"));
        // Typed input is rounded to the slider's 0.1 grid.
        let out = apply(XML, &[exponent(1, "flight_move_yaw", 1.249)]).unwrap();
        assert!(out.contains("<flight_move_yaw exponent=\"1.2\"/>"));
        let out = apply(XML, &[exponent(1, "flight_move_yaw", 3.04)]).unwrap();
        assert!(out.contains("<flight_move_yaw exponent=\"3\"/>"));
    }

    #[test]
    fn a_new_node_lands_at_its_tree_order_position() {
        let out = apply(XML, &[exponent(1, "flight_move_pitch", 1.5)]).unwrap();
        assert_eq!(out, XML.replace("   <flight_move_yaw", "   <flight_move_pitch exponent=\"1.5\"/>\n   <flight_move_yaw"));
        let out = apply(XML, &[invert(1, "flight_zoom", Some(true))]).unwrap();
        assert_eq!(out, XML.replace("   <mining_throttle", "   <flight_zoom invert=\"1\"/>\n   <mining_throttle"));
        let out = apply(XML, &[invert(1, "mining", Some(false))]).unwrap();
        // Mining's own toggle replaces the throttle's inversion, which leaves
        // that element empty: it goes, mining takes its place.
        assert_eq!(out, XML.replace("   <mining_throttle invert=\"0\"/>\n", "   <mining invert=\"0\"/>\n"));
    }

    #[test]
    fn a_group_curve_replaces_the_curves_below_it() {
        let out = apply(XML, &[exponent(1, "flight_view", 1.5)]).unwrap();
        let expected = XML
            .replace("<flight_view exponent=\"0.69999999\"/>", "<flight_view exponent=\"1.5\"/>")
            .replace("<flight_view_pitch invert=\"1\" exponent=\"2\"/>", "<flight_view_pitch invert=\"1\"/>")
            .replace("   <flight_view_yaw exponent=\"2\"/>\n", "");
        assert_eq!(out, expected);
        // A custom group curve does the same.
        let points = vec![[0.0, 0.0], [1.0, 1.0]];
        let out = apply(XML, &[curve(DeviceKind::Joystick, 1, "flight_view", CurveValue::Points { points })]).unwrap();
        assert!(out.contains("   <flight_view>\n    <nonlinearity_curve>\n     <point in=\"0\" out=\"0\"/>\n     <point in=\"1\" out=\"1\"/>\n    </nonlinearity_curve>\n   </flight_view>\n"));
        assert!(out.contains("<flight_view_pitch invert=\"1\"/>") && !out.contains("flight_view_yaw"));
        // Setting a child after the group keeps both.
        let out = apply(XML, &[exponent(1, "flight_view", 1.5), exponent(1, "flight_view_yaw", 0.5)]).unwrap();
        assert!(out.contains("<flight_view exponent=\"1.5\"/>\n   <flight_view_pitch invert=\"1\"/>\n   <flight_view_yaw exponent=\"0.5\"/>\n"));
    }

    #[test]
    fn set_default_removes_only_that_nodes_curve() {
        let out = apply(XML, &[curve(DeviceKind::Joystick, 1, "flight_view", CurveValue::Default)]).unwrap();
        assert_eq!(out, XML.replace("   <flight_view exponent=\"0.69999999\"/>\n", ""));
        // The inversion stays.
        let out = apply(XML, &[curve(DeviceKind::Joystick, 1, "flight_view_pitch", CurveValue::Default)]).unwrap();
        assert_eq!(out, XML.replace("<flight_view_pitch invert=\"1\" exponent=\"2\"/>", "<flight_view_pitch invert=\"1\"/>"));
        // Nothing set: nothing to do.
        let out = apply(XML, &[curve(DeviceKind::Joystick, 2, "flight_view", CurveValue::Default)]).unwrap();
        assert_eq!(out, XML);
    }

    #[test]
    fn the_last_child_leaves_a_self_closing_options_element() {
        let out = apply(XML, &[curve(DeviceKind::Keyboard, 1, "fps", CurveValue::Default)]).unwrap();
        let block = "  <options type=\"keyboard\" instance=\"1\" Product=\"Keyboard  {6F1D2B61-D5A0-11CF-BFC7-444553540000}\">\n   <fps>\n    <nonlinearity_curve>\n     <point in=\"0\" out=\"0\"/>\n     <point in=\"0.5\" out=\"0.2\"/>\n     <point in=\"1\" out=\"1\"/>\n    </nonlinearity_curve>\n   </fps>\n  </options>\n";
        assert_eq!(out, XML.replace(block, "  <options type=\"keyboard\" instance=\"1\" Product=\"Keyboard  {6F1D2B61-D5A0-11CF-BFC7-444553540000}\"/>\n"));
        // And a self-closing one is opened for a first child.
        let back = apply(&out, &[curve(DeviceKind::Keyboard, 1, "fps", CurveValue::Points { points: vec![[0.0, 0.0], [0.5, 0.2], [1.0, 1.0]] })]).unwrap();
        assert_eq!(back, XML);
    }

    #[test]
    fn custom_points_replace_the_exponent_sorted_by_in() {
        let points = vec![[1.0, 1.0], [0.0, 0.0], [0.5, 0.25], [0.3, 0.7]];
        let out = apply(XML, &[curve(DeviceKind::Joystick, 1, "flight_move_yaw", CurveValue::Points { points })]).unwrap();
        assert_eq!(
            out,
            XML.replace(
                "   <flight_move_yaw exponent=\"2\"/>\n",
                "   <flight_move_yaw>\n    <nonlinearity_curve>\n     <point in=\"0\" out=\"0\"/>\n     <point in=\"0.30000001\" out=\"0.69999999\"/>\n     <point in=\"0.5\" out=\"0.25\"/>\n     <point in=\"1\" out=\"1\"/>\n    </nonlinearity_curve>\n   </flight_move_yaw>\n"
            )
        );
        // The slider drops the custom curve again.
        let back = apply(&out, &[exponent(1, "flight_move_yaw", 2.0)]).unwrap();
        assert_eq!(back, XML);
        // An existing curve is replaced in place.
        let points = vec![[0.0, 0.0], [0.6, 0.1], [1.0, 1.0]];
        let out = apply(XML, &[curve(DeviceKind::Keyboard, 1, "fps", CurveValue::Points { points })]).unwrap();
        assert_eq!(out, XML.replace("<point in=\"0.5\" out=\"0.2\"/>", "<point in=\"0.60000002\" out=\"0.1\"/>"));
        // The inversion of a node keeps its place next to a new curve.
        let points = vec![[0.0, 0.0], [1.0, 1.0]];
        let out = apply(XML, &[curve(DeviceKind::Joystick, 1, "flight_view_pitch", CurveValue::Points { points })]).unwrap();
        assert!(out.contains("   <flight_view_pitch invert=\"1\">\n    <nonlinearity_curve>\n"), "{out}");
    }

    #[test]
    fn a_new_node_with_points_opens_an_empty_slot() {
        let points = vec![[0.0, 0.0], [1.0, 1.0]];
        let out = apply(XML, &[curve(DeviceKind::Joystick, 2, "flight_move_pitch", CurveValue::Points { points })]).unwrap();
        let slot = format!("  <options type=\"joystick\" instance=\"2\" Product=\"{STICK_R}\"");
        assert_eq!(
            out,
            XML.replace(
                &format!("{slot}/>\n"),
                &format!("{slot}>\n   <flight_move_pitch>\n    <nonlinearity_curve>\n     <point in=\"0\" out=\"0\"/>\n     <point in=\"1\" out=\"1\"/>\n    </nonlinearity_curve>\n   </flight_move_pitch>\n  </options>\n")
            )
        );
    }

    #[test]
    fn curve_input_is_validated() {
        let bad_points = [
            vec![[0.0, 0.0]],
            vec![[0.0, 0.0], [0.5, 0.5]],
            vec![[0.1, 0.0], [1.0, 1.0]],
            vec![[0.0, 0.0], [1.0, 1.0], [1.2, 0.5]],
            vec![[0.0, 0.0], [1.0, 1.0], [0.5, -0.1]],
            vec![[0.0, 0.0], [1.0, 1.0], [f32::NAN, 0.5]],
            [[0.0, 0.0], [1.0, 1.0]].into_iter().chain(std::iter::repeat_n([0.5, 0.5], 63)).collect(),
        ];
        for points in bad_points {
            let c = curve(DeviceKind::Joystick, 1, "flight_move_yaw", CurveValue::Points { points: points.clone() });
            assert!(apply(XML, &[c]).is_err(), "{points:?}");
        }
        let max: Vec<[f32; 2]> = [[0.0, 0.0], [1.0, 1.0]].into_iter().chain(std::iter::repeat_n([0.5, 0.5], 62)).collect();
        assert!(apply(XML, &[curve(DeviceKind::Joystick, 1, "flight_move_yaw", CurveValue::Points { points: max })]).is_ok());
        for value in [0.04, 3.06, -1.0, f32::NAN, f32::INFINITY] {
            assert!(apply(XML, &[exponent(1, "flight_move_yaw", value)]).is_err(), "{value}");
        }
        assert!(apply(XML, &[exponent(1, "flight_move_yaw", 0.05)]).is_ok(), "rounds to 0.1");
    }

    // --- inversion ----------------------------------------------------------

    #[test]
    fn invert_goes_before_exponent_and_default_removes_it() {
        let out = apply(XML, &[invert(1, "flight_move_yaw", Some(true))]).unwrap();
        assert_eq!(out, XML.replace("<flight_move_yaw exponent=\"2\"/>", "<flight_move_yaw invert=\"1\" exponent=\"2\"/>"));
        let out = apply(XML, &[invert(1, "flight_move_roll", Some(false))]).unwrap();
        assert_eq!(out, XML.replace("<flight_move_roll invert=\"1\"/>", "<flight_move_roll invert=\"0\"/>"));
        let out = apply(XML, &[invert(1, "flight_move_roll", None)]).unwrap();
        assert_eq!(out, XML.replace("   <flight_move_roll invert=\"1\"/>\n", ""));
        let out = apply(XML, &[invert(1, "flight_view_pitch", None)]).unwrap();
        assert_eq!(out, XML.replace("<flight_view_pitch invert=\"1\" exponent=\"2\"/>", "<flight_view_pitch exponent=\"2\"/>"));
    }

    #[test]
    fn the_mining_toggle_replaces_the_inversion_below_it() {
        let xml = XML.replace("<mining_throttle invert=\"0\"/>", "<mining_throttle invert=\"0\" exponent=\"2\"/>");
        let out = apply(&xml, &[invert(1, "mining", Some(true))]).unwrap();
        assert_eq!(out, xml.replace("   <mining_throttle invert=\"0\" exponent=\"2\"/>\n", "   <mining invert=\"1\"/>\n   <mining_throttle exponent=\"2\"/>\n"));
        // Its Set Default touches only mining itself.
        let out2 = apply(&out, &[invert(1, "mining", None)]).unwrap();
        assert_eq!(out2, xml.replace("   <mining_throttle invert=\"0\" exponent=\"2\"/>\n", "   <mining_throttle exponent=\"2\"/>\n"));
    }

    // --- refusals -----------------------------------------------------------

    #[test]
    fn node_changes_are_refused_when_they_do_not_fit() {
        let refused = |c: ConfigChange| apply(XML, &[c]).unwrap_err();
        assert!(refused(exponent(4, "flight_move_yaw", 1.0)).contains("js4"), "BindSight never creates a slot");
        assert!(refused(exponent(1, "no_such_node", 1.0)).contains("unknown"));
        assert!(refused(exponent(1, "flight_zoom", 1.0)).contains("no curve"));
        assert!(refused(exponent(1, "flight_move", 1.0)).contains("no curve"), "a group header");
        assert!(refused(invert(1, "flight_view", Some(true))).contains("no inversion"));
        assert!(refused(invert(1, "x\"/><y", Some(true))).contains("invalid"));
        assert!(refused(curve(DeviceKind::Gamepad, 2, "fps_view", CurveValue::Exponent { value: 1.0 })).contains("gp2"));
        assert!(apply_config(XML, &[exponent(1, "flight_move_yaw", 1.0)], &[]).unwrap_err().contains("no option tree"));
        assert!(apply(XML, &[]).is_err());
        assert!(apply(XML, &[attribute(ManagedAttribute::AdsPercent, 50.0)]).is_err(), "not part of the bindings file");
        // Two elements for one slot or one node: refused, not guessed.
        let twice = XML.replace("<options type=\"joystick\" instance=\"2\"", "<options type=\"joystick\" instance=\"1\"");
        assert!(apply(&twice, &[exponent(1, "flight_move_yaw", 1.0)]).unwrap_err().contains("more than one"));
        let dup = XML.replace("   <flight_move_roll invert=\"1\"/>\n", "   <flight_move_roll invert=\"1\"/>\n   <flight_move_roll invert=\"0\"/>\n");
        assert!(apply(&dup, &[invert(1, "flight_move_roll", Some(true))]).unwrap_err().contains("more than one"));
    }

    // --- device options ------------------------------------------------------

    #[test]
    fn deadzone_and_saturation_update_every_duplicate() {
        let out = apply(XML, &[deadzone(STICK_L, "x", 0.15)]).unwrap();
        assert_eq!(out, XML.replace("deadzone=\"0.2277\"", "deadzone=\"0.1485\""));
        let out = apply(XML, &[saturation(STICK_L, "y", 0.92)]).unwrap();
        assert_eq!(out, XML.replace("saturation=\"0.49500001\"", "saturation=\"0.91079998\""));
        let out = apply(XML, &[saturation(STICK_L, "y", 0.14)]).unwrap();
        assert_eq!(out.matches("saturation=\"0.13860001\"").count(), 2);
    }

    #[test]
    fn a_new_entry_is_appended_and_a_new_device_created() {
        // Saturation for x: x has only a deadzone entry, so it is a new one.
        let out = apply(XML, &[saturation(STICK_L, "x", 0.92)]).unwrap();
        assert_eq!(
            out,
            XML.replace("   <option input=\"y\" saturation=\"0.49500001\"/>\n", "   <option input=\"y\" saturation=\"0.49500001\"/>\n   <option input=\"x\" saturation=\"0.91079998\"/>\n")
        );
        let out = apply(XML, &[deadzone(GAMEPAD_DEVICE, "thumbr", 1.0)]).unwrap();
        assert!(out.contains("   <option input=\"thumbl\" deadzone=\"0.58434999\"/>\n   <option input=\"thumbr\" deadzone=\"0.89899999\"/>\n  </deviceoptions>\n"));
        // The second stick has no element yet: one after the last.
        let out = apply(XML, &[deadzone(STICK_R, "rotz", 0.14)]).unwrap();
        assert_eq!(
            out,
            XML.replace(
                "   <option input=\"@pause_OptionsMouseSmoothing\" saturation=\"0.29699999\"/>\n  </deviceoptions>\n",
                &format!("   <option input=\"@pause_OptionsMouseSmoothing\" saturation=\"0.29699999\"/>\n  </deviceoptions>\n  <deviceoptions name=\"{STICK_R}\">\n   <option input=\"rotz\" deadzone=\"0.13860001\"/>\n  </deviceoptions>\n")
            )
        );
    }

    #[test]
    fn without_any_device_element_the_first_goes_before_the_options() {
        let start = XML.find("  <deviceoptions").unwrap();
        let end = XML.find("  <options").unwrap();
        let bare = format!("{}{}", &XML[..start], &XML[end..]);
        let out = apply(&bare, &[ConfigChange::MouseSmoothing { value: 0.5 }]).unwrap();
        assert_eq!(
            out,
            bare.replace(
                "  <options type=\"keyboard\"",
                "  <deviceoptions name=\"Mouse\">\n   <option input=\"@pause_OptionsMouseSmoothing\" saturation=\"0.44999999\"/>\n  </deviceoptions>\n  <options type=\"keyboard\""
            )
        );
    }

    #[test]
    fn mouse_acceleration_and_smoothing_use_the_games_inputs() {
        let out = apply(XML, &[ConfigChange::MouseAcceleration { value: 0.5 }, ConfigChange::MouseSmoothing { value: 0.5 }]).unwrap();
        assert_eq!(out, XML.replace("acceleration=\"0.077\"", "acceleration=\"0.050000001\"").replace("saturation=\"0.29699999\"", "saturation=\"0.44999999\""));
    }

    #[test]
    fn device_changes_are_refused_when_they_do_not_fit() {
        let refused = |c: ConfigChange| apply(XML, &[c]).unwrap_err();
        assert!(refused(saturation(GAMEPAD_DEVICE, "thumbl", 0.5)).contains("no saturation"));
        assert!(refused(deadzone(MOUSE_DEVICE, "x", 0.5)).contains("mouse"));
        assert!(refused(deadzone(STICK_L, "slider1", 0.5)).contains("invalid input"));
        assert!(refused(deadzone(STICK_L, "thumbl", 0.5)).contains("invalid input"));
        assert!(refused(deadzone(GAMEPAD_DEVICE, "x", 0.5)).contains("invalid input"));
        assert!(refused(deadzone("Some Other Stick {X}", "x", 0.5)).contains("unknown device"));
        assert!(refused(deadzone("a\"/><x", "x", 0.5)).contains("invalid device name"));
        assert!(refused(deadzone("", "x", 0.5)).contains("invalid device name"));
        for value in [1.01, -0.01, f32::NAN, f32::INFINITY] {
            assert!(apply(XML, &[deadzone(STICK_L, "x", value)]).is_err(), "{value}");
            assert!(apply(XML, &[ConfigChange::MouseAcceleration { value }]).is_err(), "{value}");
        }
        assert!(apply(XML, &[deadzone(STICK_L, "x", 1.004)]).is_ok(), "rounds to 1.00");
        let twice = XML.replace("<deviceoptions name=\"Controller (Gamepad)\">", &format!("<deviceoptions name=\"{STICK_L}\">"));
        assert!(apply(&twice, &[deadzone(STICK_L, "x", 0.5)]).unwrap_err().contains("more than one"));
    }

    // --- XML edge cases -----------------------------------------------------

    #[test]
    fn crlf_tabs_and_a_bom_are_kept() {
        let crlf = format!("\u{feff}{}", XML.replace('\n', "\r\n"));
        let points = vec![[0.0, 0.0], [1.0, 1.0]];
        let changes = [
            curve(DeviceKind::Joystick, 2, "flight_move_pitch", CurveValue::Points { points }),
            deadzone(STICK_R, "x", 0.5),
            invert(1, "flight_move_roll", None),
        ];
        let out = apply(&crlf, &changes).unwrap();
        assert!(out.starts_with('\u{feff}'));
        assert_eq!(out.matches('\n').count(), out.matches("\r\n").count(), "no bare LF introduced");
        let lf = apply(XML, &changes).unwrap();
        assert_eq!(out, format!("\u{feff}{}", lf.replace('\n', "\r\n")));

        // Tab indentation: the new lines follow it.
        let tabs: String = XML
            .lines()
            .map(|l| {
                let n = l.len() - l.trim_start_matches(' ').len();
                format!("{}{}\n", "\t".repeat(n), &l[n..])
            })
            .collect();
        let out = apply(&tabs, &[exponent(2, "flight_move_pitch", 1.5)]).unwrap();
        assert!(out.contains(&format!("\t\t<options type=\"joystick\" instance=\"2\" Product=\"{STICK_R}\">\n\t\t\t<flight_move_pitch exponent=\"1.5\"/>\n\t\t</options>\n")), "{out}");
    }

    #[test]
    fn look_alikes_in_comments_and_cdata_are_left_alone() {
        let xml = XML
            .replace(
                "  <deviceoptions name=\"Mouse\">\n",
                "  <!-- <deviceoptions name=\"Mouse\"><option input=\"@pause_OptionsMouseSmoothing\" saturation=\"9\"/></deviceoptions> -->\n  <deviceoptions name=\"Mouse\">\n",
            )
            .replace(
                "  <options type=\"joystick\" instance=\"1\"",
                "  <!-- <options type=\"joystick\" instance=\"1\"><flight_move_yaw exponent=\"9\"/></options> -->\n  <options type=\"joystick\" instance=\"1\"",
            )
            .replace("   <flight_move_roll invert=\"1\"/>\n", "   <flight_move_roll invert=\"1\"/>\n   <![CDATA[<flight_view exponent=\"9\"/>]]>\n");
        let out = apply(&xml, &[exponent(1, "flight_move_yaw", 0.5), ConfigChange::MouseSmoothing { value: 0.5 }, exponent(1, "flight_view", 1.0)]).unwrap();
        assert!(out.contains("<!-- <options type=\"joystick\" instance=\"1\"><flight_move_yaw exponent=\"9\"/></options> -->"));
        assert!(out.contains("saturation=\"9\"/></deviceoptions> -->"));
        assert!(out.contains("<![CDATA[<flight_view exponent=\"9\"/>]]>"));
        assert!(out.contains("<flight_move_yaw exponent=\"0.5\"/>"));
        assert!(out.contains("<flight_view exponent=\"1\"/>"));
        assert!(out.contains("saturation=\"0.44999999\""));
    }

    #[test]
    fn single_quotes_and_odd_spacing_are_read_and_kept() {
        let xml = XML
            .replace(&format!("<options type=\"joystick\" instance=\"2\" Product=\"{STICK_R}\"/>"), &format!("<options type='joystick' instance='2' Product='{STICK_R}' />"))
            .replace("<option input=\"x\" deadzone=\"0.2277\"/>", "<option input='x' deadzone = '0.2277'/>")
            .replace("<flight_move_roll invert=\"1\"/>", "<flight_move_roll invert='1' />");
        let out = apply(&xml, &[exponent(2, "flight_move_roll", 2.0), deadzone(STICK_L, "x", 0.15), exponent(1, "flight_move_roll", 3.0)]).unwrap();
        assert!(out.contains(&format!("<options type='joystick' instance='2' Product='{STICK_R}'>\n   <flight_move_roll exponent=\"2\"/>\n  </options>")), "{out}");
        assert!(out.contains("<option input='x' deadzone = '0.1485'/>"));
        assert!(out.contains("<flight_move_roll invert='1' exponent=\"3\"/>"));
    }

    #[test]
    fn the_bindings_stay_untouched() {
        let out = apply(XML, &[deadzone(STICK_R, "x", 0.3), exponent(2, "flight_move_roll", 2.0)]).unwrap();
        verify_bindings_untouched(XML, &out).unwrap();
        let changed = out.replace("js1_button1", "js1_button2");
        assert!(verify_bindings_untouched(XML, &changed).unwrap_err().contains("bindings changed"));
        let moved = out.replace("instance=\"2\" Product", "instance=\"3\" Product");
        assert!(verify_bindings_untouched(XML, &moved).is_err());
    }

    #[test]
    fn an_element_with_a_comment_inside_keeps_it_when_its_attributes_go() {
        // Not empty with a comment inside: the attribute goes, the element
        // and the user's comment stay.
        let xml = XML.replace("<flight_move_roll invert=\"1\"/>", "<flight_move_roll invert=\"1\"><!-- keep --></flight_move_roll>");
        let out = apply(&xml, &[invert(1, "flight_move_roll", None)]).unwrap();
        assert_eq!(out, xml.replace("<flight_move_roll invert=\"1\">", "<flight_move_roll>"));
        // While it keeps an attribute, all is well.
        let out = apply(&xml, &[invert(1, "flight_move_roll", Some(false))]).unwrap();
        assert!(out.contains("<flight_move_roll invert=\"0\"><!-- keep --></flight_move_roll>"));
        // A curve set to default beside a comment: the curve goes, the
        // comment stays; and so does a node holding CDATA or text.
        let xml = XML.replace(
            "   <flight_view_yaw exponent=\"2\"/>\n",
            "   <flight_view_yaw>\n    <!-- mine -->\n    <nonlinearity_curve>\n     <point in=\"0\" out=\"0\"/>\n     <point in=\"1\" out=\"1\"/>\n    </nonlinearity_curve>\n   </flight_view_yaw>\n",
        );
        let out = apply(&xml, &[curve(DeviceKind::Joystick, 1, "flight_view_yaw", CurveValue::Default)]).unwrap();
        assert!(out.contains("   <flight_view_yaw>\n    <!-- mine -->\n   </flight_view_yaw>\n"), "{out}");
        for inside in ["<![CDATA[x]]>", "text"] {
            let xml = XML.replace("<flight_move_roll invert=\"1\"/>", &format!("<flight_move_roll invert=\"1\">{inside}</flight_move_roll>"));
            let out = apply(&xml, &[invert(1, "flight_move_roll", None)]).unwrap();
            assert!(out.contains(&format!("<flight_move_roll>{inside}</flight_move_roll>")), "{out}");
        }
    }

    /// The keyboard `fps` curve of [`XML`] with `inside` as its content.
    fn fps_curve(inside: &str) -> String {
        let old = "    <nonlinearity_curve>\n     <point in=\"0\" out=\"0\"/>\n     <point in=\"0.5\" out=\"0.2\"/>\n     <point in=\"1\" out=\"1\"/>\n    </nonlinearity_curve>\n";
        XML.replace(old, &format!("    <nonlinearity_curve>\n{inside}    </nonlinearity_curve>\n"))
    }

    #[test]
    fn a_replaced_curve_keeps_its_comments_in_place() {
        let new = "     <point in=\"0\" out=\"0\"/>\n     <point in=\"0.5\" out=\"0.25\"/>\n     <point in=\"1\" out=\"1\"/>\n";
        let points = || CurveValue::Points { points: vec![[0.0, 0.0], [0.5, 0.25], [1.0, 1.0]] };
        // Comments around and between the points: only the points change.
        let old = "     <!-- mine -->\n     <point in=\"0\" out=\"0\"/>\n     <!-- mid -->\n     <point in=\"0.5\" out=\"0.2\"/>\n     <point in=\"1\" out=\"1\"/>\n     <!-- end -->\n";
        let out = apply(&fps_curve(old), &[curve(DeviceKind::Keyboard, 1, "fps", points())]).unwrap();
        assert_eq!(out, fps_curve(&format!("     <!-- mine -->\n{new}     <!-- mid -->\n     <!-- end -->\n")));
        let fps = &parse_device_config(&out).unwrap().options[0].nodes[0];
        assert_eq!((fps.points.as_ref().map(Vec::len), fps.curve_other), (Some(3), 3));
        // A curve holding only a comment gets the points after it.
        let out = apply(&fps_curve("     <!-- mine -->\n"), &[curve(DeviceKind::Keyboard, 1, "fps", points())]).unwrap();
        assert_eq!(out, fps_curve(&format!("     <!-- mine -->\n{new}")));
    }

    #[test]
    fn a_removed_curve_leaves_its_comment_in_the_curve_element() {
        let commented = fps_curve("     <!-- mine -->\n     <point in=\"0\" out=\"0\"/>\n     <point in=\"0.5\" out=\"0.2\"/>\n     <point in=\"1\" out=\"1\"/>\n");
        let bare = fps_curve("     <!-- mine -->\n");
        // Set Default: the element stays with the comment only.
        let out = apply(&commented, &[curve(DeviceKind::Keyboard, 1, "fps", CurveValue::Default)]).unwrap();
        assert_eq!(out, bare);
        // An exponent instead of the curve: the same, beside the exponent.
        let out = apply(&commented, &[curve(DeviceKind::Keyboard, 1, "fps", CurveValue::Exponent { value: 1.5 })]).unwrap();
        assert_eq!(out, bare.replace("   <fps>\n", "   <fps exponent=\"1.5\">\n"));
        // A curve on a group wipes the one below it, comment kept.
        let xml = XML.replace(
            "   <flight_view_yaw exponent=\"2\"/>\n",
            "   <flight_view_yaw>\n    <nonlinearity_curve>\n     <!-- mine -->\n     <point in=\"0\" out=\"0\"/>\n     <point in=\"1\" out=\"1\"/>\n    </nonlinearity_curve>\n   </flight_view_yaw>\n",
        );
        let out = apply(&xml, &[exponent(1, "flight_view", 1.5)]).unwrap();
        assert!(out.contains("   <flight_view_yaw>\n    <nonlinearity_curve>\n     <!-- mine -->\n    </nonlinearity_curve>\n   </flight_view_yaw>\n"), "{out}");
        // Removing again changes nothing; a curve without any content goes.
        assert_eq!(apply(&bare, &[curve(DeviceKind::Keyboard, 1, "fps", CurveValue::Default)]).unwrap(), bare);
        let empty = fps_curve("");
        let out = apply(&empty, &[curve(DeviceKind::Keyboard, 1, "fps", CurveValue::Exponent { value: 1.5 })]).unwrap();
        assert!(out.contains("   <fps exponent=\"1.5\"/>\n"), "{out}");
    }

    #[test]
    fn a_curve_element_without_points_is_no_custom_curve() {
        for inside in ["     <!-- mine -->\n", "", "     <!-- a -->\n     <!-- b -->\n"] {
            let xml = fps_curve(inside).replace("   <fps>\n", "   <fps exponent=\"2\">\n");
            let view = view(&parse_device_config(&xml).unwrap(), None);
            let fps = &view.options[0].values["fps"];
            assert_eq!((fps.exponent, fps.points.as_ref()), (Some(2.0), None), "{inside:?}");
        }
        // Nothing else set: no value at all.
        let view = view(&parse_device_config(&fps_curve("     <!-- mine -->\n")).unwrap(), None);
        assert!(view.options[0].values.is_empty());
        // Several curve elements: the last one with points counts.
        let xml = fps_curve("     <point in=\"0\" out=\"0\"/>\n     <point in=\"1\" out=\"1\"/>\n").replace(
            "    </nonlinearity_curve>\n   </fps>",
            "    </nonlinearity_curve>\n    <nonlinearity_curve>\n     <!-- mine -->\n    </nonlinearity_curve>\n   </fps>",
        );
        assert_eq!(parse_device_config(&xml).unwrap().options[0].nodes[0].points.as_ref().map(Vec::len), Some(2));
        // As an apply source it is no curve either: the live curve goes.
        let source = fps_curve("     <!-- mine -->\n");
        let out = settings(XML, &source, &[pair(DeviceKind::Keyboard, 1, 1)]).unwrap();
        assert!(!out.contains("<fps>"), "{out}");
    }

    // --- attributes.xml -----------------------------------------------------

    #[test]
    fn attributes_patch_keeps_every_other_byte() {
        let out = apply_attributes(ATTRS, &[attribute(ManagedAttribute::MouseSensitivity, 12.0)]).unwrap();
        assert_eq!(out, ATTRS.replace("value=\"6.06061\"", "value=\"8.88889\""));
        let out = apply_attributes(ATTRS, &[attribute(ManagedAttribute::GamepadSensitivity, 1.2)]).unwrap();
        assert_eq!(out, ATTRS.replace("<Attr name=\"Sensitivity\" value=\"1\"/>", "<Attr name=\"Sensitivity\" value=\"1.2\"/>"));
        // Changes for actionmaps.xml are not this file's business.
        assert!(apply_attributes(ATTRS, &[deadzone(STICK_L, "x", 0.1)]).is_err());
    }

    #[test]
    fn a_missing_attribute_is_inserted_at_its_sorted_position() {
        let out = apply_attributes(
            ATTRS,
            &[
                attribute(ManagedAttribute::AdsPercent, 50.0),
                attribute(ManagedAttribute::ZoomScalingEnabled, 1.0),
                attribute(ManagedAttribute::ZoomScalingPercent, 75.0),
            ],
        )
        .unwrap();
        assert_eq!(
            out,
            concat!(
                "<Attributes Version=\"35\">\n",
                " <Attr name=\"ADSMouseSensitivity\" value=\"0.5\"/>\n",
                " <Attr name=\"AutoZoom\" value=\"1\"/>\n",
                " <Attr name=\"MouseSensitivity\" value=\"6.06061\"/>\n",
                " <Attr name=\"Sensitivity\" value=\"1\"/>\n",
                " <Attr name=\"SysSpec\" value=\"3\"/>\n",
                " <Attr name=\"ZoomSensitivityMultiplier\" value=\"0.75\"/>\n",
                " <Attr name=\"ZoomSensitivityMultiplierToggle\" value=\"1\"/>\n",
                "</Attributes>\n",
            )
        );
    }

    #[test]
    fn attributes_edge_cases() {
        // Duplicates all get the value.
        let dup = ATTRS.replace(" <Attr name=\"SysSpec\"", " <Attr name=\"Sensitivity\" value=\"0.5\"/>\n <Attr name=\"SysSpec\"");
        let out = apply_attributes(&dup, &[attribute(ManagedAttribute::GamepadSensitivity, 2.0)]).unwrap();
        assert_eq!(out.matches("<Attr name=\"Sensitivity\" value=\"2\"/>").count(), 2);
        // CRLF, single quotes.
        let crlf = ATTRS.replace('\n', "\r\n").replace("<Attr name=\"MouseSensitivity\" value=\"6.06061\"/>", "<Attr name='MouseSensitivity' value='6.06061'/>");
        let out = apply_attributes(&crlf, &[attribute(ManagedAttribute::MouseSensitivity, 100.0), attribute(ManagedAttribute::AdsPercent, 100.0)]).unwrap();
        assert!(out.contains("<Attr name='MouseSensitivity' value='40'/>"));
        assert!(out.starts_with("<Attributes Version=\"35\">\r\n <Attr name=\"ADSMouseSensitivity\" value=\"1\"/>\r\n"));
        assert_eq!(out.matches('\n').count(), out.matches("\r\n").count());
        // An empty or self-closing root.
        let out = apply_attributes("<Attributes Version=\"35\"/>\n", &[attribute(ManagedAttribute::AdsPercent, 80.0)]).unwrap();
        assert_eq!(out, "<Attributes Version=\"35\">\n <Attr name=\"ADSMouseSensitivity\" value=\"0.8\"/>\n</Attributes>\n");
        let out = apply_attributes("<Attributes Version=\"35\">\n</Attributes>\n", &[attribute(ManagedAttribute::AdsPercent, 80.0)]).unwrap();
        assert_eq!(out, "<Attributes Version=\"35\">\n <Attr name=\"ADSMouseSensitivity\" value=\"0.8\"/>\n</Attributes>\n");
        // A comment with a look-alike is skipped.
        let commented = ATTRS.replace(" <Attr name=\"AutoZoom\"", " <!-- <Attr name=\"MouseSensitivity\" value=\"1\"/> -->\n <Attr name=\"AutoZoom\"");
        let out = apply_attributes(&commented, &[attribute(ManagedAttribute::MouseSensitivity, 18.0)]).unwrap();
        assert!(out.contains("<!-- <Attr name=\"MouseSensitivity\" value=\"1\"/> -->"));
        assert!(out.contains("<Attr name=\"MouseSensitivity\" value=\"11.0101\"/>"));
        // Refusals.
        assert!(apply_attributes("<ActionMaps/>", &[attribute(ManagedAttribute::AdsPercent, 50.0)]).is_err());
        assert!(apply_attributes(ATTRS, &[attribute(ManagedAttribute::MouseSensitivity, 0.0)]).is_err());
        assert!(apply_attributes(ATTRS, &[]).is_err());
    }

    // --- contract -----------------------------------------------------------

    #[test]
    fn changes_deserialize_from_the_frontends_json() {
        let json = r#"[
            {"type":"curve","kind":"joystick","instance":1,"node":"flight_view","value":{"kind":"exponent","value":1.5}},
            {"type":"curve","kind":"keyboard","instance":1,"node":"fps","value":{"kind":"points","points":[[0,0],[0.5,0.2],[1,1]]}},
            {"type":"curve","kind":"gamepad","instance":1,"node":"fps_view","value":{"kind":"default"}},
            {"type":"invert","kind":"joystick","instance":2,"node":"mining","value":true},
            {"type":"invert","kind":"joystick","instance":2,"node":"mining","value":null},
            {"type":"deadzone","device":"Controller (Gamepad)","input":"thumbl","value":0.65},
            {"type":"saturation","device":" VKB","input":"y","value":0.92},
            {"type":"mouse_acceleration","value":0.5},
            {"type":"mouse_smoothing","value":0.33},
            {"type":"attribute","setting":"mouse_sensitivity","value":12},
            {"type":"attribute","setting":"zoom_scaling_enabled","value":1}
        ]"#;
        let changes: Vec<ConfigChange> = serde_json::from_str(json).unwrap();
        assert_eq!(changes.len(), 11);
        assert_eq!(changes[0], exponent(1, "flight_view", 1.5));
        assert_eq!(changes[2], curve(DeviceKind::Gamepad, 1, "fps_view", CurveValue::Default));
        assert_eq!(changes[4], invert(2, "mining", None));
        assert_eq!(changes[9], attribute(ManagedAttribute::MouseSensitivity, 12.0));
        // The view serializes with snake_case fields and short f32 numbers.
        let text = serde_json::to_string(&view(&parse_device_config(XML).unwrap(), None)).unwrap();
        assert!(text.contains(r#""kind":"keyboard""#));
        assert!(text.contains(r#"{"input":"x","deadzone":0.23,"saturation":null}"#), "{text}");
        assert!(text.contains(r#""zoom_scaling_enabled":null"#));
        assert!(text.contains(r#""has_attributes":false"#));
    }

    #[test]
    fn config_labels_resolve_and_fall_back_to_english() {
        let loc = scdata::parse_localization(concat!(
            "ui_DeadzoneJoystickX=Deadzone Joystick X Axis\r\n",
            "ui_DeadzoneJoystickRotz=Deadzone Joystick Z Rotation\r\n",
            "ui_SaturationJoystickY=Saturation Joystick Y Axis\r\n",
            "ui_DeadzoneXIThumbl=Deadzone Gamepad Thumb Left\r\n",
            "ui_GamePadSensitivity=GamePad Sensitvity\r\n",
            "pause_OptionsMouseSensitivity=Mouse Sensitivity (loc)\r\n",
        ));
        let labels = config_labels(&loc);
        assert_eq!(labels.joystick_deadzone["x"], "Deadzone Joystick X Axis");
        assert_eq!(labels.joystick_deadzone["rotz"], "Deadzone Joystick Z Rotation");
        assert_eq!(labels.joystick_deadzone.len(), 6);
        assert_eq!(labels.joystick_saturation["y"], "Saturation Joystick Y Axis");
        assert_eq!(labels.gamepad_deadzone["thumbl"], "Deadzone Gamepad Thumb Left");
        assert_eq!(labels.gamepad_sensitivity, "GamePad Sensitvity");
        // Keys that do not resolve: the game's English label (as its
        // `global.ini` has it).
        assert_eq!(labels.joystick_deadzone["y"], "Deadzone Joystick Y Axis");
        assert_eq!(labels.joystick_deadzone["rotx"], "Deadzone Joystick X Rotation");
        assert_eq!(labels.joystick_saturation["z"], "Saturation Joystick Z Axis");
        assert_eq!(labels.joystick_saturation["rotz"], "Saturation Joystick Z Rotation");
        assert_eq!(labels.gamepad_deadzone["thumbr"], "Deadzone Gamepad Thumb Right");
        let empty = config_labels(&HashMap::new());
        assert_eq!(empty.gamepad_sensitivity, "GamePad Sensitvity");
        assert_eq!(empty.joystick_deadzone["x"], "Deadzone Joystick X Axis");
        assert_eq!(empty.joystick_saturation["roty"], "Saturation Joystick Y Rotation");
        assert_eq!(empty.gamepad_deadzone["thumbl"], "Deadzone Gamepad Thumb Left");
        assert_eq!(empty.mouse_zoom_scaling_enabled, "Mouse Sensitivity - ADS - Zoom Scaling Enabled");
        assert_eq!(labels.mouse_sensitivity, "Mouse Sensitivity (loc)");
        assert_eq!(labels.mouse_ads_percent, "Mouse Sensitivity - ADS - %");
        assert_eq!(labels.mouse_smoothing, "Mouse Smoothing");
    }

    // --- apply_settings / copy_attributes ------------------------------------

    fn pair(kind: DeviceKind, source: u32, target: u32) -> SlotPair {
        SlotPair { kind, source, target }
    }

    fn settings(live: &str, source: &str, pairs: &[SlotPair]) -> Result<String, String> {
        apply_settings(live, &parse_device_config(source).unwrap(), pairs, &trees())
    }

    const JS1_CHILDREN: &str = concat!(
        "   <flight_move_yaw exponent=\"2\"/>\n",
        "   <flight_move_roll invert=\"1\"/>\n",
        "   <flight_view exponent=\"0.69999999\"/>\n",
        "   <flight_view_pitch invert=\"1\" exponent=\"2\"/>\n",
        "   <flight_view_yaw exponent=\"2\"/>\n",
        "   <mining_throttle invert=\"0\"/>\n",
    );

    #[test]
    fn settings_land_on_the_target_slot_and_device() {
        // Source js1 onto live js2: js2's slot gets js1's children in tree
        // order, js2's device (another Product) js1's deadzones; js1 itself
        // stays as it is.
        let out = settings(XML, XML, &[pair(DeviceKind::Joystick, 1, 2)]).unwrap();
        let js2 = format!("  <options type=\"joystick\" instance=\"2\" Product=\"{STICK_R}\"/>\n");
        let expected = XML
            .replace(&js2, &format!("  <options type=\"joystick\" instance=\"2\" Product=\"{STICK_R}\">\n{JS1_CHILDREN}  </options>\n"))
            .replace(
                "  </deviceoptions>\n  <options type=\"keyboard\"",
                &format!(
                    "  </deviceoptions>\n  <deviceoptions name=\"{STICK_R}\">\n   <option input=\"x\" deadzone=\"0.2277\"/>\n   <option input=\"y\" saturation=\"0.49500001\"/>\n  </deviceoptions>\n  <options type=\"keyboard\""
                ),
            );
        assert_eq!(out, expected);
        // Taken over again: nothing differs any more.
        assert_eq!(settings(&out, XML, &[pair(DeviceKind::Joystick, 1, 2)]).unwrap(), out);
        // A source identical to the live file changes nothing at all.
        let all = [pair(DeviceKind::Keyboard, 1, 1), pair(DeviceKind::Gamepad, 1, 1), pair(DeviceKind::Joystick, 1, 1), pair(DeviceKind::Joystick, 2, 2)];
        assert_eq!(settings(XML, XML, &all).unwrap(), XML);
    }

    #[test]
    fn live_settings_the_source_lacks_are_removed() {
        let source = XML
            .replace(
                JS1_CHILDREN,
                concat!(
                    "   <flight_move_roll invert=\"0\"/>\n",
                    "   <flight_view_pitch>\n",
                    "    <nonlinearity_curve>\n",
                    "     <point in=\"0\" out=\"0\"/>\n",
                    "     <point in=\"0.5\" out=\"0.25\"/>\n",
                    "     <point in=\"1\" out=\"1\"/>\n",
                    "    </nonlinearity_curve>\n",
                    "   </flight_view_pitch>\n",
                ),
            )
            .replace(
                concat!(
                    "   <option input=\"x\" deadzone=\"0.2277\"/>\n",
                    "   <option input=\"y\" saturation=\"0.91079998\"/>\n",
                    "   <option input=\"y\" saturation=\"0.49500001\"/>\n",
                ),
                "   <option input=\"x\" deadzone=\"0.495\"/>\n",
            );
        let out = settings(XML, &source, &[pair(DeviceKind::Joystick, 1, 1)]).unwrap();
        let mut expected = parse_device_config(XML).unwrap();
        let js1 = expected.options.iter_mut().find(|o| o.kind == DeviceKind::Joystick && o.instance == 1).unwrap();
        let pt = |i: &str, o: &str| (i.to_string(), o.to_string());
        js1.nodes = vec![
            NodeEntry { name: "flight_move_roll".into(), attrs: vec![("invert".into(), "0".into())], points: None, other_children: 0, curve_other: 0 },
            NodeEntry {
                name: "flight_view_pitch".into(),
                attrs: Vec::new(),
                points: Some(vec![pt("0", "0"), pt("0.5", "0.25"), pt("1", "1")]),
                other_children: 0,
                curve_other: 0,
            },
        ];
        expected.device_options[0].entries = vec![OptionEntry { input: "x".into(), attrs: vec![("deadzone".into(), "0.495".into())] }];
        assert_eq!(parse_device_config(&out).unwrap(), expected);
        // The duplicates are gone as lines, nothing left behind.
        assert!(!out.contains("saturation=\"0.91079998\"") && !out.contains("saturation=\"0.49500001\""));
        assert!(out.contains("   <flight_view_pitch>\n    <nonlinearity_curve>\n     <point in=\"0\" out=\"0\"/>\n"), "{out}");

        // A source without any settings for the device empties both; the
        // device element goes with its lines, the slot stays self-closing.
        let source = XML.replace(JS1_CHILDREN, "").replace(
            concat!(
                "   <option input=\"x\" deadzone=\"0.2277\"/>\n",
                "   <option input=\"y\" saturation=\"0.91079998\"/>\n",
                "   <option input=\"y\" saturation=\"0.49500001\"/>\n",
            ),
            "",
        );
        let out = settings(XML, &source, &[pair(DeviceKind::Joystick, 1, 1)]).unwrap();
        let expected = XML
            .replace(
                &format!(
                    "  <deviceoptions name=\"{STICK_L}\">\n   <option input=\"x\" deadzone=\"0.2277\"/>\n   <option input=\"y\" saturation=\"0.91079998\"/>\n   <option input=\"y\" saturation=\"0.49500001\"/>\n  </deviceoptions>\n"
                ),
                "",
            )
            .replace(
                &format!("  <options type=\"joystick\" instance=\"1\" Product=\"{STICK_L}\">\n{JS1_CHILDREN}  </options>\n"),
                &format!("  <options type=\"joystick\" instance=\"1\" Product=\"{STICK_L}\"/>\n"),
            );
        assert_eq!(out, expected);
        // CRLF and tabs: the line goes the same way.
        let crlf = |x: &str| x.replace("\n  <deviceoptions", "\n\t<deviceoptions").replace('\n', "\r\n");
        assert_eq!(settings(&crlf(XML), &source, &[pair(DeviceKind::Joystick, 1, 1)]).unwrap(), crlf(&expected));
    }

    #[test]
    fn an_emptied_device_element_goes_the_mouse_and_the_pad_too() {
        let source = XML
            .replace("   <option input=\"thumbl\" deadzone=\"0.58434999\"/>\n", "")
            .replace("   <option input=\"@pause_OptionsMouseAcceleration\" acceleration=\"0.077\"/>\n", "")
            .replace("   <option input=\"@pause_OptionsMouseSmoothing\" saturation=\"0.29699999\"/>\n", "");
        let out = settings(XML, &source, &[pair(DeviceKind::Keyboard, 1, 1), pair(DeviceKind::Gamepad, 1, 1)]).unwrap();
        assert!(!out.contains("<deviceoptions name=\"Controller (Gamepad)\"") && !out.contains("<deviceoptions name=\"Mouse\""), "{out}");
        assert!(out.contains("  </deviceoptions>\n  <options type=\"keyboard\""), "{out}");
        let parsed = parse_device_config(&out).unwrap();
        assert_eq!(parsed.device_options.len(), 1);
        // Taken over again: nothing left to remove, nothing written.
        assert_eq!(settings(&out, &source, &[pair(DeviceKind::Gamepad, 1, 1)]).unwrap(), out);
    }

    #[test]
    fn a_device_element_with_a_comment_inside_stays() {
        let live = XML.replace(
            "   <option input=\"thumbl\" deadzone=\"0.58434999\"/>\n",
            "   <!-- my pad -->\n   <option input=\"thumbl\" deadzone=\"0.58434999\"/>\n",
        );
        let source = XML.replace("   <option input=\"thumbl\" deadzone=\"0.58434999\"/>\n", "");
        let out = settings(&live, &source, &[pair(DeviceKind::Gamepad, 1, 1)]).unwrap();
        assert_eq!(out, XML.replace("   <option input=\"thumbl\" deadzone=\"0.58434999\"/>\n", "   <!-- my pad -->\n"));
    }

    #[test]
    fn a_device_element_is_refilled_before_it_is_emptied() {
        // Live x deadzone, source y saturation only: the new value is
        // written first, so the element never empties and keeps its place.
        let options = concat!(
            "   <option input=\"x\" deadzone=\"0.2277\"/>\n",
            "   <option input=\"y\" saturation=\"0.91079998\"/>\n",
            "   <option input=\"y\" saturation=\"0.49500001\"/>\n",
        );
        let live = XML.replace(options, "   <option input=\"x\" deadzone=\"0.2277\"/>\n");
        let source = XML.replace(options, "   <option input=\"y\" saturation=\"0.49500001\"/>\n");
        let out = settings(&live, &source, &[pair(DeviceKind::Joystick, 1, 1)]).unwrap();
        assert_eq!(out, source);
    }

    #[test]
    fn gamepad_and_mouse_settings_follow_the_source() {
        let source = XML
            .replace("   <option input=\"thumbl\" deadzone=\"0.58434999\"/>\n", "   <option input=\"thumbr\" deadzone=\"0.44949999\"/>\n")
            .replace("acceleration=\"0.077\"", "acceleration=\"0.050000001\"")
            .replace("   <option input=\"@pause_OptionsMouseSmoothing\" saturation=\"0.29699999\"/>\n", "");
        let kb_open = "  <options type=\"keyboard\" instance=\"1\" Product=\"Keyboard  {6F1D2B61-D5A0-11CF-BFC7-444553540000}\">\n";
        let kb_block_start = XML.find(kb_open).unwrap();
        let kb_block_end = XML[kb_block_start..].find("  </options>\n").unwrap() + kb_block_start + "  </options>\n".len();
        let kb_block = &XML[kb_block_start..kb_block_end];
        let source = source.replace(kb_block, &kb_open.replace("\">\n", "\"/>\n"));

        let pairs = [pair(DeviceKind::Keyboard, 1, 1), pair(DeviceKind::Gamepad, 1, 1)];
        let out = settings(XML, &source, &pairs).unwrap();
        // The gamepad's thumbr comes, then thumbl goes (the element never
        // empties); the mouse's acceleration changes, the smoothing goes;
        // the keyboard slot's curve goes.
        assert_eq!(out, source);

        // CRLF and a BOM survive the same rewrite.
        let crlf = format!("\u{feff}{}", XML.replace('\n', "\r\n"));
        let out = settings(&crlf, &source, &pairs).unwrap();
        assert_eq!(out, format!("\u{feff}{}", source.replace('\n', "\r\n")));
    }

    #[test]
    fn settings_apply_refuses_what_it_cannot_take_over() {
        // No such live slot: refused when the source has settings for it,
        // nothing to do when it has none.
        let err = settings(XML, XML, &[pair(DeviceKind::Joystick, 1, 3)]).unwrap_err();
        assert!(err.contains("no js3"), "{err}");
        assert_eq!(settings(XML, XML, &[pair(DeviceKind::Joystick, 2, 3)]).unwrap(), XML);
        // Values that are no game values, or would need escaping.
        for (from, to) in [
            ("<flight_move_roll invert=\"1\"/>", "<flight_move_roll invert=\"2\"/>"),
            ("<flight_view exponent=\"0.69999999\"/>", "<flight_view exponent=\"9\"/>"),
            ("<flight_view exponent=\"0.69999999\"/>", "<flight_view exponent=\"0.7&quot; x=&quot;1\"/>"),
            ("deadzone=\"0.2277\"", "deadzone=\"0.2&lt;\""),
            ("deadzone=\"0.2277\"", "deadzone=\"1.5\""),
            ("<point in=\"0.5\" out=\"0.2\"/>", "<point in=\"0.5\" out=\"NaN\"/>"),
        ] {
            let source = XML.replace(from, to);
            assert_ne!(source, XML, "{from}");
            let pairs = [pair(DeviceKind::Joystick, 1, 2), pair(DeviceKind::Keyboard, 1, 1)];
            assert!(settings(XML, &source, &pairs).is_err(), "{to} was taken over");
        }
    }

    #[test]
    fn attributes_copy_writes_inserts_and_removes_only_the_managed_ones() {
        let source: BTreeMap<String, String> =
            [("MouseSensitivity", "8.88889"), ("ADSMouseSensitivity", "0.5"), ("SysSpec", "1")].map(|(k, v)| (k.to_string(), v.to_string())).into();
        let out = copy_attributes(ATTRS, &source, managed_attributes(DeviceKind::Keyboard)).unwrap();
        assert_eq!(
            out,
            ATTRS
                .replace("value=\"6.06061\"", "value=\"8.88889\"")
                .replace(" <Attr name=\"AutoZoom\"", " <Attr name=\"ADSMouseSensitivity\" value=\"0.5\"/>\n <Attr name=\"AutoZoom\"")
        );
        // The gamepad's setting is not in the source: it goes; SysSpec is
        // never touched, whatever the source holds.
        let out = copy_attributes(ATTRS, &source, managed_attributes(DeviceKind::Gamepad)).unwrap();
        assert_eq!(out, ATTRS.replace(" <Attr name=\"Sensitivity\" value=\"1\"/>\n", ""));
        // Nothing differs: the same text.
        let same: BTreeMap<String, String> = [("MouseSensitivity", "6.06061")].map(|(k, v)| (k.to_string(), v.to_string())).into();
        assert_eq!(copy_attributes(ATTRS, &same, &[ManagedAttribute::MouseSensitivity, ManagedAttribute::AdsPercent]).unwrap(), ATTRS);
        assert!(managed_attributes(DeviceKind::Joystick).is_empty());
        // A value that would need escaping is refused.
        let bad: BTreeMap<String, String> = [("Sensitivity", "1\" x=\"")].map(|(k, v)| (k.to_string(), v.to_string())).into();
        assert!(copy_attributes(ATTRS, &bad, managed_attributes(DeviceKind::Gamepad)).is_err());
    }

    // --- zones and the report ------------------------------------------------

    #[test]
    fn device_names_map_onto_hardware_ids() {
        assert_eq!(hardware_id_of(STICK_L).as_deref(), Some("{0201231D-0000-0000-0000-504944564944}"));
        assert_eq!(hardware_id_of(" VKB  {0201231d-0000-0000-0000-504944564944}").as_deref(), Some("{0201231D-0000-0000-0000-504944564944}"));
        assert_eq!(hardware_id_of(GAMEPAD_DEVICE).as_deref(), Some("gamepad"));
        assert_eq!(hardware_id_of(MOUSE_DEVICE), None);
        assert_eq!(hardware_id_of("Some Stick"), None);
    }

    #[test]
    fn zones_are_the_stored_values_with_the_last_duplicate_winning() {
        let xml = XML.replace(
            "   <option input=\"thumbl\" deadzone=\"0.58434999\"/>\n",
            "   <option input=\"thumbl\" deadzone=\"0.58434999\"/>\n   <option input=\"thumbr\" saturation=\"0.5\"/>\n   <option input=\"thumbr\" deadzone=\"oops\"/>\n",
        );
        let xml = xml.replace("   <option input=\"x\" deadzone=\"0.2277\"/>\n", "   <option input=\"x\" deadzone=\"0.2277\"/>\n   <option input=\"slider1\" deadzone=\"0.3\"/>\n");
        let zones = axis_zones(&parse_device_config(&xml).unwrap());
        let stick = &zones["{0201231D-0000-0000-0000-504944564944}"];
        assert_eq!(stick["x"], crate::input::AxisZone { deadzone: Some(0.2277), saturation: None });
        assert_eq!(stick["y"], crate::input::AxisZone { deadzone: None, saturation: Some(0.495) });
        assert_eq!(stick.len(), 2, "no slider: not one of the game's six axes");
        // The gamepad: deadzones only, an unreadable value left out.
        assert_eq!(zones["gamepad"].len(), 1);
        assert_eq!(zones["gamepad"]["thumbl"].deadzone, Some(0.58435));
        // The mouse has no axes here.
        assert_eq!(zones.len(), 2);
    }

    #[test]
    fn config_text_holds_the_elements_and_the_managed_attributes_verbatim() {
        let xml = XML.replace("  <modifiers />\n", "  <!-- <options type=\"joystick\" instance=\"9\"/> -->\n  <modifiers />\n");
        let first = XML.find("  <deviceoptions").unwrap();
        let last = XML.find("  <modifiers").unwrap();
        let elements: String = XML[first..last].lines().map(|l| format!("    {l}\n")).collect();
        assert_eq!(config_text(&xml, None).unwrap(), format!("bindings file\n{elements}settings file\n    not found\n"));
        assert_eq!(
            config_text(&xml, Some(Ok(ATTRS.to_string()))).unwrap(),
            format!(
                "bindings file\n{elements}settings file\n    <Attr name=\"MouseSensitivity\" value=\"6.06061\"/>\n    <Attr name=\"Sensitivity\" value=\"1\"/>\n"
            )
        );
        let none = config_text("<ActionMaps/>", Some(Err("denied".into()))).unwrap();
        assert_eq!(none, "bindings file\n    none\nsettings file\n    error: denied\n");
        assert!(config_text(&xml, Some(Ok("<Attributes></Attributes>".into()))).unwrap().ends_with("settings file\n    none\n"));
    }
}
