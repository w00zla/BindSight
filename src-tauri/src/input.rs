//! Live device input via SDL: the raw joystick API for sticks and throttles,
//! the GameController API for gamepads.
//!
//! A single background thread owns the one SDL context (rust-sdl2 allows only
//! one at a time), keeps every connected device open so their events are
//! reported, pumps the event loop and forwards each button/axis/hat change to
//! the frontend as a `joy-input` Tauri event. It also maintains the shared
//! device list and emits `devices-changed` on hot-plug; the `list_devices`
//! command only reads that list, so a second SDL context never exists.
//!
//! A device SDL recognises as a game controller is opened twice: as a joystick
//! (for the raw facts in [`DeviceInfo`]) and as a controller (for the events).
//! Its raw `Joy*` events are dropped so every pad input is reported once, under
//! its SC name (`a`, `dpad_up`, `thumblx`) rather than an index. SC binds
//! exactly one pad, so the first one in SDL index order gets the `gp1` slot.
//!
//! The keyboard is a synthetic entry appended to the device list: SC has one
//! `kb1` and the actual key events are captured in the webview, not here.
//!
//! The standalone [`enumerate`] path (used by the examples) makes its own
//! short-lived context and must not run while the app's input thread is alive.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use log::{error, info, warn};
use sdl2::controller::{Axis, Button, GameController};
use sdl2::event::Event;
use sdl2::joystick::{HatState, Joystick};
use sdl2::{GameControllerSubsystem, JoystickSubsystem};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::guid::sdl_guid_to_sc_product;
use crate::scdata::DeviceKind;

/// Only forward an axis once it has moved more than this since the last
/// forwarded value, so continuous jitter does not flood the frontend.
const AXIS_EMIT_THRESHOLD: i32 = 3000;

/// And at most this often per axis (SDL event time, ms): two sticks worked
/// at once produce hundreds of motion events a second, more than the
/// webview can pulse, resolve and log without lagging. A return to the
/// centre always goes through, so a rested axis never looks held.
const AXIS_MIN_INTERVAL_MS: u32 = 20;

/// App-side resting zones around an axis centre (the game's own per-device
/// deadzones from `<deviceoptions>` are not read yet): a value inside the
/// zone counts as the centre, so a stick at rest produces no event at all —
/// nothing pulses, resolves or gets recorded. Joysticks: ~12 % of travel.
/// Gamepads: Microsoft's XInput constants, rounded —
/// `XINPUT_GAMEPAD_LEFT_THUMB_DEADZONE` 7849 and
/// `XINPUT_GAMEPAD_RIGHT_THUMB_DEADZONE` 8689 (~25 %), and
/// `XINPUT_GAMEPAD_TRIGGER_THRESHOLD` 30 of 255 (~12 %).
const JOYSTICK_DEADZONE: i16 = 4000;
const PAD_THUMB_DEADZONE: i16 = 8000;
const PAD_TRIGGER_DEADZONE: i16 = 4000;

/// Near full travel: the point past which a gamepad trigger or thumb stick
/// direction counts as a pressed button (SC's `triggerl_btn`, `thumbl_left`,
/// …, which have no axis of their own). A trigger presses right there, like
/// a shoulder button; a stick direction only after [`STICK_HOLD_MS`]. SC's
/// own rule is unknown; half and three quarter travel both felt too early
/// (user, 2026-09-12).
const DERIVED_BUTTON_THRESHOLD: i16 = 30000;

/// How long a thumb stick must stay in a corner before its direction button
/// presses: the stick is an axis first, and only a deliberate hold turns it
/// into `thumbl_left` & co. — a sweep through the corner must not (user,
/// 2026-09-12).
const STICK_HOLD_MS: u32 = 500;

/// A derived button lets go only below this (hysteresis): a stick held right
/// at the threshold would otherwise flutter between pressed and released.
const DERIVED_BUTTON_RELEASE: i16 = 24000;

/// How long the event loop waits for an event before it checks the derived
/// buttons that are past the threshold but not yet pressed — a stick held
/// still sends no event of its own.
const LOOP_TICK_MS: u32 = 25;

/// SDL index of the synthetic keyboard entry — far beyond any real device, so
/// it never collides and always sorts last.
const KEYBOARD_INDEX: u32 = 10000;

/// The image-map/hardware key of the one keyboard and the one gamepad SC
/// knows. Joysticks use their SC Product GUID instead.
const KEYBOARD_HARDWARE_ID: &str = "keyboard";
const GAMEPAD_HARDWARE_ID: &str = "gamepad";

/// One hidapi interface behind the device's USB vendor/product (a device can
/// expose several: joystick, keyboard, vendor-specific …). Device-log only.
#[derive(Debug, Clone, Default, Serialize)]
pub struct HidInterface {
    pub path: String,
    pub interface_number: i32,
    pub usage_page: u16,
    pub usage: u16,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial: Option<String>,
    /// `bcdDevice`, the device release number.
    pub release: u16,
    pub bus_type: String,
}

/// A connected device as BindSight sees it. Everything below `axes_error`
/// is troubleshooting detail for the Device Info view.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DeviceInfo {
    /// Joystick, gamepad, or the synthetic keyboard.
    pub kind: DeviceKind,
    /// The key for image-maps and `imagemap_choices`: a joystick's SC Product
    /// GUID, the fixed `"gamepad"` for the pad holding the `gp1` slot, the
    /// fixed `"keyboard"`, and `None` for anything that cannot be bound
    /// (a further pad, a joystick with an unparseable GUID).
    pub hardware_id: Option<String>,
    /// `Some(1)` for the pad on SC's one `gp1` slot: Windows the first game
    /// controller in SDL index order, Linux the first gamepad in Wine's key
    /// order (XInput's user 0). `None` for every further pad and for non-pads.
    pub gamepad_slot: Option<u32>,
    /// `SDL_GameControllerName` for pads, `None` otherwise. Usually friendlier
    /// than the raw joystick name.
    pub controller_name: Option<String>,
    /// Linux: the game's XInput gamepad by winebus's rule (an SDL device
    /// with exactly 6 axes and 14+ buttons, or an SDL controller that is
    /// no wheel / flight stick) although SDL has no controller mapping for
    /// it; its raw input is mapped like Wine maps it (`wine_pad_button`,
    /// `wine_pad_axis`). False for a real SDL controller and on Windows.
    pub wine_gamepad: bool,
    /// SDL enumeration index (NOT SC's `jsN` instance number — the two differ).
    pub index: u32,
    /// The device name exactly as SC shows it: the HID product string
    /// (iProduct), matched by vendor/product ID. `None` if no HID match.
    pub sc_name: Option<String>,
    /// SDL's own (evdev) name — differs from SC's; kept for debugging only.
    pub sdl_name: String,
    /// SDL's 32-hex-char GUID.
    pub sdl_guid: String,
    /// SC `options/@Product` GUID derived from the SDL GUID, or `None` if the
    /// GUID could not be parsed.
    pub sc_product_guid: Option<String>,
    pub num_buttons: u32,
    pub num_axes: u32,
    pub num_hats: u32,
    /// SC axis name per SDL axis index (`x`, `rotz`, `slider1`, …), derived
    /// from the HID report descriptor (see `hid.rs`). Empty when that failed;
    /// `axes_error` then says why.
    pub axes: Vec<String>,
    pub axes_error: Option<String>,
    /// SDL's per-session instance id (the `which` of its events).
    pub sdl_instance_id: u32,
    pub sdl_vendor: u16,
    pub sdl_product: u16,
    pub sdl_product_version: u16,
    /// `SDL_JoystickGetSerial`, `None` when SDL knows none.
    pub sdl_serial: Option<String>,
    /// SDL's device type guess (`flight_stick`, `throttle`, `unknown`, …).
    pub sdl_type: String,
    /// The OS device path SDL opened (evdev node / DirectInput path).
    pub sdl_path: Option<String>,
    pub power_level: String,
    pub num_balls: u32,
    pub has_rumble: bool,
    pub has_led: bool,
    /// Every hidapi interface with the same vendor/product.
    pub hid_interfaces: Vec<HidInterface>,
    /// The joystick interface's report descriptor, hex.
    pub hid_descriptor: Option<String>,
    /// Its input fields in report order (`X Y Rz Z Btn1 … Hat`).
    pub hid_usages: Vec<String>,
}

