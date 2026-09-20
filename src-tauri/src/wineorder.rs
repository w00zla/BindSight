//! Linux: SC's joystick order the way Wine's DirectInput yields it.
//!
//! Under Wine SC runs the same `EnumDevices` call as on Windows, and Wine
//! (11.7, the LUG runner) answers it from the registry: `dinput` walks
//! `SetupDiEnumDeviceInterfaces` over `DeviceClasses\{HID}`, setupapi
//! enumerates the subkeys with `RegEnumKeyExW`, and wineserver keeps subkeys
//! sorted (`find_subkey`: case-insensitive, at equal prefix the shorter
//! first). The order is therefore the alphabetical order of the
//! device-interface key names, which winebus + hidclass build as
//!
//! ```text
//! ##?#HID#VID_xxxx&PID_yyyy[&MI_nn][&IG_00]#<version>&<serial>&<uid>&<index>&<gp>#{4D1E55B2-…}
//! ```
//!
//! Different devices are ranked by (vendor id, product id); identical ones by
//! the rest, where `index` is winebus's counter per (vid, pid, interface) in
//! creation order — udev's enumeration at start, sorted by sysfs path. That
//! is the USB-port risk Windows has too.
//!
//! Which devices SC sees at all (`dinput/joystick_hid.c`,
//! `winebus.sys/main.c`, `bus_sdl.c`):
//!
//! - only a HID top-level usage of Joystick (1:4) or Gamepad (1:5);
//! - one backend per device: hidraw or SDL, decided per hidraw interface on
//!   its **first** top-level collection (`is_hidraw_enabled` via
//!   `get_device_usages`, which reads `CollectionDesc[0]`): a mouse,
//!   keyboard or digitizer first is never hidraw; anything outside Generic
//!   Desktop, or a Generic Desktop usage other than joystick / gamepad, is
//!   always hidraw; a joystick / gamepad first goes by a list of vendors /
//!   products (every VKB and VPC, some Thrustmaster / Fanatec / Simucube,
//!   DualShock 4 / DualSense, Atmel with 32 / 50 / 64 buttons —
//!   [`hidraw_preferred`], [`hidraw_taken`]). The hidraw node must open
//!   read-write or the device is absent; the SDL twin of a hidraw-preferred
//!   vendor's device is dropped either way;
//! - hidclass splits a hidraw interface with several top-level collections
//!   into one device per collection (`&ColNN`, 1-based, plus a `&NNNN`
//!   instance suffix, 0-based), and dinput takes the joystick / gamepad
//!   ones: the Keychron Link dongle (bar-code page first, joystick second,
//!   `MI_01&COL02`) is a joystick to the game while SDL never lists it;
//! - an SDL-fed device that SDL maps as a game controller (unless SDL calls
//!   it a wheel or flight stick), or that has exactly 6 axes and at least 14
//!   buttons, is a gamepad ([`sdl_is_gamepad`]): `&IG_00`, driven through
//!   winexinput, listed by SC as `xinput` without a slot. hidraw devices are
//!   never gamepads.
//!
//! Verified 2026-09-13 against the prefix's `system.reg`, sysfs and the
//! `Game.log` of the last start: VKB R `231D:0200` ranks before VKB L
//! `231D:0201` whatever evdev does; a Keychron K2 HE (6 axes, 16 buttons) is
//! `IG_00` and invisible to the game; the Keychron Link is `js3` via
//! hidraw (`system.reg` + Game.log, 2026-09-20). Not replicated: registry
//! overrides (`Software\Wine\WineBus`, `Software\Wine\DirectInput\Joysticks`),
//! the evdev backend (SDL switched off in the registry), winebus's
//! synthesized serial for devices without one (the key shows `0000`). The
//! pure parts compile everywhere for the tests; `enumerate` is Linux only
//! (see `order::live`).

use std::cmp::Ordering;
use std::collections::HashMap;

use serde::Serialize;

use crate::order::DeviceOrder;
use crate::scdata::JoystickDevice;

