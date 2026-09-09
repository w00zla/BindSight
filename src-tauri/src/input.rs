//! Joystick input via the SDL raw joystick API (not the GameController
//! abstraction).
//!
//! A single background thread owns the one SDL context (rust-sdl2 allows only
//! one at a time), keeps every connected joystick open so their events are
//! reported, pumps the event loop and forwards each button/axis/hat change to
//! the frontend as a `joy-input` Tauri event. It also maintains the shared
//! device list and emits `devices-changed` on hot-plug; the `list_joysticks`
//! command only reads that list, so a second SDL context never exists.
//!
//! The standalone [`enumerate`] path (used by the examples) makes its own
//! short-lived context and must not run while the app's input thread is alive.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use log::{debug, error, info, warn};
use sdl2::event::Event;
use sdl2::joystick::{HatState, Joystick};
use sdl2::JoystickSubsystem;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::guid::sdl_guid_to_sc_product;

/// Only forward an axis once it has moved more than this since the last
/// forwarded value, so continuous jitter does not flood the frontend.
const AXIS_EMIT_THRESHOLD: i32 = 3000;

/// A connected joystick as BindSight sees it.
#[derive(Debug, Clone, Serialize)]
pub struct DeviceInfo {
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
}

/// Shared, hot-pluggable device list, maintained by the input thread and read
/// by the `list_joysticks` command.
pub type DeviceList = Arc<Mutex<Vec<DeviceInfo>>>;

/// A single live input change, forwarded to the frontend as a `joy-input`
/// event. `guid` is the device's SDL GUID, the join key to [`DeviceInfo`].
#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum InputEvent {
    Button { guid: String, index: u8, pressed: bool },
    Axis { guid: String, index: u8, value: i16 },
    Hat { guid: String, index: u8, direction: String },
}

/// What hidapi knows about a USB `(vendor, product)`: the HID product string
/// (the name SC uses) and, for its joystick-class interface, the axis names.
struct HidInfo {
    name: Option<String>,
    /// Deferred: the descriptor is only read for devices SDL actually lists,
    /// and needs SDL's axis count to cross-check.
    joystick_path: Option<std::ffi::CString>,
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
                .or_insert(HidInfo { name: None, joystick_path: None });
            if entry.name.is_none() {
                entry.name = dev.product_string().map(str::to_string);
            }
            // Generic Desktop joystick (4), gamepad (5) or multi-axis (8).
            if entry.joystick_path.is_none() && dev.usage_page() == 0x01 && matches!(dev.usage(), 4 | 5 | 8) {
                entry.joystick_path = Some(dev.path().to_owned());
            }
        }
    }
    HidTable { api, map }
}

impl HidTable {
    fn get(&self, vid_pid: (u16, u16)) -> Option<&HidInfo> {
        self.map.get(&vid_pid)
    }

    /// SC axis names by SDL index for a device, or why they are unavailable.
    fn axes(&self, vid_pid: Option<(u16, u16)>, sdl_axes: u32) -> Result<Vec<String>, String> {
        let api = self.api.as_ref().ok_or("hidapi unavailable")?;
        let info = vid_pid.and_then(|k| self.get(k)).ok_or("no HID device for this vendor/product")?;
        let path = info.joystick_path.as_ref().ok_or("no HID joystick interface")?;
        let dev = api.open_path(path).map_err(|e| format!("{}: {e}", path.to_string_lossy()))?;
        crate::hid::sc_axes_for(&dev, sdl_axes)
    }
}

fn device_info(stick: &Joystick, index: u32, hid: &HidTable) -> DeviceInfo {
    let sdl_guid = stick.guid().string();
    let vid_pid = crate::guid::sdl_guid_vendor_product(&sdl_guid);
    let sc_name = vid_pid.and_then(|k| hid.get(k)).and_then(|i| i.name.clone());
    let num_axes = stick.num_axes();
    let (axes, axes_error) = match hid.axes(vid_pid, num_axes) {
        Ok(axes) => (axes, None),
        Err(e) => (Vec::new(), Some(e)),
    };
    DeviceInfo {
        index,
        sc_name,
        sdl_name: stick.name(),
        sc_product_guid: sdl_guid_to_sc_product(&sdl_guid),
        sdl_guid,
        num_buttons: stick.num_buttons(),
        num_axes,
        num_hats: stick.num_hats(),
        axes,
        axes_error,
    }
}