/// Shared, hot-pluggable device list, maintained by the input thread and read
/// by the `list_devices` command.
pub type DeviceList = Arc<Mutex<Vec<DeviceInfo>>>;

/// A device hidapi lists with a joystick / gamepad / multi-axis interface
/// that SDL does not list (Device List only): the user sees why it is in
/// no Monitor. `product_guid` is SC's Product GUID for the vendor /
/// product, the key the Wine rows match by.
#[derive(Debug, Clone, Serialize)]
pub struct HidOnlyDevice {
    pub vid: u16,
    pub pid: u16,
    pub product_guid: String,
    pub name: Option<String>,
    /// The joystick interface's HID usage (4 joystick, 5 gamepad, 8 multi-axis).
    pub usage: u16,
    pub path: String,
    /// Every hidapi interface with the same vendor / product.
    pub interfaces: Vec<HidInterface>,
}

/// The hidapi joystick-class devices whose vendor / product SDL's list
/// (`listed`) lacks, in hidapi's order.
pub fn hid_only_devices(listed: &[DeviceInfo]) -> Vec<HidOnlyDevice> {
    let table = hid_table();
    let mut out: Vec<HidOnlyDevice> = Vec::new();
    for ((vid, pid), info) in &table.map {
        let Some(path) = &info.joystick_path else { continue };
        if listed.iter().any(|d| d.kind != DeviceKind::Keyboard && d.sdl_vendor == *vid && d.sdl_product == *pid) {
            continue;
        }
        let path = path.to_string_lossy().into_owned();
        // hidapi lists one entry per usage of a node: take the joystick-class one.
        let usage = info
            .interfaces
            .iter()
            .find(|i| i.path == path && i.usage_page == 0x01 && matches!(i.usage, 4 | 5 | 8))
            .map_or(0, |i| i.usage);
        out.push(HidOnlyDevice {
            vid: *vid,
            pid: *pid,
            product_guid: crate::wineorder::product_guid(*vid, *pid),
            name: info.name.clone(),
            usage,
            path,
            interfaces: info.interfaces.clone(),
        });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// A single live input change, forwarded to the frontend as a `joy-input`
/// event. `guid` is the device's SDL GUID, the join key to [`DeviceInfo`].
/// `timestamp` is SDL's event time (ms since SDL init), `instance_id` the
/// SDL joystick instance the event came from. Pad events carry SC's input
/// name instead of an index; the frontend adds `key` events of the same shape
/// for the keyboard.
#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum InputEvent {
    Button { guid: String, index: u8, pressed: bool, timestamp: u32, instance_id: u32 },
    Axis { guid: String, index: u8, value: i16, timestamp: u32, instance_id: u32 },
    Hat { guid: String, index: u8, direction: String, raw: u8, timestamp: u32, instance_id: u32 },
    PadButton { guid: String, name: String, pressed: bool, timestamp: u32, instance_id: u32 },
    PadAxis { guid: String, name: String, value: i16, timestamp: u32, instance_id: u32 },
}

/// SC's name for an SDL controller button. `Guide`, `Misc1`, `Paddle1..4` and
/// `Touchpad` have no SC token — they are still reported so the Device Info view and
/// the image-map editor see every press.
fn pad_button_name(button: Button) -> &'static str {
    match button {
        Button::A => "a",
        Button::B => "b",
        Button::X => "x",
        Button::Y => "y",
        Button::Back => "back",
        Button::Start => "start",
        Button::Guide => "guide",
        Button::LeftShoulder => "shoulderl",
        Button::RightShoulder => "shoulderr",
        Button::LeftStick => "thumbl",
        Button::RightStick => "thumbr",
        Button::DPadUp => "dpad_up",
        Button::DPadDown => "dpad_down",
        Button::DPadLeft => "dpad_left",
        Button::DPadRight => "dpad_right",
        Button::Misc1 => "misc1",
        Button::Paddle1 => "paddle1",
        Button::Paddle2 => "paddle2",
        Button::Paddle3 => "paddle3",
        Button::Paddle4 => "paddle4",
        Button::Touchpad => "touchpad",
    }
}

/// SC's name for an SDL controller axis.
fn pad_axis_name(axis: Axis) -> &'static str {
    match axis {
        Axis::LeftX => "thumblx",
        Axis::LeftY => "thumbly",
        Axis::RightX => "thumbrx",
        Axis::RightY => "thumbry",
        Axis::TriggerLeft => "triggerl",
        Axis::TriggerRight => "triggerr",
    }
}

/// The resting zone of a pad axis, see [`JOYSTICK_DEADZONE`].
fn pad_axis_deadzone(axis: Axis) -> i16 {
    match axis {
        Axis::LeftX | Axis::LeftY | Axis::RightX | Axis::RightY => PAD_THUMB_DEADZONE,
        Axis::TriggerLeft | Axis::TriggerRight => PAD_TRIGGER_DEADZONE,
    }
}

/// Whether an axis motion is worth forwarding: moved by more than the jitter
/// threshold since the last forwarded value, and either back at the centre
/// or at least [`AXIS_MIN_INTERVAL_MS`] after the last forwarded event.
fn axis_due(value: i16, prev: i16, timestamp: u32, last_at: Option<u32>) -> bool {
    if (value as i32 - prev as i32).abs() <= AXIS_EMIT_THRESHOLD {
        return false;
    }
    value == 0 || last_at.is_none_or(|t| timestamp.wrapping_sub(t) >= AXIS_MIN_INTERVAL_MS)
}

/// The other axis of a pad stick, for the dominance rule.
fn pad_stick_partner(axis: Axis) -> Option<Axis> {
    match axis {
        Axis::LeftX => Some(Axis::LeftY),
        Axis::LeftY => Some(Axis::LeftX),
        Axis::RightX => Some(Axis::RightY),
        Axis::RightY => Some(Axis::RightX),
        Axis::TriggerLeft | Axis::TriggerRight => None,
    }
}

/// The other axis of a joystick's stick (SC's `x` / `y`), by the device's
/// SC axis names; anything else (twist, sliders, a mini stick) stands alone.
fn joystick_stick_partner(axes: &[String], idx: u8) -> Option<u8> {
    let other = match axes.get(idx as usize)?.as_str() {
        "x" => "y",
        "y" => "x",
        _ => return None,
    };
    axes.iter().position(|a| a == other).map(|i| i as u8)
}

/// Whether an axis leads its stick: a stick pushed diagonally moves both
/// axes, and only the one deflected further is forwarded, so what the card
/// shows and what the image lights is one input, not two taking turns. The
/// centre (0) always passes, and an axis without a partner always leads.
fn leads_stick(value: i16, partner: Option<i16>) -> bool {
    value == 0 || partner.is_none_or(|p| (value as i32).abs() >= (p as i32).abs())
}

/// An axis value with its resting zone applied: the centre inside the zone,
/// the raw value outside it (no rescaling — what the device reports is what
/// the event carries).
fn apply_deadzone(value: i16, deadzone: i16) -> i16 {
    if (value as i32).abs() < deadzone as i32 {
        0
    } else {
        value
    }
}

