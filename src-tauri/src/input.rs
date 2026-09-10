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

/// Half of SDL's axis range: the point at which a gamepad trigger or thumb
/// stick direction counts as a pressed button (SC's `triggerl_btn`,
/// `thumbl_left`, …, which have no axis of their own).
const DERIVED_BUTTON_THRESHOLD: i16 = 16384;

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
/// is troubleshooting detail for the device log.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DeviceInfo {
    /// Joystick, gamepad, or the synthetic keyboard.
    pub kind: DeviceKind,
    /// The key for image-maps and `imagemap_choices`: a joystick's SC Product
    /// GUID, the fixed `"gamepad"` for the pad holding the `gp1` slot, the
    /// fixed `"keyboard"`, and `None` for anything that cannot be bound
    /// (a further pad, a joystick with an unparseable GUID).
    pub hardware_id: Option<String>,
    /// `Some(1)` for the first game controller in SDL index order — SC's one
    /// `gp1` slot, first come first serve. `None` for every further pad and
    /// for non-pads.
    pub gamepad_slot: Option<u32>,
    /// `SDL_GameControllerName` for pads, `None` otherwise. Usually friendlier
    /// than the raw joystick name.
    pub controller_name: Option<String>,
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
/// `Touchpad` have no SC token — they are still reported so the device log and
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

/// The buttons SC derives from a pad axis, as `(name, pressed)` for the
/// current axis value: triggers press past half travel, thumb sticks press in
/// each direction. SDL's Y axis is negative upwards.
fn derived_pad_buttons(axis: Axis, value: i16) -> Vec<(&'static str, bool)> {
    let (low, high) = match axis {
        Axis::LeftX => ("thumbl_left", "thumbl_right"),
        Axis::LeftY => ("thumbl_up", "thumbl_down"),
        Axis::RightX => ("thumbr_left", "thumbr_right"),
        Axis::RightY => ("thumbr_up", "thumbr_down"),
        Axis::TriggerLeft => return vec![("triggerl_btn", value >= DERIVED_BUTTON_THRESHOLD)],
        Axis::TriggerRight => return vec![("triggerr_btn", value >= DERIVED_BUTTON_THRESHOLD)],
    };
    vec![
        (low, value <= -DERIVED_BUTTON_THRESHOLD),
        (high, value >= DERIVED_BUTTON_THRESHOLD),
    ]
}