/// Enumerate connected joysticks with a short-lived SDL context. For the
/// standalone examples only — the running app reads [`DeviceList`] instead.
pub fn enumerate() -> Result<Vec<DeviceInfo>, String> {
    let sdl = sdl2::init()?;
    let joystick = sdl.joystick()?;
    let hid = hid_table();
    let count = joystick.num_joysticks()?;
    let mut devices = Vec::with_capacity(count as usize);
    for index in 0..count {
        if let Ok(stick) = joystick.open(index) {
            devices.push(device_info(&stick, index, &hid));
        }
    }
    Ok(devices)
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
    let sdl = sdl2::init()?;
    let joystick = sdl.joystick()?;
    let mut event_pump = sdl.event_pump()?;
    info!("SDL initialized, input thread started");

    // instance_id -> open handle (kept alive so its events keep being reported)
    let mut opened: HashMap<u32, Joystick> = HashMap::new();
    // instance_id -> SDL GUID, to tag outgoing events
    let mut guids: HashMap<u32, String> = HashMap::new();
    // (instance_id, axis) -> last forwarded value, for jitter throttling
    let mut last_axis: HashMap<(u32, u8), i16> = HashMap::new();

    reopen_all(&joystick, &mut opened, &mut guids, &app, &devices)?;

    for event in event_pump.wait_iter() {
        match event {
            Event::JoyDeviceAdded { .. } | Event::JoyDeviceRemoved { .. } => {
                last_axis.clear();
                reopen_all(&joystick, &mut opened, &mut guids, &app, &devices)?;
            }
            Event::JoyButtonDown { which, button_idx, .. } => {
                if let Some(guid) = guids.get(&which) {
                    let _ = app.emit(
                        "joy-input",
                        InputEvent::Button { guid: guid.clone(), index: button_idx, pressed: true },
                    );
                }
            }
            Event::JoyButtonUp { which, button_idx, .. } => {
                if let Some(guid) = guids.get(&which) {
                    let _ = app.emit(
                        "joy-input",
                        InputEvent::Button { guid: guid.clone(), index: button_idx, pressed: false },
                    );
                }
            }
            Event::JoyHatMotion { which, hat_idx, state, .. } => {
                if let Some(guid) = guids.get(&which) {
                    let _ = app.emit(
                        "joy-input",
                        InputEvent::Hat { guid: guid.clone(), index: hat_idx, direction: hat_direction(state) },
                    );
                }
            }
            Event::JoyAxisMotion { which, axis_idx, value, .. } => {
                let prev = last_axis.get(&(which, axis_idx)).copied().unwrap_or(0);
                if (value as i32 - prev as i32).abs() > AXIS_EMIT_THRESHOLD {
                    last_axis.insert((which, axis_idx), value);
                    if let Some(guid) = guids.get(&which) {
                        let _ = app.emit(
                            "joy-input",
                            InputEvent::Axis { guid: guid.clone(), index: axis_idx, value },
                        );
                    }
                }
            }
            Event::Quit { .. } => break,
            _ => {}
        }
    }

    Ok(())
}

/// Re-enumerate all joysticks: reopen every device, rebuild the GUID map and
/// the shared device list, and tell the frontend the list changed.
fn reopen_all(
    joystick: &JoystickSubsystem,
    opened: &mut HashMap<u32, Joystick>,
    guids: &mut HashMap<u32, String>,
    app: &AppHandle,
    devices: &DeviceList,
) -> Result<(), String> {
    opened.clear();
    guids.clear();

    let hid = hid_table();
    let count = joystick.num_joysticks()?;
    let mut list = Vec::with_capacity(count as usize);
    for index in 0..count {
        let stick = match joystick.open(index) {
            Ok(stick) => stick,
            Err(e) => {
                warn!("failed to open joystick {index}: {e}");
                continue;
            }
        };
        let info = device_info(&stick, index, &hid);
        guids.insert(stick.instance_id(), info.sdl_guid.clone());
        list.push(info);
        opened.insert(stick.instance_id(), stick);
    }

    // SDL raises one JoyDeviceAdded per device at startup, each of which
    // lands here; only log the list when it actually differs.
    let changed = devices
        .lock()
        .map(|shared| shared.iter().map(|d| &d.sdl_guid).ne(list.iter().map(|d| &d.sdl_guid)))
        .unwrap_or(true);
    if changed {
        info!("enumerating joysticks: {count} found");
        for info in &list {
            let name = info.sc_name.as_deref().unwrap_or(&info.sdl_name);
            info!(
                "device {}: {name} sdl_guid={} sc_guid={} buttons={} axes={} hats={} sc_axes={:?}",
                info.index,
                info.sdl_guid,
                info.sc_product_guid.as_deref().unwrap_or("-"),
                info.num_buttons,
                info.num_axes,
                info.num_hats,
                info.axes
            );
            if let Some(err) = &info.axes_error {
                warn!("device {}: {name}: axes unavailable: {err}", info.index);
            }
        }
    } else {
        debug!("enumerating joysticks: {count} found, unchanged");
    }

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