/// The buttons SC derives from a pad axis, as `(name, travel)`: the axis
/// value as travel towards that button (positive = pressing it). Triggers
/// have one, thumb sticks one per direction; SDL's Y axis is negative
/// upwards.
fn derived_pad_buttons(axis: Axis, value: i16) -> Vec<(&'static str, i32)> {
    let towards = value as i32;
    let away = -(value as i32);
    let (low, high) = match axis {
        Axis::LeftX => ("thumbl_left", "thumbl_right"),
        Axis::LeftY => ("thumbl_up", "thumbl_down"),
        Axis::RightX => ("thumbr_left", "thumbr_right"),
        Axis::RightY => ("thumbr_up", "thumbr_down"),
        Axis::TriggerLeft => return vec![("triggerl_btn", towards)],
        Axis::TriggerRight => return vec![("triggerr_btn", towards)],
    };
    vec![(low, away), (high, towards)]
}

/// What a derived button does at this travel, given whether it is pressed:
/// a pressed one lets go below the release point, an unpressed one starts
/// (or keeps) waiting past the threshold and stops waiting below it. The
/// press itself happens once the wait is over, see [`derived_due`].
#[derive(Debug, PartialEq)]
enum DerivedStep {
    Release,
    Wait,
    Idle,
    Hold,
}

fn derived_step(travel: i32, pressed: bool) -> DerivedStep {
    if pressed {
        if travel > DERIVED_BUTTON_RELEASE as i32 { DerivedStep::Hold } else { DerivedStep::Release }
    } else if travel >= DERIVED_BUTTON_THRESHOLD as i32 {
        DerivedStep::Wait
    } else {
        DerivedStep::Idle
    }
}

/// A trigger button presses the moment its axis is past the threshold; a
/// stick direction waits out [`STICK_HOLD_MS`] first.
fn presses_at_once(name: &str) -> bool {
    name.starts_with("trigger")
}

/// Whether a stick direction waiting since `since` (SDL ticks) presses at
/// `now`.
fn derived_due(since: u32, now: u32) -> bool {
    now.wrapping_sub(since) >= STICK_HOLD_MS
}

/// SC's `triggerl_r_btn` ("Left and Right Trigger", the Melee Block default):
/// pressed while both derived trigger buttons are.
fn both_triggers(derived: &HashMap<(u32, &'static str), bool>, which: u32) -> bool {
    derived.get(&(which, "triggerl_btn")).copied().unwrap_or(false)
        && derived.get(&(which, "triggerr_btn")).copied().unwrap_or(false)
}

/// The synthetic keyboard entry, appended last to every device list. SC knows
/// exactly one keyboard and never logs it, so it is always "there"; its key
/// events are captured in the webview, never here.
fn keyboard_device() -> DeviceInfo {
    DeviceInfo {
        kind: DeviceKind::Keyboard,
        hardware_id: Some(KEYBOARD_HARDWARE_ID.to_string()),
        index: KEYBOARD_INDEX,
        sc_name: Some("Keyboard/Mouse".to_string()),
        sdl_name: "Keyboard/Mouse".to_string(),
        sdl_guid: KEYBOARD_HARDWARE_ID.to_string(),
        power_level: String::new(),
        ..DeviceInfo::default()
    }
}

/// What hidapi knows about a USB `(vendor, product)`: the HID product string
/// (the name SC uses) and, for its joystick-class interface, the axis names.
struct HidInfo {
    name: Option<String>,
    /// Deferred: the descriptor is only read for devices SDL actually lists,
    /// and needs SDL's axis count to cross-check.
    joystick_path: Option<std::ffi::CString>,
    interfaces: Vec<HidInterface>,
}

/// Short-lived hidapi context plus its device table; independent of SDL.
struct HidTable {
    api: Option<hidapi::HidApi>,
    map: HashMap<(u16, u16), HidInfo>,
}

fn hid_table() -> HidTable {
    let mut map: HashMap<(u16, u16), HidInfo> = HashMap::new();
    let api = match hidapi::HidApi::new() {
        Ok(api) => Some(api),
        Err(e) => {
            warn!("hidapi init failed: {e}");
            None
        }
    };
    if let Some(api) = &api {
        for dev in api.device_list() {
            let entry = map
                .entry((dev.vendor_id(), dev.product_id()))
                .or_insert(HidInfo { name: None, joystick_path: None, interfaces: Vec::new() });
            if entry.name.is_none() {
                entry.name = dev.product_string().map(str::to_string);
            }
            // Generic Desktop joystick (4), gamepad (5) or multi-axis (8).
            if entry.joystick_path.is_none() && dev.usage_page() == 0x01 && matches!(dev.usage(), 4 | 5 | 8) {
                entry.joystick_path = Some(dev.path().to_owned());
            }
            entry.interfaces.push(HidInterface {
                path: dev.path().to_string_lossy().into_owned(),
                interface_number: dev.interface_number(),
                usage_page: dev.usage_page(),
                usage: dev.usage(),
                manufacturer: dev.manufacturer_string().map(str::to_string),
                product: dev.product_string().map(str::to_string),
                serial: dev.serial_number().map(str::to_string),
                release: dev.release_number(),
                bus_type: format!("{:?}", dev.bus_type()).to_lowercase(),
            });
        }
    }
    HidTable { api, map }
}

impl HidTable {
    fn get(&self, vid_pid: (u16, u16)) -> Option<&HidInfo> {
        self.map.get(&vid_pid)
    }

    /// The joystick interface's report descriptor, or why it is unavailable.
    fn descriptor(&self, vid_pid: Option<(u16, u16)>) -> Result<Vec<u8>, String> {
        let api = self.api.as_ref().ok_or("hidapi unavailable")?;
        let info = vid_pid.and_then(|k| self.get(k)).ok_or("no HID device for this vendor/product")?;
        let path = info.joystick_path.as_ref().ok_or("no HID joystick interface")?;
        let dev = api.open_path(path).map_err(|e| format!("{}: {e}", path.to_string_lossy()))?;
        crate::hid::read_descriptor(&dev)
    }
}

fn sdl_type_name(t: sdl2::sys::SDL_JoystickType) -> &'static str {
    use sdl2::sys::SDL_JoystickType::*;
    match t {
        SDL_JOYSTICK_TYPE_UNKNOWN => "unknown",
        SDL_JOYSTICK_TYPE_GAMECONTROLLER => "gamecontroller",
        SDL_JOYSTICK_TYPE_WHEEL => "wheel",
        SDL_JOYSTICK_TYPE_ARCADE_STICK => "arcade_stick",
        SDL_JOYSTICK_TYPE_FLIGHT_STICK => "flight_stick",
        SDL_JOYSTICK_TYPE_DANCE_PAD => "dance_pad",
        SDL_JOYSTICK_TYPE_GUITAR => "guitar",
        SDL_JOYSTICK_TYPE_DRUM_KIT => "drum_kit",
        SDL_JOYSTICK_TYPE_ARCADE_PAD => "arcade_pad",
        SDL_JOYSTICK_TYPE_THROTTLE => "throttle",
    }
}

/// How a device is a gamepad, if it is one: SDL's controller name (a real
/// SDL controller) or none (the Wine rule). The `gp1` slot is given
/// afterwards by `open_all`.
struct PadFacts {
    controller_name: Option<String>,
    wine: bool,
}