/// The synthetic keyboard entry, appended last to every device list. SC knows
/// exactly one keyboard and never logs it, so it is always "there"; its key
/// events are captured in the webview, never here.
fn keyboard_device() -> DeviceInfo {
    DeviceInfo {
        kind: DeviceKind::Keyboard,
        hardware_id: Some(KEYBOARD_HARDWARE_ID.to_string()),
        index: KEYBOARD_INDEX,
        sc_name: Some("Keyboard".to_string()),
        sdl_name: "Keyboard".to_string(),
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

/// Build the [`DeviceInfo`] for one open joystick. `pad` carries the game
/// controller facts when SDL recognises the device as one, and whether it took
/// SC's single `gp1` slot.
fn device_info(stick: &Joystick, index: u32, hid: &HidTable, pad: Option<(&GameController, bool)>) -> DeviceInfo {
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
    let (sdl_vendor, sdl_product, sdl_product_version, sdl_type, sdl_path) = unsafe {
        let path = sdl2::sys::SDL_JoystickPathForIndex(i);
        (
            sdl2::sys::SDL_JoystickGetDeviceVendor(i),
            sdl2::sys::SDL_JoystickGetDeviceProduct(i),
            sdl2::sys::SDL_JoystickGetDeviceProductVersion(i),
            sdl_type_name(sdl2::sys::SDL_JoystickGetDeviceType(i)).to_string(),
            (!path.is_null()).then(|| std::ffi::CStr::from_ptr(path).to_string_lossy().into_owned()),
        )
    };
    let sc_product_guid = sdl_guid_to_sc_product(&sdl_guid);
    let kind = if pad.is_some() { DeviceKind::Gamepad } else { DeviceKind::Joystick };
    let has_slot = matches!(pad, Some((_, true)));
    DeviceInfo {
        kind,
        // Only the pad on SC's single `gp1` slot can carry bindings, so only
        // it gets an image-map key; a further pad gets none.
        hardware_id: match kind {
            DeviceKind::Gamepad => has_slot.then(|| GAMEPAD_HARDWARE_ID.to_string()),
            _ => sc_product_guid.clone(),
        },
        gamepad_slot: has_slot.then_some(1),
        controller_name: pad.map(|(c, _)| c.name()),
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
}

/// Open every connected device and describe it. The first device SDL
/// recognises as a game controller takes SC's single `gp1` slot.
fn open_all(joystick: &JoystickSubsystem, controllers: &GameControllerSubsystem, hid: &HidTable) -> Result<OpenDevices, String> {
    let count = joystick.num_joysticks()?;
    let mut open = OpenDevices {
        infos: Vec::with_capacity(count as usize),
        sticks: Vec::with_capacity(count as usize),
        pads: Vec::new(),
        pad_instances: HashSet::new(),
    };
    let mut slot_taken = false;

    for index in 0..count {
        let stick = match joystick.open(index) {
            Ok(stick) => stick,
            Err(e) => {
                warn!("failed to open joystick {index}: {e}");
                continue;
            }
        };
        // A pad is opened twice: as a joystick for the raw facts below, as a
        // controller for its named events.
        let pad = controllers.is_game_controller(index).then(|| controllers.open(index)).transpose();
        let pad = match pad {
            Ok(pad) => pad,
            Err(e) => {
                warn!("failed to open game controller {index}: {e}");
                None
            }
        };
        let has_slot = pad.is_some() && !slot_taken;
        slot_taken |= pad.is_some();

        open.infos.push(device_info(&stick, index, hid, pad.as_ref().map(|c| (c, has_slot))));
        if let Some(pad) = pad {
            open.pad_instances.insert(pad.instance_id());
            open.pads.push(pad);
        }
        open.sticks.push(stick);
    }

    Ok(open)
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
    };
    // instance_id -> SDL GUID, to tag outgoing events
    let mut guids: HashMap<u32, String> = HashMap::new();
    // (instance_id, axis) -> last forwarded value, for jitter throttling
    let mut last_axis: HashMap<(u32, u8), i16> = HashMap::new();
    // (instance_id, derived button) -> last emitted state, so a stick held
    // past the threshold reports one press, not one per axis event
    let mut derived: HashMap<(u32, &'static str), bool> = HashMap::new();

    reopen_all(&joystick, &controllers, &mut opened, &mut guids, &app, &devices)?;

    for event in event_pump.wait_iter() {
        // A pad reports every input twice — raw and named. Only the named one
        // carries SC's vocabulary, so the raw copy is dropped (the
        // `!opened.pad_instances.contains(..)` guards below).
        match event {
            Event::JoyDeviceAdded { .. } | Event::JoyDeviceRemoved { .. } => {
                last_axis.clear();
                derived.clear();
                reopen_all(&joystick, &controllers, &mut opened, &mut guids, &app, &devices)?;
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
                let prev = last_axis.get(&(which, axis_idx)).copied().unwrap_or(0);
                if (value as i32 - prev as i32).abs() > AXIS_EMIT_THRESHOLD {
                    last_axis.insert((which, axis_idx), value);
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
                // SC's trigger/thumb-direction "buttons" have no axis, so they
                // are derived here and reported only when they change.
                for (name, pressed) in derived_pad_buttons(axis, value) {
                    if derived.insert((which, name), pressed) != Some(pressed) {
                        let _ = app.emit(
                            "joy-input",
                            InputEvent::PadButton {
                                guid: guid.clone(),
                                name: name.to_string(),
                                pressed,
                                timestamp,
                                instance_id: which,
                            },
                        );
                    }
                }
                // The axis itself is throttled like a joystick axis. SDL
                // controller axes are numbered 0..5, so they share the map
                // without colliding with the raw axes (which are dropped).
                let idx = axis as u8;
                let prev = last_axis.get(&(which, idx)).copied().unwrap_or(0);
                if (value as i32 - prev as i32).abs() > AXIS_EMIT_THRESHOLD {
                    last_axis.insert((which, idx), value);
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
            Event::Quit { .. } => break,
            _ => {}
        }
    }

    Ok(())
}

/// Re-enumerate everything: reopen every device, rebuild the GUID map and the
/// shared device list (with the synthetic keyboard last), and tell the
/// frontend the list changed.
fn reopen_all(
    joystick: &JoystickSubsystem,
    controllers: &GameControllerSubsystem,
    opened: &mut OpenDevices,
    guids: &mut HashMap<u32, String>,
    app: &AppHandle,
    devices: &DeviceList,
) -> Result<(), String> {
    guids.clear();

    let hid = hid_table();
    *opened = open_all(joystick, controllers, &hid)?;
    for stick in &opened.sticks {
        if let Some(info) = opened.infos.iter().find(|i| i.sdl_instance_id == stick.instance_id()) {
            guids.insert(stick.instance_id(), info.sdl_guid.clone());
        }
    }

    let mut list = opened.infos.clone();
    list.push(keyboard_device());

    // Device details stay out of the app log by design (the Devices mode's
    // device log has them all); only failures are logged above.

    if let Ok(mut shared) = devices.lock() {
        *shared = list;
    }
    let _ = app.emit("devices-changed", ());
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
    fn derives_trigger_and_thumb_buttons_from_axes() {
        // Triggers rest at 0 and press past half travel.
        assert_eq!(derived_pad_buttons(Axis::TriggerLeft, 0), vec![("triggerl_btn", false)]);
        assert_eq!(derived_pad_buttons(Axis::TriggerLeft, 16383), vec![("triggerl_btn", false)]);
        assert_eq!(derived_pad_buttons(Axis::TriggerLeft, 16384), vec![("triggerl_btn", true)]);
        assert_eq!(derived_pad_buttons(Axis::TriggerRight, 32767), vec![("triggerr_btn", true)]);

        // A thumb stick reports both directions of its axis, never both at once.
        assert_eq!(
            derived_pad_buttons(Axis::LeftX, 0),
            vec![("thumbl_left", false), ("thumbl_right", false)]
        );
        assert_eq!(
            derived_pad_buttons(Axis::LeftX, -20000),
            vec![("thumbl_left", true), ("thumbl_right", false)]
        );
        // SDL's Y axis is negative upwards.
        assert_eq!(
            derived_pad_buttons(Axis::LeftY, -20000),
            vec![("thumbl_up", true), ("thumbl_down", false)]
        );
        assert_eq!(
            derived_pad_buttons(Axis::RightY, 20000),
            vec![("thumbr_up", false), ("thumbr_down", true)]
        );
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
        assert_eq!(kb.sc_name.as_deref(), Some("Keyboard"));
        assert_eq!(kb.sc_product_guid, None); // cannot be excluded, has no GUID
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