/// One device as winebus would expose it — the fields its HID interface key
/// is built from, plus SC's name for it.
#[derive(Debug, Clone, PartialEq)]
pub struct WineDevice {
    pub vid: u16,
    pub pid: u16,
    /// The hidraw interface number (`&MI_nn`); `None` for an SDL-fed device
    /// (winebus's `input = -1`).
    pub interface: Option<u32>,
    /// USB `bcdDevice` (hidraw) or SDL's product version.
    pub version: u16,
    /// The serial winebus found, `0000` when it found none.
    pub serial: String,
    /// `&IG_00`: an XInput device to the game, no slot.
    pub is_gamepad: bool,
    /// hidraw interface with several top-level collections: this device is
    /// its collection number `n` (0-based; hidclass writes `&COL{n+1}` and
    /// the instance suffix `&{n:04}`). `None` for a single collection or
    /// an SDL device.
    pub collection: Option<u32>,
    /// What SC shows: the USB product string (hidraw) or SDL's name.
    pub product_name: String,
    /// sysfs path of the node Wine opens — the creation-order tie-break
    /// between identical devices.
    pub syspath: String,
}

/// Whether winebus takes this vendor / product through hidraw instead of SDL
/// (`is_hidraw_enabled`, Wine 11.7, registry defaults). `buttons` matters
/// for Atmel only.
pub fn hidraw_preferred(vid: u16, pid: u16, buttons: u32) -> bool {
    match vid {
        // DualShock 4, DualSense (Edge).
        0x054c => matches!(pid, 0x05c4 | 0x09cc | 0x0ba0 | 0x0ce6 | 0x0df2),
        // ThrustMaster T-Rudder, TWCS Throttle, T.16000M.
        0x044f => matches!(pid, 0xb679 | 0xb687 | 0xb10a),
        // Simucube 2 Sport / Pro / Ultimate, Simucube 1.
        0x16d0 => matches!(pid, 0x0d61 | 0x0d60 | 0x0d5f | 0x0d5a),
        // Fanatec ClubSport Pedals.
        0x0eb7 => matches!(pid, 0x183b | 0x1839),
        // Every VKB and every VPC device.
        0x231d | 0x3344 => true,
        // Atmel: user-configured button limits, or the VPC MT-50 CM2.
        0x03eb => matches!(buttons, 32 | 50 | 64) || pid == 0x2055,
        _ => false,
    }
}

/// Whether winebus takes a hidraw interface at all (`is_hidraw_enabled`,
/// Wine 11.x `winebus.sys/main.c`), decided on the interface's first
/// top-level collection `first` = (usage page, usage): a digitizer, mouse or
/// keyboard first is never hidraw (the SDL twin decides); a joystick or
/// gamepad first goes by the vendor list; everything else — another usage
/// page, or a Generic Desktop usage such as multi-axis — is always hidraw.
/// The Keychron Link's joystick interface starts with a bar-code collection
/// and is hidraw to Wine, whatever SDL does with the device.
pub fn hidraw_taken(first: (u16, u16), vid: u16, pid: u16, buttons: u32) -> bool {
    match first {
        (0x0D, _) | (0x01, 0x02) | (0x01, 0x06) => false,
        (0x01, 0x04) | (0x01, 0x05) => hidraw_preferred(vid, pid, buttons),
        _ => true,
    }
}

/// Whether winebus's SDL backend flags the device as a gamepad
/// (`sdl_add_device`): SDL's controller mapping counts unless SDL calls the
/// device a wheel or flight stick, otherwise exactly 6 axes and at least 14
/// buttons do. `sdl_type` is the name `input.rs` reports.
pub fn sdl_is_gamepad(is_controller: bool, sdl_type: &str, axes: u32, buttons: u32) -> bool {
    if is_controller && sdl_type != "wheel" && sdl_type != "flight_stick" {
        return true;
    }
    axes == 6 && buttons >= 14
}