/// Build the [`DeviceInfo`] for one open joystick; `pad` is set when the
/// device is a gamepad.
fn device_info(stick: &Joystick, index: u32, hid: &HidTable, pad: Option<&PadFacts>) -> DeviceInfo {
    let sdl_guid = stick.guid().string();
    let vid_pid = crate::guid::sdl_guid_vendor_product(&sdl_guid);
    let hid_info = vid_pid.and_then(|k| hid.get(k));
    let sc_name = hid_info.and_then(|i| i.name.clone());
    let num_axes = stick.num_axes();
    let descriptor = hid.descriptor(vid_pid);
    let (axes, axes_error) = match descriptor.as_ref().map_err(Clone::clone).and_then(|d| crate::hid::sc_axes_from(d, num_axes)) {
        Ok(axes) => (axes, None),
        Err(e) => (Vec::new(), Some(e)),
    };
    let hid_usages = descriptor
        .as_ref()
        .map(|d| crate::hid::axis_usages(d).iter().map(|&(p, u)| crate::hid::usage_name(p, u)).collect())
        .unwrap_or_default();
    let hid_descriptor = descriptor.ok().map(|d| d.iter().map(|b| format!("{b:02x}")).collect());
    // Index-based SDL queries the safe wrapper does not expose.
    let i = index as std::os::raw::c_int;
    let (sdl_vendor, sdl_product, sdl_product_version, sdl_type, sdl_path, sdl_serial) = unsafe {
        let path = sdl2::sys::SDL_JoystickPathForIndex(i);
        let cstr = |p: *const std::os::raw::c_char| (!p.is_null()).then(|| std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned());
        // The serial needs the open handle, which the safe wrapper hides.
        let raw = sdl2::sys::SDL_JoystickFromInstanceID(stick.instance_id() as i32);
        let serial = if raw.is_null() { None } else { cstr(sdl2::sys::SDL_JoystickGetSerial(raw)) };
        (
            sdl2::sys::SDL_JoystickGetDeviceVendor(i),
            sdl2::sys::SDL_JoystickGetDeviceProduct(i),
            sdl2::sys::SDL_JoystickGetDeviceProductVersion(i),
            sdl_type_name(sdl2::sys::SDL_JoystickGetDeviceType(i)).to_string(),
            cstr(path),
            serial,
        )
    };
    let sc_product_guid = sdl_guid_to_sc_product(&sdl_guid);
    let kind = if pad.is_some() { DeviceKind::Gamepad } else { DeviceKind::Joystick };
    DeviceInfo {
        kind,
        // Only the pad on SC's single `gp1` slot can carry bindings, so only
        // it gets an image-map key (set by `open_all` with the slot); a
        // further pad gets none.
        hardware_id: match kind {
            DeviceKind::Gamepad => None,
            _ => sc_product_guid.clone(),
        },
        gamepad_slot: None,
        controller_name: pad.and_then(|p| p.controller_name.clone()),
        wine_gamepad: pad.is_some_and(|p| p.wine),
        index,
        sc_name,
        sdl_name: stick.name(),
        sc_product_guid,
        sdl_guid,
        num_buttons: stick.num_buttons(),
        num_axes,
        num_hats: stick.num_hats(),
        axes,
        axes_error,
        sdl_instance_id: stick.instance_id(),
        sdl_vendor,
        sdl_product,
        sdl_product_version,
        sdl_serial,
        sdl_type,
        sdl_path,
        power_level: stick
            .power_level()
            .map(|p| format!("{p:?}").to_lowercase())
            .unwrap_or_else(|e| format!("<{e}>")),
        num_balls: stick.num_balls(),
        has_rumble: stick.has_rumble(),
        has_led: stick.has_led(),
        hid_interfaces: hid_info.map(|i| i.interfaces.clone()).unwrap_or_default(),
        hid_descriptor,
        hid_usages,
    }
}

/// Every device SDL currently lists, with its handles kept open. The joystick
/// handle keeps a device's events flowing; the controller handle is what turns
/// a pad's raw buttons into named `Controller*` events.
struct OpenDevices {
    infos: Vec<DeviceInfo>,
    sticks: Vec<Joystick>,
    pads: Vec<GameController>,
    /// Instance ids of the pads, whose raw `Joy*` events are dropped.
    pad_instances: HashSet<u32>,
    /// Instance ids of the Wine-rule pads (Linux): their raw `Joy*` events
    /// are translated into controller events (`translate_wine_pad`).
    wine_pads: HashSet<u32>,
}

/// Open every connected device and describe it. What is a gamepad and
/// which one holds SC's single `gp1` slot follows the game: on Windows the
/// SDL controllers (XInput devices), the first in SDL's order takes the
/// slot; on Linux winebus's rule (`wineorder::sdl_is_gamepad`) decides,
/// and the slot goes to the first gamepad in Wine's key order (XInput's
/// user 0), which is not SDL's order.
fn open_all(joystick: &JoystickSubsystem, controllers: &GameControllerSubsystem, hid: &HidTable) -> Result<OpenDevices, String> {
    let count = joystick.num_joysticks()?;
    let mut open = OpenDevices {
        infos: Vec::with_capacity(count as usize),
        sticks: Vec::with_capacity(count as usize),
        pads: Vec::new(),
        pad_instances: HashSet::new(),
        wine_pads: HashSet::new(),
    };

    for index in 0..count {
        let stick = match joystick.open(index) {
            Ok(stick) => stick,
            Err(e) => {
                warn!("failed to open joystick {index}: {e}");
                continue;
            }
        };
        let is_controller = controllers.is_game_controller(index);
        // Linux: the game's view. A device winebus takes through hidraw
        // (VKB, VPC, …) is never a gamepad; of the SDL-fed rest, an SDL
        // controller that is a wheel / flight stick is a joystick to the
        // game, and a plain joystick with 6 axes and 14+ buttons is its
        // gamepad, mapped like Wine maps it.
        #[cfg(target_os = "linux")]
        let is_pad = {
            let i = index as std::os::raw::c_int;
            let (vid, pid, t) = unsafe {
                (
                    sdl2::sys::SDL_JoystickGetDeviceVendor(i),
                    sdl2::sys::SDL_JoystickGetDeviceProduct(i),
                    sdl_type_name(sdl2::sys::SDL_JoystickGetDeviceType(i)),
                )
            };
            // winebus reaches a device through its HID joystick / gamepad
            // interface; a node it cannot open read-write does not exist to the
            // game (`wine_devices` applies the same rule to the hidraw path).
            // SDL still lists such a device, but SC does not see it, so it must
            // not become a device here — it shows only as an hid-only row in the
            // Device List. A device with no HID joystick interface is kept: SDL
            // reaches it through evdev, without hidraw.
            if let Some(path) = hid.get((vid, pid)).and_then(|h| h.joystick_path.as_ref()) {
                if std::fs::OpenOptions::new().read(true).write(true).open(&*path.to_string_lossy()).is_err() {
                    continue;
                }
            }
            let buttons = stick.num_buttons();
            !crate::wineorder::hidraw_preferred(vid, pid, buttons)
                && crate::wineorder::sdl_is_gamepad(is_controller, t, stick.num_axes(), buttons)
        };
        #[cfg(not(target_os = "linux"))]
        let is_pad = is_controller;
        // A real controller is opened twice: as a joystick for the raw facts,
        // as a controller for its named events.
        let pad = (is_pad && is_controller).then(|| controllers.open(index)).transpose();
        let pad = match pad {
            Ok(pad) => pad,
            Err(e) => {
                warn!("failed to open game controller {index}: {e}");
                None
            }
        };
        let facts = is_pad.then(|| PadFacts { controller_name: pad.as_ref().map(|c| c.name()), wine: pad.is_none() });

        open.infos.push(device_info(&stick, index, hid, facts.as_ref()));
        if let Some(pad) = pad {
            open.pad_instances.insert(pad.instance_id());
            open.pads.push(pad);
        } else if is_pad {
            open.wine_pads.insert(stick.instance_id());
        }
        open.sticks.push(stick);
    }

    if let Some(i) = gp1_holder(&open.infos) {
        open.infos[i].gamepad_slot = Some(1);
        open.infos[i].hardware_id = Some(GAMEPAD_HARDWARE_ID.to_string());
    }
    Ok(open)
}

/// Position in `infos` of the pad that holds `gp1`: Windows the first pad
/// in SDL's order, Linux the first in Wine's key order.
fn gp1_holder(infos: &[DeviceInfo]) -> Option<usize> {
    #[cfg(target_os = "linux")]
    {
        use crate::wineorder::{first_gamepad, syspath, WineDevice};
        let pads: Vec<usize> = (0..infos.len()).filter(|&i| infos[i].kind == DeviceKind::Gamepad).collect();
        let wine: Vec<WineDevice> = pads
            .iter()
            .map(|&i| {
                let d = &infos[i];
                WineDevice {
                    vid: d.sdl_vendor,
                    pid: d.sdl_product,
                    interface: None,
                    version: d.sdl_product_version,
                    serial: d.sdl_serial.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| "0000".to_string()),
                    is_gamepad: true,
                    product_name: d.sdl_name.clone(),
                    syspath: d.sdl_path.as_deref().map(|p| syspath("input", p)).unwrap_or_default(),
                }
            })
            .collect();
        first_gamepad(&wine).map(|k| pads[k])
    }
    #[cfg(not(target_os = "linux"))]
    {
        infos.iter().position(|d| d.kind == DeviceKind::Gamepad)
    }
}

/// Wine's XInput mapping of a plain joystick's button (bus_sdl builds a
/// generic report, button `n` = usage `n+1`; xinput1_3 takes usages 1..10
/// as A B X Y LB RB Back Start LS RS, the rest is not in the report).
fn wine_pad_button(index: u8) -> Option<Button> {
    Some(match index {
        0 => Button::A,
        1 => Button::B,
        2 => Button::X,
        3 => Button::Y,
        4 => Button::LeftShoulder,
        5 => Button::RightShoulder,
        6 => Button::Back,
        7 => Button::Start,
        8 => Button::LeftStick,
        9 => Button::RightStick,
        _ => return None,
    })
}

/// Wine's XInput mapping of a plain joystick's axis: SDL index order is
/// the report's X Y Z RX RY RZ, xinput1_3 takes X/Y as the left stick,
/// RX/RY as the right, Z/RZ as the triggers. A trigger is the full raw
/// range scaled to 0..32767 (rest = mid-travel, as the game sees it).
fn wine_pad_axis(index: u8, value: i16) -> Option<(Axis, i16)> {
    Some(match index {
        0 => (Axis::LeftX, value),
        1 => (Axis::LeftY, value),
        2 => (Axis::TriggerLeft, ((value as i32 + 32768) / 2) as i16),
        3 => (Axis::RightX, value),
        4 => (Axis::RightY, value),
        5 => (Axis::TriggerRight, ((value as i32 + 32768) / 2) as i16),
        _ => return None,
    })
}

/// The D-pad buttons that change between two hat states (SDL hat bits: 1 up,
/// 2 right, 4 down, 8 left), as (button, pressed).
fn hat_dpad_changes(old: u8, new: u8) -> Vec<(Button, bool)> {
    const BITS: [(u8, Button); 4] =
        [(1, Button::DPadUp), (2, Button::DPadRight), (4, Button::DPadDown), (8, Button::DPadLeft)];
    BITS.iter()
        .filter(|(bit, _)| (old & bit) != (new & bit))
        .map(|&(bit, button)| (button, new & bit != 0))
        .collect()
}