/// SC's Product GUID for a vendor / product: `{PPPPVVVV-0000-0000-0000-504944564944}`.
pub fn product_guid(vid: u16, pid: u16) -> String {
    crate::dinput::guid_string(((pid as u32) << 16) | vid as u32, 0, 0, [0, 0, 0x50, 0x49, 0x44, 0x56, 0x49, 0x44])
}

/// The device-interface key name without the `##?#` prefix and the class
/// GUID suffix (identical for every device, so they never decide). `uid` is
/// always 0 in Wine 11.7.
fn interface_key(d: &WineDevice, index: u32) -> String {
    let mut key = format!("HID#VID_{:04X}&PID_{:04X}", d.vid, d.pid);
    if let Some(mi) = d.interface {
        key.push_str(&format!("&MI_{mi:02}"));
    }
    if let Some(col) = d.collection {
        key.push_str(&format!("&COL{:02}", col + 1));
    }
    if d.is_gamepad {
        key.push_str("&IG_00");
    }
    key.push_str(&format!("#{}&{}&0&{}&{}", d.version, d.serial, index, d.is_gamepad as u8));
    if let Some(col) = d.collection {
        key.push_str(&format!("&{col:04}"));
    }
    key
}

/// wineserver's subkey order (`find_subkey`): case-insensitive over the
/// common length, then the shorter name first.
fn wine_key_cmp(a: &str, b: &str) -> Ordering {
    let la = a.chars().flat_map(char::to_lowercase);
    let lb = b.chars().flat_map(char::to_lowercase);
    for (x, y) in la.zip(lb) {
        match x.cmp(&y) {
            Ordering::Equal => {}
            other => return other,
        }
    }
    a.chars().count().cmp(&b.chars().count())
}

/// The order as SC assigns it: gamepads skipped, the rest ranked `js1`,
/// `js2`, … by Wine's key order. No timestamp — the caller stamps it when
/// the order changes.
pub fn rank(devices: &[WineDevice]) -> DeviceOrder {
    let mut joysticks: Vec<JoystickDevice> = Vec::new();
    for (_, i) in keyed(devices) {
        let d = &devices[i];
        if d.is_gamepad {
            continue;
        }
        joysticks.push(JoystickDevice {
            instance: joysticks.len() as u32 + 1,
            product_name: d.product_name.trim().to_string(),
            product_guid: Some(product_guid(d.vid, d.pid)),
            product: format!("{} {}", d.product_name.trim(), product_guid(d.vid, d.pid)),
        });
    }
    DeviceOrder { joysticks, timestamp: None }
}

/// Every device's interface key with its position in `devices`, in Wine's
/// key order — the list `rank` walks.
fn keyed(devices: &[WineDevice]) -> Vec<(String, usize)> {
    // winebus's `index`: per (vid, pid, interface) in creation order, which
    // is udev's enumeration — sorted by sysfs path.
    let mut creation: Vec<usize> = (0..devices.len()).collect();
    creation.sort_by(|&a, &b| devices[a].syspath.cmp(&devices[b].syspath));
    let mut counts: HashMap<(u16, u16, Option<u32>), u32> = HashMap::new();
    // The collections of one hidraw interface are one winebus device: they
    // share its index (their node), the collection suffix tells them apart.
    let mut node_index: HashMap<&str, u32> = HashMap::new();
    let mut keyed: Vec<(String, usize)> = Vec::with_capacity(devices.len());
    for i in creation {
        let d = &devices[i];
        let index = match node_index.get(d.syspath.as_str()) {
            Some(&n) if d.collection.is_some() => n,
            _ => {
                let count = counts.entry((d.vid, d.pid, d.interface)).or_default();
                let n = *count;
                *count += 1;
                node_index.insert(d.syspath.as_str(), n);
                n
            }
        };
        keyed.push((interface_key(d, index), i));
    }
    keyed.sort_by(|a, b| wine_key_cmp(&a.0, &b.0));
    keyed
}