/// A Wine-rule pad's raw joystick event as the controller events the pad
/// path handles (the game sees the device through winexinput, never raw).
/// `hats` keeps the last hat state per instance. Anything Wine's report
/// drops (buttons past 10, a second hat, axes past 6) yields nothing.
fn translate_wine_pad(event: Event, hats: &mut HashMap<u32, u8>) -> Vec<Event> {
    match event {
        Event::JoyButtonDown { timestamp, which, button_idx, .. } => wine_pad_button(button_idx)
            .map(|button| vec![Event::ControllerButtonDown { timestamp, which, button }])
            .unwrap_or_default(),
        Event::JoyButtonUp { timestamp, which, button_idx, .. } => wine_pad_button(button_idx)
            .map(|button| vec![Event::ControllerButtonUp { timestamp, which, button }])
            .unwrap_or_default(),
        Event::JoyAxisMotion { timestamp, which, axis_idx, value, .. } => wine_pad_axis(axis_idx, value)
            .map(|(axis, value)| vec![Event::ControllerAxisMotion { timestamp, which, axis, value }])
            .unwrap_or_default(),
        Event::JoyHatMotion { timestamp, which, hat_idx: 0, state, .. } => {
            let new = state.to_raw();
            let old = hats.insert(which, new).unwrap_or(0);
            hat_dpad_changes(old, new)
                .into_iter()
                .map(|(button, pressed)| {
                    if pressed {
                        Event::ControllerButtonDown { timestamp, which, button }
                    } else {
                        Event::ControllerButtonUp { timestamp, which, button }
                    }
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

/// The joystick instance a raw `Joy*` event belongs to.
fn joy_instance(event: &Event) -> Option<u32> {
    match event {
        Event::JoyButtonDown { which, .. }
        | Event::JoyButtonUp { which, .. }
        | Event::JoyAxisMotion { which, .. }
        | Event::JoyHatMotion { which, .. } => Some(*which),
        _ => None,
    }
}

/// The SDL context, with SDL's own joystick thread switched on. On Windows,
/// RawInput devices (e.g. an Xbox pad over Bluetooth) learn about arrival,
/// removal and their input only through messages to SDL's hidden message
/// window, and without SDL's video subsystem nothing pumps that window: such a
/// pad plugged in after start never showed up. The joystick thread pumps it.
/// No effect on other platforms.
pub fn init_sdl() -> Result<sdl2::Sdl, String> {
    sdl2::hint::set("SDL_JOYSTICK_THREAD", "1");
    sdl2::init()
}

/// Enumerate connected hardware with a short-lived SDL context. For the
/// standalone examples only — the running app reads [`DeviceList`] instead,
/// which also carries the synthetic keyboard.
pub fn enumerate() -> Result<Vec<DeviceInfo>, String> {
    let sdl = init_sdl()?;
    let joystick = sdl.joystick()?;
    let controllers = sdl.game_controller()?;
    let hid = hid_table();
    Ok(open_all(&joystick, &controllers, &hid)?.infos)
}

/// Spawn the input thread. Returns immediately; the thread runs for the life of
/// the app.
pub fn spawn(app: AppHandle, devices: DeviceList) {
    std::thread::spawn(move || {
        if let Err(e) = run(app, devices) {
            error!("input thread stopped: {e}");
        }
    });
}

fn run(app: AppHandle, devices: DeviceList) -> Result<(), String> {
    let sdl = init_sdl()?;
    let joystick = sdl.joystick()?;
    let controllers = sdl.game_controller()?;
    let mut event_pump = sdl.event_pump()?;
    info!("SDL initialized, input thread started");

    // Open handles, kept alive so their events keep being reported.
    let mut opened = OpenDevices {
        infos: Vec::new(),
        sticks: Vec::new(),
        pads: Vec::new(),
        pad_instances: HashSet::new(),
        wine_pads: HashSet::new(),
    };
    // instance_id -> last hat state of a Wine-rule pad (its D-pad)
    let mut hats: HashMap<u32, u8> = HashMap::new();
    // instance_id -> SDL GUID, to tag outgoing events
    let mut guids: HashMap<u32, String> = HashMap::new();
    // (instance_id, axis) -> last forwarded value and its time, for throttling
    let mut last_axis: HashMap<(u32, u8), (i16, u32)> = HashMap::new();
    // (instance_id, axis) -> latest value after the deadzone, forwarded or
    // not, so a stick's other axis is known for the dominance rule
    let mut raw_axis: HashMap<(u32, u8), i16> = HashMap::new();
    // (instance_id, derived button) -> last emitted state, so a stick held
    // past the threshold reports one press, not one per axis event
    let mut derived: HashMap<(u32, &'static str), bool> = HashMap::new();
    // (instance_id, derived button) -> SDL ticks since when the axis has been
    // past the threshold without the button being pressed yet
    let mut waiting: HashMap<(u32, &'static str), u32> = HashMap::new();
    let timer = sdl.timer()?;

    reopen_all(&joystick, &controllers, &mut opened, &mut guids, &app, &devices, true)?;

    loop {
        // Derived buttons whose wait is over press now; a stick held still
        // sends no event, so this runs on the tick as well as on events.
        let now = timer.ticks();
        let due: Vec<(u32, &'static str)> =
            waiting.iter().filter(|(_, since)| derived_due(**since, now)).map(|(k, _)| *k).collect();
        for key in due {
            waiting.remove(&key);
            derived.insert(key, true);
            if let Some(guid) = guids.get(&key.0) {
                let _ = app.emit(
                    "joy-input",
                    InputEvent::PadButton {
                        guid: guid.clone(),
                        name: key.1.to_string(),
                        pressed: true,
                        timestamp: now,
                        instance_id: key.0,
                    },
                );
            }
        }
        let Some(event) = event_pump.wait_event_timeout(LOOP_TICK_MS) else { continue };
        // A Wine-rule pad's raw events become controller events first, so
        // the pad path below (names, deadzones, derived buttons) is the one
        // path for every gamepad.
        let events = match joy_instance(&event) {
            Some(which) if opened.wine_pads.contains(&which) => translate_wine_pad(event, &mut hats),
            _ => vec![event],
        };
        let mut quit = false;
        for event in events {
            // A pad reports every input twice — raw and named. Only the named one
            // carries SC's vocabulary, so the raw copy is dropped (the
            // `!opened.pad_instances.contains(..)` guards below).
            match event {
                Event::JoyDeviceAdded { .. } | Event::JoyDeviceRemoved { .. } => {
                    last_axis.clear();
                    raw_axis.clear();
                    derived.clear();
                    waiting.clear();
                    hats.clear();
                    reopen_all(&joystick, &controllers, &mut opened, &mut guids, &app, &devices, false)?;
                }
                Event::JoyButtonDown { timestamp, which, button_idx, .. } if !opened.pad_instances.contains(&which) => {
                    if let Some(guid) = guids.get(&which) {
                        let _ = app.emit(
                            "joy-input",
                            InputEvent::Button { guid: guid.clone(), index: button_idx, pressed: true, timestamp, instance_id: which },
                        );
                    }
                }
                Event::JoyButtonUp { timestamp, which, button_idx, .. } if !opened.pad_instances.contains(&which) => {
                    if let Some(guid) = guids.get(&which) {
                        let _ = app.emit(
                            "joy-input",
                            InputEvent::Button { guid: guid.clone(), index: button_idx, pressed: false, timestamp, instance_id: which },
                        );
                    }
                }
                Event::JoyHatMotion { timestamp, which, hat_idx, state, .. } if !opened.pad_instances.contains(&which) => {
                    if let Some(guid) = guids.get(&which) {
                        let _ = app.emit(
                            "joy-input",
                            InputEvent::Hat {
                                guid: guid.clone(),
                                index: hat_idx,
                                direction: hat_direction(state),
                                raw: state.to_raw(),
                                timestamp,
                                instance_id: which,
                            },
                        );
                    }
                }
                Event::JoyAxisMotion { timestamp, which, axis_idx, value, .. } if !opened.pad_instances.contains(&which) => {
                    let value = apply_deadzone(value, JOYSTICK_DEADZONE);
                    raw_axis.insert((which, axis_idx), value);
                    let partner = opened
                        .infos
                        .iter()
                        .find(|d| d.sdl_instance_id == which)
                        .and_then(|d| joystick_stick_partner(&d.axes, axis_idx))
                        .and_then(|p| raw_axis.get(&(which, p)).copied());
                    if !leads_stick(value, partner) {
                        continue;
                    }
                    let last = last_axis.get(&(which, axis_idx)).copied();
                    if axis_due(value, last.map_or(0, |l| l.0), timestamp, last.map(|l| l.1)) {
                        last_axis.insert((which, axis_idx), (value, timestamp));
                        if let Some(guid) = guids.get(&which) {
                            let _ = app.emit(
                                "joy-input",
                                InputEvent::Axis { guid: guid.clone(), index: axis_idx, value, timestamp, instance_id: which },
                            );
                        }
                    }
                }
                Event::ControllerButtonDown { timestamp, which, button } => {
                    if let Some(guid) = guids.get(&which) {
                        let _ = app.emit(
                            "joy-input",
                            InputEvent::PadButton {
                                guid: guid.clone(),
                                name: pad_button_name(button).to_string(),
                                pressed: true,
                                timestamp,
                                instance_id: which,
                            },
                        );
                    }
                }
                Event::ControllerButtonUp { timestamp, which, button } => {
                    if let Some(guid) = guids.get(&which) {
                        let _ = app.emit(
                            "joy-input",
                            InputEvent::PadButton {
                                guid: guid.clone(),
                                name: pad_button_name(button).to_string(),
                                pressed: false,
                                timestamp,
                                instance_id: which,
                            },
                        );
                    }
                }
                Event::ControllerAxisMotion { timestamp, which, axis, value } => {
                    let Some(guid) = guids.get(&which).cloned() else { continue };
                    let value = apply_deadzone(value, pad_axis_deadzone(axis));
                    // SC's trigger/thumb-direction "buttons" have no axis, so they
                    // are derived here and reported only when they change.
                    for (name, travel) in derived_pad_buttons(axis, value) {
                        let key = (which, name);
                        let pressed = derived.get(&key).copied().unwrap_or(false);
                        match derived_step(travel, pressed) {
                            DerivedStep::Release => {
                                derived.insert(key, false);
                                let _ = app.emit(
                                    "joy-input",
                                    InputEvent::PadButton {
                                        guid: guid.clone(),
                                        name: name.to_string(),
                                        pressed: false,
                                        timestamp,
                                        instance_id: which,
                                    },
                                );
                            }
                            DerivedStep::Wait if presses_at_once(name) => {
                                derived.insert(key, true);
                                let _ = app.emit(
                                    "joy-input",
                                    InputEvent::PadButton {
                                        guid: guid.clone(),
                                        name: name.to_string(),
                                        pressed: true,
                                        timestamp,
                                        instance_id: which,
                                    },
                                );
                            }
                            DerivedStep::Wait => {
                                waiting.entry(key).or_insert(timestamp);
                            }
                            DerivedStep::Idle => {
                                waiting.remove(&key);
                            }
                            DerivedStep::Hold => {}
                        }
                    }
                    // A trigger is only ever its derived button: SC labels the
                    // axis token (`gp1_triggerl`) like the button and ships no
                    // default on it, so the app does not know the axis at all.
                    // Both buttons together are a third one, `triggerl_r_btn`.
                    if matches!(axis, Axis::TriggerLeft | Axis::TriggerRight) {
                        let both = both_triggers(&derived, which);
                        // The first trigger event only sets the state; unknown
                        // counts as released, not as a change.
                        if derived.insert((which, "triggerl_r_btn"), both).unwrap_or(false) != both {
                            let _ = app.emit(
                                "joy-input",
                                InputEvent::PadButton {
                                    guid: guid.clone(),
                                    name: "triggerl_r_btn".to_string(),
                                    pressed: both,
                                    timestamp,
                                    instance_id: which,
                                },
                            );
                        }
                        continue;
                    }
                    // The axis itself is throttled like a joystick axis. SDL
                    // controller axes are numbered 0..5, so they share the map
                    // without colliding with the raw axes (which are dropped).
                    let idx = axis as u8;
                    raw_axis.insert((which, idx), value);
                    let partner = pad_stick_partner(axis).and_then(|p| raw_axis.get(&(which, p as u8)).copied());
                    if !leads_stick(value, partner) {
                        continue;
                    }
                    let last = last_axis.get(&(which, idx)).copied();
                    if axis_due(value, last.map_or(0, |l| l.0), timestamp, last.map(|l| l.1)) {
                        last_axis.insert((which, idx), (value, timestamp));
                        let _ = app.emit(
                            "joy-input",
                            InputEvent::PadAxis {
                                guid,
                                name: pad_axis_name(axis).to_string(),
                                value,
                                timestamp,
                                instance_id: which,
                            },
                        );
                    }
                }
                Event::Quit { .. } => quit = true,
                _ => {}
            }
        }
        if quit {
            break;
        }
    }

    Ok(())
}

/// Payload of `devices-changed`: what a re-enumeration added and removed
/// (by SDL instance id, which is unique per connection). Both are empty at
/// startup and for SDL's initial arrival events, so the frontend announces
/// only real hot-plugs.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DevicesChanged {
    pub added: Vec<DeviceInfo>,
    pub removed: Vec<DeviceInfo>,
}

/// The devices in `new` that `old` lacks, by instance id.
fn devices_added(old: &[DeviceInfo], new: &[DeviceInfo]) -> Vec<DeviceInfo> {
    new.iter().filter(|n| !old.iter().any(|o| o.sdl_instance_id == n.sdl_instance_id)).cloned().collect()
}

/// Re-enumerate everything: reopen every device, rebuild the GUID map and the
/// shared device list (with the synthetic keyboard last), and tell the
/// frontend the list changed. `initial` is the startup enumeration: nothing
/// was plugged in or out, so the payload carries no change.
fn reopen_all(
    joystick: &JoystickSubsystem,
    controllers: &GameControllerSubsystem,
    opened: &mut OpenDevices,
    guids: &mut HashMap<u32, String>,
    app: &AppHandle,
    devices: &DeviceList,
    initial: bool,
) -> Result<(), String> {
    guids.clear();

    let hid = hid_table();
    let before = std::mem::take(&mut opened.infos);
    *opened = open_all(joystick, controllers, &hid)?;
    let change = if initial {
        DevicesChanged::default()
    } else {
        DevicesChanged { added: devices_added(&before, &opened.infos), removed: devices_added(&opened.infos, &before) }
    };
    for stick in &opened.sticks {
        if let Some(info) = opened.infos.iter().find(|i| i.sdl_instance_id == stick.instance_id()) {
            guids.insert(stick.instance_id(), info.sdl_guid.clone());
        }
    }

    let mut list = opened.infos.clone();
    list.push(keyboard_device());

    // Device details stay out of the app log by design (the Devices mode's
    // Device Info view has them all); only failures are logged above.

    if let Ok(mut shared) = devices.lock() {
        *shared = list;
    }
    let _ = app.emit("devices-changed", &change);
    Ok(())
}

fn hat_direction(state: HatState) -> String {
    match state {
        HatState::Centered => "centered",
        HatState::Up => "up",
        HatState::Right => "right",
        HatState::Down => "down",
        HatState::Left => "left",
        HatState::RightUp => "rightup",
        HatState::RightDown => "rightdown",
        HatState::LeftUp => "leftup",
        HatState::LeftDown => "leftdown",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wine_pad_buttons_follow_xinputs_usage_order() {
        let names: Vec<&str> = (0..10).map(|i| pad_button_name(wine_pad_button(i).unwrap())).collect();
        assert_eq!(names, ["a", "b", "x", "y", "shoulderl", "shoulderr", "back", "start", "thumbl", "thumbr"]);
        // Usages past 10 are not in winexinput's report: the K2 HE's buttons 10-15 vanish.
        assert_eq!(wine_pad_button(10), None);
        assert_eq!(wine_pad_button(15), None);
    }

    #[test]
    fn wine_pad_axes_are_sticks_and_scaled_triggers() {
        assert_eq!(wine_pad_axis(0, -1234), Some((Axis::LeftX, -1234)));
        assert_eq!(wine_pad_axis(1, 32767), Some((Axis::LeftY, 32767)));
        assert_eq!(wine_pad_axis(3, 5), Some((Axis::RightX, 5)));
        assert_eq!(wine_pad_axis(4, -5), Some((Axis::RightY, -5)));
        // Z / RZ are the triggers, the raw range mapped onto 0..32767: rest is mid-travel.
        assert_eq!(wine_pad_axis(2, -32768), Some((Axis::TriggerLeft, 0)));
        assert_eq!(wine_pad_axis(2, 0), Some((Axis::TriggerLeft, 16384)));
        assert_eq!(wine_pad_axis(5, 32767), Some((Axis::TriggerRight, 32767)));
        assert_eq!(wine_pad_axis(6, 0), None);
    }

    #[test]
    fn hat_becomes_dpad_presses_and_releases() {
        // centred -> up
        assert_eq!(hat_dpad_changes(0, 1), vec![(Button::DPadUp, true)]);
        // up -> right-up: right pressed, up stays
        assert_eq!(hat_dpad_changes(1, 3), vec![(Button::DPadRight, true)]);
        // right-up -> centred: both released
        assert_eq!(hat_dpad_changes(3, 0), vec![(Button::DPadUp, false), (Button::DPadRight, false)]);
        assert_eq!(hat_dpad_changes(4, 4), vec![]);
    }

    #[test]
    fn translate_wine_pad_yields_controller_events_only() {
        let mut hats = HashMap::new();
        let ev = translate_wine_pad(Event::JoyButtonDown { timestamp: 7, which: 3, button_idx: 4 }, &mut hats);
        assert!(matches!(ev.as_slice(), [Event::ControllerButtonDown { timestamp: 7, which: 3, button: Button::LeftShoulder }]));
        let ev = translate_wine_pad(Event::JoyButtonUp { timestamp: 8, which: 3, button_idx: 12 }, &mut hats);
        assert!(ev.is_empty());
        let ev = translate_wine_pad(Event::JoyAxisMotion { timestamp: 9, which: 3, axis_idx: 5, value: 32767 }, &mut hats);
        assert!(matches!(ev.as_slice(), [Event::ControllerAxisMotion { which: 3, axis: Axis::TriggerRight, value: 32767, .. }]));
        // The hat is the D-pad; a second hat is not in the report.
        let ev = translate_wine_pad(Event::JoyHatMotion { timestamp: 10, which: 3, hat_idx: 0, state: HatState::LeftUp }, &mut hats);
        assert_eq!(ev.len(), 2);
        assert!(ev.iter().all(|e| matches!(e, Event::ControllerButtonDown { .. })));
        let ev = translate_wine_pad(Event::JoyHatMotion { timestamp: 11, which: 3, hat_idx: 1, state: HatState::Up }, &mut hats);
        assert!(ev.is_empty());
        let ev = translate_wine_pad(Event::JoyHatMotion { timestamp: 12, which: 3, hat_idx: 0, state: HatState::Centered }, &mut hats);
        assert!(ev.iter().all(|e| matches!(e, Event::ControllerButtonUp { .. })));
        assert_eq!(ev.len(), 2);
    }

    #[test]
    fn deadzone_flattens_rest_and_keeps_travel() {
        assert_eq!(apply_deadzone(0, JOYSTICK_DEADZONE), 0);
        assert_eq!(apply_deadzone(3999, JOYSTICK_DEADZONE), 0);
        assert_eq!(apply_deadzone(-3999, JOYSTICK_DEADZONE), 0);
        assert_eq!(apply_deadzone(4000, JOYSTICK_DEADZONE), 4000);
        assert_eq!(apply_deadzone(-32768, JOYSTICK_DEADZONE), -32768);
        assert_eq!(apply_deadzone(32767, JOYSTICK_DEADZONE), 32767);

        // Pads: XInput's zones, rounded.
        assert_eq!(pad_axis_deadzone(Axis::LeftY), 8000);
        assert_eq!(pad_axis_deadzone(Axis::RightX), 8000);
        assert_eq!(pad_axis_deadzone(Axis::TriggerRight), 4000);
        assert_eq!(apply_deadzone(7999, pad_axis_deadzone(Axis::LeftX)), 0);
        assert_eq!(apply_deadzone(8000, pad_axis_deadzone(Axis::LeftX)), 8000);
        assert_eq!(apply_deadzone(3000, pad_axis_deadzone(Axis::TriggerLeft)), 0);
    }

    #[test]
    fn axis_events_are_rate_limited_but_the_centre_always_passes() {
        // Jitter below the threshold never passes, whatever the timing.
        assert!(!axis_due(2000, 0, 1000, None));
        // The first real move passes, the next one only after the interval.
        assert!(axis_due(10000, 0, 1000, None));
        assert!(!axis_due(20000, 10000, 1010, Some(1000)));
        assert!(axis_due(20000, 10000, 1020, Some(1000)));
        // Back to the centre passes at once.
        assert!(axis_due(0, 20000, 1021, Some(1020)));
    }

    #[test]
    fn the_further_deflected_axis_leads_its_stick() {
        assert!(leads_stick(10000, None)); // no partner: always
        assert!(leads_stick(10000, Some(0)));
        assert!(leads_stick(10000, Some(10000))); // a tie goes to the mover
        assert!(!leads_stick(10000, Some(-20000)));
        assert!(leads_stick(0, Some(-20000))); // the centre always passes

        assert_eq!(pad_stick_partner(Axis::LeftX), Some(Axis::LeftY));
        assert_eq!(pad_stick_partner(Axis::RightY), Some(Axis::RightX));
        assert_eq!(pad_stick_partner(Axis::TriggerLeft), None);

        let axes: Vec<String> = ["x", "y", "z", "rotx", "roty", "rotz"].map(String::from).into();
        assert_eq!(joystick_stick_partner(&axes, 0), Some(1));
        assert_eq!(joystick_stick_partner(&axes, 1), Some(0));
        assert_eq!(joystick_stick_partner(&axes, 5), None); // twist stands alone
        assert_eq!(joystick_stick_partner(&axes, 9), None); // no such axis
    }

    #[test]
    fn both_triggers_make_the_combo_button() {
        let mut derived = HashMap::new();
        assert!(!both_triggers(&derived, 7));
        derived.insert((7, "triggerl_btn"), true);
        assert!(!both_triggers(&derived, 7));
        derived.insert((7, "triggerr_btn"), true);
        assert!(both_triggers(&derived, 7));
        assert!(!both_triggers(&derived, 8)); // another pad
        derived.insert((7, "triggerl_btn"), false);
        assert!(!both_triggers(&derived, 7));
    }

    #[test]
    fn derives_trigger_and_thumb_buttons_from_axes() {
        // Triggers rest at 0 and press towards their travel.
        assert_eq!(derived_pad_buttons(Axis::TriggerLeft, 0), vec![("triggerl_btn", 0)]);
        assert_eq!(derived_pad_buttons(Axis::TriggerRight, 30000), vec![("triggerr_btn", 30000)]);

        // A thumb stick reports both directions of its axis: travel towards
        // one is travel away from the other.
        assert_eq!(derived_pad_buttons(Axis::LeftX, -30000), vec![("thumbl_left", 30000), ("thumbl_right", -30000)]);
        // SDL's Y axis is negative upwards.
        assert_eq!(derived_pad_buttons(Axis::LeftY, -30000), vec![("thumbl_up", 30000), ("thumbl_down", -30000)]);
        assert_eq!(derived_pad_buttons(Axis::RightY, 30000), vec![("thumbr_up", -30000), ("thumbr_down", 30000)]);
    }

    #[test]
    fn derived_buttons_wait_past_the_threshold_and_let_go_below_the_release_point() {
        // Unpressed: past the threshold the button waits, below it idles.
        assert_eq!(derived_step(29999, false), DerivedStep::Idle);
        assert_eq!(derived_step(30000, false), DerivedStep::Wait);
        // Travel away from the button never presses it.
        assert_eq!(derived_step(-32768, false), DerivedStep::Idle);
        // Pressed: a wobble around the threshold keeps it, below the release
        // point it lets go.
        assert_eq!(derived_step(29000, true), DerivedStep::Hold);
        assert_eq!(derived_step(24001, true), DerivedStep::Hold);
        assert_eq!(derived_step(24000, true), DerivedStep::Release);
        assert_eq!(derived_step(0, true), DerivedStep::Release);
        // The wait is over after the hold time, wrapping ticks included.
        assert!(!derived_due(1000, 1000 + STICK_HOLD_MS - 1));
        assert!(derived_due(1000, 1000 + STICK_HOLD_MS));
        assert!(derived_due(u32::MAX - 10, STICK_HOLD_MS - 10));
        // A trigger is a button like a shoulder button: no wait at all.
        assert!(presses_at_once("triggerl_btn"));
        assert!(presses_at_once("triggerr_btn"));
        assert!(!presses_at_once("thumbl_left"));
        assert!(!presses_at_once("thumbr_down"));
    }

    #[test]
    fn pad_names_are_scs_own() {
        assert_eq!(pad_button_name(Button::LeftShoulder), "shoulderl");
        assert_eq!(pad_button_name(Button::DPadUp), "dpad_up");
        assert_eq!(pad_button_name(Button::LeftStick), "thumbl");
        assert_eq!(pad_axis_name(Axis::LeftX), "thumblx");
        assert_eq!(pad_axis_name(Axis::TriggerRight), "triggerr");
    }

    #[test]
    fn synthetic_keyboard_is_a_fixed_entry() {
        let kb = keyboard_device();
        assert_eq!(kb.kind, DeviceKind::Keyboard);
        assert_eq!(kb.hardware_id.as_deref(), Some("keyboard"));
        assert_eq!(kb.sdl_guid, "keyboard");
        assert_eq!(kb.sc_name.as_deref(), Some("Keyboard/Mouse"));
        assert_eq!(kb.sc_product_guid, None); // has no GUID
        assert_eq!(kb.gamepad_slot, None);
        assert_eq!((kb.num_buttons, kb.num_axes, kb.num_hats), (0, 0, 0));
        assert!(kb.axes.is_empty() && kb.hid_interfaces.is_empty());
    }

    #[test]
    fn event_payloads_carry_the_contract_tags() {
        let json = |e: &InputEvent| serde_json::to_string(e).unwrap();
        let button = InputEvent::PadButton {
            guid: "g".into(),
            name: "thumbl_left".into(),
            pressed: true,
            timestamp: 7,
            instance_id: 3,
        };
        assert_eq!(
            json(&button),
            r#"{"kind":"padbutton","guid":"g","name":"thumbl_left","pressed":true,"timestamp":7,"instance_id":3}"#
        );
        let axis = InputEvent::PadAxis {
            guid: "g".into(),
            name: "thumblx".into(),
            value: -900,
            timestamp: 7,
            instance_id: 3,
        };
        assert_eq!(
            json(&axis),
            r#"{"kind":"padaxis","guid":"g","name":"thumblx","value":-900,"timestamp":7,"instance_id":3}"#
        );
    }
}