/// Position in `devices` of the gamepad XInput lists first (user 0, the
/// game's `gp1`): the first `is_gamepad` device in Wine's key order.
pub fn first_gamepad(devices: &[WineDevice]) -> Option<usize> {
    keyed(devices).into_iter().map(|(_, i)| i).find(|&i| devices[i].is_gamepad)
}

/// One registered interface as the Device List shows it: the key Wine
/// ranks by and the device it belongs to (`product_guid` matches
/// `DeviceInfo::sc_product_guid`; a gamepad is listed with no slot).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WineKey {
    pub product_guid: String,
    pub key: String,
    pub is_gamepad: bool,
}

/// The interface keys in Wine's order, joysticks and gamepads alike.
pub fn keys(devices: &[WineDevice]) -> Vec<WineKey> {
    keyed(devices)
        .into_iter()
        .map(|(key, i)| {
            let d = &devices[i];
            WineKey { product_guid: product_guid(d.vid, d.pid), key, is_gamepad: d.is_gamepad }
        })
        .collect()
}

#[cfg(target_os = "linux")]
pub use linux::{enumerate, live_keys, syspath, wine_devices};

#[cfg(target_os = "linux")]
mod linux {
    use log::warn;

    use super::{hidraw_preferred, hidraw_taken, rank, sdl_is_gamepad, WineDevice, WineKey};
    use crate::input::DeviceInfo;
    use crate::order::DeviceOrder;
    use crate::scdata::DeviceKind;

    /// The sysfs path behind a device node (`/dev/hidraw3` ->
    /// `/sys/devices/…/hidraw/hidraw3`), the node itself when sysfs has no
    /// entry for it.
    pub fn syspath(class: &str, node: &str) -> String {
        let name = node.rsplit('/').next().unwrap_or(node);
        std::fs::canonicalize(format!("/sys/class/{class}/{name}"))
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| node.to_string())
    }

    /// Wine's view of the connected devices: every joystick / gamepad
    /// collection of a hidraw interface winebus takes through hidraw (and can
    /// open), plus every SDL device it does not. hidapi lists one entry per
    /// top-level collection, in descriptor order, all collections of an
    /// interface sharing its node path — so an interface's first entry is
    /// the collection winebus decides on.
    pub fn wine_devices(devices: &[DeviceInfo]) -> Result<Vec<WineDevice>, String> {
        let api = hidapi::HidApi::new().map_err(|e| format!("hidapi: {e}"))?;
        let sdl_buttons = |vid: u16, pid: u16| {
            devices.iter().find(|d| d.sdl_vendor == vid && d.sdl_product == pid).map(|d| d.num_buttons).unwrap_or(0)
        };
        let entries: Vec<_> = api.device_list().collect();
        let mut list = Vec::new();
        for (pos, dev) in entries.iter().enumerate() {
            // dinput opens only Generic Desktop joysticks (4) and gamepads (5).
            if dev.usage_page() != 0x01 || !matches!(dev.usage(), 4 | 5) {
                continue;
            }
            let (vid, pid) = (dev.vendor_id(), dev.product_id());
            let siblings: Vec<usize> = (0..entries.len()).filter(|&k| entries[k].path() == dev.path()).collect();
            let first = &entries[siblings[0]];
            if !hidraw_taken((first.usage_page(), first.usage()), vid, pid, sdl_buttons(vid, pid)) {
                continue;
            }
            let collection = (siblings.len() > 1).then(|| siblings.iter().position(|&k| k == pos).unwrap_or(0) as u32);
            let path = dev.path().to_string_lossy().into_owned();
            // winebus opens the node O_RDWR; a node it cannot open does not exist to the game.
            if let Err(e) = std::fs::OpenOptions::new().read(true).write(true).open(&path) {
                warn!("Wine cannot open {path} ({vid:04x}:{pid:04x}): {e} — the game will not see this device");
                continue;
            }
            list.push(WineDevice {
                vid,
                pid,
                interface: u32::try_from(dev.interface_number()).ok(),
                version: dev.release_number(),
                serial: dev.serial_number().filter(|s| !s.is_empty()).map_or_else(|| "0000".to_string(), str::to_string),
                is_gamepad: false,
                collection,
                product_name: dev.product_string().unwrap_or_default().to_string(),
                syspath: syspath("hidraw", &path),
            });
        }
        for d in devices {
            if matches!(d.kind, DeviceKind::Keyboard) {
                continue;
            }
            // The SDL twin of a hidraw device is ignored by winebus, opened or not.
            if hidraw_preferred(d.sdl_vendor, d.sdl_product, d.num_buttons) {
                continue;
            }
            list.push(WineDevice {
                vid: d.sdl_vendor,
                pid: d.sdl_product,
                interface: None,
                version: d.sdl_product_version,
                serial: d.sdl_serial.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| "0000".to_string()),
                is_gamepad: sdl_is_gamepad(matches!(d.kind, DeviceKind::Gamepad), &d.sdl_type, d.num_axes, d.num_buttons),
                collection: None,
                product_name: d.sdl_name.clone(),
                syspath: d.sdl_path.as_deref().map(|p| syspath("input", p)).unwrap_or_default(),
            });
        }
        Ok(list)
    }

    /// SC's current joystick order under Wine, replicated from the devices
    /// SDL lists (`devices`) and the hidraw interfaces hidapi lists.
    pub fn enumerate(devices: &[DeviceInfo]) -> Result<DeviceOrder, String> {
        wine_devices(devices).map(|d| rank(&d)).map_err(|e| format!("Wine order: {e}"))
    }

    /// The interface keys Wine registers for the listed devices, in its order.
    pub fn live_keys(devices: &[DeviceInfo]) -> Result<Vec<WineKey>, String> {
        wine_devices(devices).map(|d| super::keys(&d)).map_err(|e| format!("Wine order: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VKB_R: &str = "{0200231D-0000-0000-0000-504944564944}";
    const VKB_L: &str = "{0201231D-0000-0000-0000-504944564944}";
    const K2HE: &str = "{0E213434-0000-0000-0000-504944564944}";

    fn hidraw(vid: u16, pid: u16, name: &str, syspath: &str) -> WineDevice {
        WineDevice {
            vid,
            pid,
            interface: Some(0),
            version: 8465,
            serial: "0000".into(),
            is_gamepad: false,
            collection: None,
            product_name: name.into(),
            syspath: syspath.into(),
        }
    }

    const LINK: &str = "{D0303434-0000-0000-0000-504944564944}";

    /// One collection of a hidraw interface with several (the Keychron Link
    /// dongle: bar-code page first, joystick second, interface 1).
    fn collection(vid: u16, pid: u16, interface: u32, col: u32, name: &str, syspath: &str) -> WineDevice {
        WineDevice { interface: Some(interface), collection: Some(col), version: 54016, ..hidraw(vid, pid, name, syspath) }
    }

    #[test]
    fn first_collection_decides_hidraw() {
        // Bar-code page first (the Keychron Link): hidraw whatever the vendor.
        assert!(hidraw_taken((0x8C, 0x01), 0x3434, 0xd030, 16));
        // Any other page, or a Generic Desktop usage that is no joystick /
        // gamepad (multi-axis, pointer): hidraw.
        assert!(hidraw_taken((0xFF60, 0x61), 0x3434, 0x0e21, 0));
        assert!(hidraw_taken((0x01, 0x08), 0x1234, 0x0001, 0));
        assert!(hidraw_taken((0x01, 0x01), 0x1234, 0x0001, 0));
        // Keyboard, mouse or digitizer first: never (the K2 HE's interface 0).
        assert!(!hidraw_taken((0x01, 0x06), 0x231d, 0x0200, 79));
        assert!(!hidraw_taken((0x01, 0x02), 0x231d, 0x0200, 79));
        assert!(!hidraw_taken((0x0D, 0x04), 0x231d, 0x0200, 79));
        // Joystick or gamepad first: the vendor list.
        assert!(hidraw_taken((0x01, 0x04), 0x231d, 0x0200, 79));
        assert!(!hidraw_taken((0x01, 0x04), 0x3434, 0x0e21, 16));
        assert!(hidraw_taken((0x01, 0x05), 0x3344, 0x0001, 0));
        assert!(!hidraw_taken((0x01, 0x05), 0x045e, 0x028e, 11));
    }

    #[test]
    fn collection_key_has_hidclass_layout() {
        // Verbatim from the prefix's system.reg (2026-09-20), serial aside.
        let link = collection(0x3434, 0xd030, 1, 1, "Keychron Link", "");
        assert_eq!(interface_key(&link, 0), "HID#VID_3434&PID_D030&MI_01&COL02#54016&0000&0&0&0&0001");
        let first = collection(0x3434, 0xd030, 1, 0, "Keychron Link", "");
        assert_eq!(interface_key(&first, 0), "HID#VID_3434&PID_D030&MI_01&COL01#54016&0000&0&0&0&0000");
    }

    #[test]
    fn collections_of_one_interface_share_its_index() {
        // Two joystick collections on one node: one winebus device (index 0
        // for both), two hidclass children, two slots. A second dongle would
        // be index 1.
        let devices = [
            collection(0x3434, 0xd030, 1, 1, "Link", "/sys/a"),
            collection(0x3434, 0xd030, 1, 0, "Link", "/sys/a"),
            collection(0x3434, 0xd030, 1, 0, "Link 2", "/sys/b"),
        ];
        let k: Vec<String> = keys(&devices).into_iter().map(|k| k.key).collect();
        assert_eq!(
            k,
            [
                "HID#VID_3434&PID_D030&MI_01&COL01#54016&0000&0&0&0&0000",
                "HID#VID_3434&PID_D030&MI_01&COL01#54016&0000&0&1&0&0000",
                "HID#VID_3434&PID_D030&MI_01&COL02#54016&0000&0&0&0&0001",
            ]
        );
        assert_eq!(rank(&devices).joysticks.len(), 3);
    }

    #[test]
    fn keychron_link_is_js3_behind_the_vkbs() {
        // The real case (Game.log 2026-09-19): R js1, L js2, the dongle's
        // joystick collection js3; the K2 HE itself is the gamepad.
        let devices = [
            collection(0x3434, 0xd030, 1, 1, "Keychron Link", "/sys/usb1/1-3"),
            sdl(0x3434, 0x0e21, "Keychron K2 HE", true, "/sys/usb2/2-1"),
            hidraw(0x231d, 0x0201, " VKBsim Gladiator EVO  L  ", "/sys/usb3/3-2"),
            hidraw(0x231d, 0x0200, " VKBsim Gladiator EVO  R  ", "/sys/usb3/3-3"),
        ];
        let o = rank(&devices);
        let got: Vec<_> = o.joysticks.iter().map(|j| (j.instance, j.product_guid.clone().unwrap())).collect();
        assert_eq!(got, [(1, VKB_R.into()), (2, VKB_L.into()), (3, LINK.into())]);
    }

    fn sdl(vid: u16, pid: u16, name: &str, is_gamepad: bool, syspath: &str) -> WineDevice {
        WineDevice { interface: None, version: 273, is_gamepad, ..hidraw(vid, pid, name, syspath) }
    }

    #[test]
    fn hidraw_list_matches_winebus() {
        assert!(hidraw_preferred(0x231d, 0x0200, 79)); // every VKB
        assert!(hidraw_preferred(0x231d, 0xffff, 0));
        assert!(hidraw_preferred(0x3344, 0x0001, 0)); // every VPC
        assert!(hidraw_preferred(0x044f, 0xb10a, 16)); // T.16000M
        assert!(!hidraw_preferred(0x044f, 0xb108, 16)); // another ThrustMaster: SDL
        assert!(hidraw_preferred(0x054c, 0x0ce6, 0)); // DualSense
        assert!(hidraw_preferred(0x03eb, 0x1234, 50)); // Atmel by button count
        assert!(!hidraw_preferred(0x03eb, 0x1234, 20));
        assert!(hidraw_preferred(0x03eb, 0x2055, 20)); // Atmel by pid
        assert!(!hidraw_preferred(0x3434, 0x0e21, 16)); // Keychron: SDL
        assert!(!hidraw_preferred(0x045e, 0x028e, 11));
    }

    #[test]
    fn gamepad_rule_matches_winebus() {
        // SDL's controller mapping wins …
        assert!(sdl_is_gamepad(true, "gamecontroller", 6, 11));
        assert!(sdl_is_gamepad(true, "unknown", 4, 10));
        // … unless SDL calls it a wheel or flight stick, then the heuristic decides.
        assert!(!sdl_is_gamepad(true, "flight_stick", 4, 20));
        assert!(!sdl_is_gamepad(true, "wheel", 3, 20));
        assert!(sdl_is_gamepad(true, "wheel", 6, 14));
        // The heuristic: exactly 6 axes and 14+ buttons. The K2 HE case.
        assert!(sdl_is_gamepad(false, "unknown", 6, 16));
        assert!(sdl_is_gamepad(false, "unknown", 6, 14));
        assert!(!sdl_is_gamepad(false, "unknown", 6, 13));
        assert!(!sdl_is_gamepad(false, "unknown", 7, 30));
        assert!(!sdl_is_gamepad(false, "unknown", 5, 30));
        // A VKB fed through SDL would fall in too — which is why the hidraw
        // split comes first.
        assert!(sdl_is_gamepad(false, "unknown", 6, 79));
    }

    #[test]
    fn product_guid_is_sc_style() {
        assert_eq!(product_guid(0x231d, 0x0200), VKB_R);
        assert_eq!(product_guid(0x3434, 0x0e21), K2HE);
    }

    #[test]
    fn key_has_wines_layout() {
        let r = hidraw(0x231d, 0x0200, "", "");
        assert_eq!(interface_key(&r, 0), "HID#VID_231D&PID_0200&MI_00#8465&0000&0&0&0");
        let mut pad = sdl(0x3434, 0x0e21, "", true, "");
        pad.serial = "0000".into();
        assert_eq!(interface_key(&pad, 2), "HID#VID_3434&PID_0E21&IG_00#273&0000&0&2&1");
    }

    #[test]
    fn key_order_is_wineservers() {
        assert_eq!(wine_key_cmp("abc", "ABC"), Ordering::Equal);
        assert_eq!(wine_key_cmp("ab", "abc"), Ordering::Less);
        assert_eq!(wine_key_cmp("PID_0200&MI_00", "PID_0201"), Ordering::Less);
        // `&` (0x26) sorts after `#` (0x23): a plain instance before an `&MI_` one.
        assert_eq!(wine_key_cmp("PID_0200#1", "PID_0200&MI_00#1"), Ordering::Less);
    }

    #[test]
    fn ranks_by_vendor_and_product_not_by_sysfs() {
        // The real Linux case: L on the lower USB port, R still js1 (Game.log 2026-09-10).
        let devices = [
            hidraw(0x231d, 0x0201, " VKBsim Gladiator EVO  L  ", "/sys/devices/pci0000:00/usb3/3-2/3-2.3/x"),
            hidraw(0x231d, 0x0200, " VKBsim Gladiator EVO  R  ", "/sys/devices/pci0000:00/usb3/3-2/3-2.4/x"),
        ];
        let o = rank(&devices);
        assert_eq!(o.joysticks.len(), 2);
        assert_eq!(o.joysticks[0].instance, 1);
        assert_eq!(o.joysticks[0].product_guid.as_deref(), Some(VKB_R));
        assert_eq!(o.joysticks[0].product_name, "VKBsim Gladiator EVO  R");
        assert_eq!(o.joysticks[1].instance, 2);
        assert_eq!(o.joysticks[1].product_guid.as_deref(), Some(VKB_L));
        assert_eq!(o.timestamp, None);
    }

    #[test]
    fn vendor_decides_before_product() {
        let devices = [hidraw(0x231d, 0x0001, "vkb", "/a"), hidraw(0x044f, 0xb10a, "tm", "/b")];
        let o = rank(&devices);
        assert_eq!(o.joysticks[0].product_name, "tm");
        assert_eq!(o.joysticks[1].product_name, "vkb");
    }

    #[test]
    fn gamepads_take_no_slot() {
        let devices = [
            hidraw(0x231d, 0x0200, "R", "/a"),
            sdl(0x3434, 0x0e21, "Keychron K2 HE", true, "/b"),
            sdl(0x045e, 0x028e, "Xbox", true, "/c"),
            sdl(0x1234, 0x0001, "stick", false, "/d"),
        ];
        let o = rank(&devices);
        let names: Vec<_> = o.joysticks.iter().map(|j| (j.instance, j.product_name.as_str())).collect();
        assert_eq!(names, [(1, "stick"), (2, "R")]);
    }

    #[test]
    fn keys_list_every_interface_in_wines_order() {
        // The Device List's view: gamepads included, each with the key Wine
        // ranks by and the Product GUID that names its `DeviceInfo`.
        let devices = [
            hidraw(0x231d, 0x0200, "R", "/a"),
            sdl(0x3434, 0x0e21, "Keychron K2 HE", true, "/b"),
            sdl(0x1234, 0x0001, "stick", false, "/d"),
        ];
        let keys = keys(&devices);
        let rows: Vec<(&str, &str, bool)> =
            keys.iter().map(|k| (k.product_guid.as_str(), k.key.as_str(), k.is_gamepad)).collect();
        assert_eq!(
            rows,
            [
                ("{00011234-0000-0000-0000-504944564944}", "HID#VID_1234&PID_0001#273&0000&0&0&0", false),
                (VKB_R, "HID#VID_231D&PID_0200&MI_00#8465&0000&0&0&0", false),
                (K2HE, "HID#VID_3434&PID_0E21&IG_00#273&0000&0&0&1", true),
            ]
        );
        // Same walk as the ranking: the joysticks' keys in this order are js1, js2.
        let o = rank(&devices);
        assert_eq!(o.joysticks.iter().map(|j| j.product_guid.as_deref().unwrap()).collect::<Vec<_>>(), [rows[0].0, rows[1].0]);
    }

    #[test]
    fn first_gamepad_is_xinput_user_0() {
        // The game's gp1 is XInput's user 0: the first gamepad in key order,
        // not SDL's order and not a joystick. Xbox (045E) before Keychron (3434).
        let devices = [
            sdl(0x3434, 0x0e21, "Keychron K2 HE", true, "/a"),
            hidraw(0x231d, 0x0200, "R", "/b"),
            sdl(0x045e, 0x028e, "Xbox", true, "/c"),
        ];
        assert_eq!(first_gamepad(&devices), Some(2));
        assert_eq!(first_gamepad(&devices[..2]), Some(0));
        assert_eq!(first_gamepad(&devices[1..2]), None);
    }

    #[test]
    fn identical_devices_rank_by_sysfs_path() {
        let devices = [
            hidraw(0x231d, 0x0200, "second", "/sys/devices/usb3/3-2/3-2.4/x"),
            hidraw(0x231d, 0x0200, "first", "/sys/devices/usb3/3-2/3-2.3/x"),
        ];
        let o = rank(&devices);
        assert_eq!(o.joysticks[0].product_name, "first");
        assert_eq!(o.joysticks[1].product_name, "second");
        // The counter is per interface: an SDL twin would not shift it.
        let mut with_sdl = devices.to_vec();
        with_sdl.push(sdl(0x231d, 0x0200, "sdl", false, "/sys/devices/usb3/3-2/3-2.1/x"));
        assert_eq!(interface_key(&with_sdl[2], 0), "HID#VID_231D&PID_0200#273&0000&0&0&0");
    }

    #[test]
    fn nothing_attached_is_an_empty_order() {
        assert!(rank(&[]).joysticks.is_empty());
    }
}
