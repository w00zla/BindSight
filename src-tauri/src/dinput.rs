//! SC's joystick order, taken the way SC takes it: DirectInput 8
//! `EnumDevices(DI8DEVCLASS_GAMECTRL, DIEDFL_ATTACHEDONLY)`. The rank in that
//! enumeration is `jsN`, and `guidProduct` is byte for byte the GUID SC writes
//! as `options/@Product` (vendor/product up front, `PIDVID` at the end).
//! Verified against `Game.log` and a saved `actionmaps.xml` (2026-09-12).
//!
//! XInput devices (a device path carrying `ig_`, Microsoft's own tell) are
//! what SC drives through XInput and logs as `xinput`, not `joystick`; they
//! take no slot and are skipped.
//!
//! Windows only (see `order::live`). The pure helpers compile everywhere for
//! the tests.

use crate::order::DeviceOrder;
use crate::scdata::JoystickDevice;

/// One device as DirectInput lists it, in enumeration order.
#[derive(Debug, Clone, PartialEq)]
pub struct DiDevice {
    pub product_name: String,
    pub instance_name: String,
    /// Formatted like SC writes it: `{0200231D-0000-0000-0000-504944564944}`.
    pub product_guid: String,
    pub instance_guid: String,
    pub dev_type: u32,
    pub usage_page: u16,
    pub usage: u16,
    /// `DIPROP_GUIDANDPATH` device path, empty if the property could not be
    /// read.
    pub path: String,
}

impl DiDevice {
    /// An XInput device: SC lists it as `xinput`, never as a joystick.
    pub fn is_xinput(&self) -> bool {
        is_xinput_path(&self.path)
    }
}

/// Whether a DirectInput device path marks an XInput device (`ig_` in the
/// hardware id, any case — DirectInput hands the path out lower-case).
pub fn is_xinput_path(path: &str) -> bool {
    path.to_ascii_lowercase().contains("ig_")
}

/// SC's registry-style, upper-case GUID form from the raw fields.
pub fn guid_string(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> String {
    format!(
        "{{{data1:08X}-{data2:04X}-{data3:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        data4[0], data4[1], data4[2], data4[3], data4[4], data4[5], data4[6], data4[7]
    )
}

/// The order as SC assigns it: non-XInput devices ranked `js1`, `js2`, … in
/// DirectInput order, XInput devices skipped. No timestamp — the caller
/// stamps it when the order changes.
pub fn to_order(devices: &[DiDevice]) -> DeviceOrder {
    let mut joysticks: Vec<JoystickDevice> = Vec::new();
    for d in devices.iter().filter(|d| !d.is_xinput()) {
        joysticks.push(JoystickDevice {
            instance: joysticks.len() as u32 + 1,
            product_name: d.product_name.trim().to_string(),
            product_guid: Some(d.product_guid.clone()),
        });
    }
    DeviceOrder { joysticks, timestamp: None }
}

/// SC's current joystick order, live from DirectInput.
#[cfg(windows)]
pub fn enumerate() -> Result<DeviceOrder, String> {
    list().map(|d| to_order(&d)).map_err(|e| format!("DirectInput: {e}"))
}

#[cfg(windows)]
pub use win::list;

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    use std::ptr;

    use windows_sys::core::{GUID, HRESULT};
    use windows_sys::Win32::Devices::HumanInterfaceDevice::{
        DirectInput8Create, DIDEVICEINSTANCEW, DIPROPGUIDANDPATH, DIPROPHEADER, DI8DEVCLASS_GAMECTRL,
        DIEDFL_ATTACHEDONLY, DIENUM_CONTINUE, DIPH_DEVICE, DIRECTINPUT_VERSION,
    };
    use windows_sys::Win32::Foundation::BOOL;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;

    use super::{guid_string, DiDevice};

    /// `IID_IDirectInput8W` {BF798031-483A-4DA2-AA99-5D64ED369700}.
    const IID_IDIRECTINPUT8W: GUID = GUID::from_u128(0xbf798031_483a_4da2_aa99_5d64ed369700);

    /// `DIPROP_GUIDANDPATH` is `MAKEDIPROP(12)`: the *pointer value* 12, not
    /// a GUID in memory (DirectInput tells predefined properties apart by the
    /// address). windows-sys exposes it as a GUID constant, which would be
    /// wrong to pass by reference.
    const DIPROP_GUIDANDPATH_PTR: *const GUID = 12 as *const GUID;

    // windows-sys ships no COM vtables; these are the two we need, laid out
    // like the SDK headers (IUnknown first). Entries we never call are
    // opaque pointers.
    type Unused = *const c_void;

    #[repr(C)]
    struct IDirectInput8WVtbl {
        query_interface: Unused,
        add_ref: Unused,
        release: unsafe extern "system" fn(this: *mut c_void) -> u32,
        create_device:
            unsafe extern "system" fn(this: *mut c_void, rguid: *const GUID, device: *mut *mut c_void, outer: *mut c_void) -> HRESULT,
        enum_devices: unsafe extern "system" fn(
            this: *mut c_void,
            dev_type: u32,
            callback: unsafe extern "system" fn(*mut DIDEVICEINSTANCEW, *mut c_void) -> BOOL,
            context: *mut c_void,
            flags: u32,
        ) -> HRESULT,
        // GetDeviceStatus, RunControlPanel, Initialize, FindDevice,
        // EnumDevicesBySemantics, ConfigureDevices — unused.
    }

    #[repr(C)]
    struct IDirectInputDevice8WVtbl {
        query_interface: Unused,
        add_ref: Unused,
        release: unsafe extern "system" fn(this: *mut c_void) -> u32,
        get_capabilities: Unused,
        enum_objects: Unused,
        get_property: unsafe extern "system" fn(this: *mut c_void, prop: *const GUID, header: *mut DIPROPHEADER) -> HRESULT,
        // SetProperty … GetImageInfo — unused.
    }

    fn wide(s: &[u16]) -> String {
        let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
        String::from_utf16_lossy(&s[..end])
    }

    fn guid(g: &GUID) -> String {
        guid_string(g.data1, g.data2, g.data3, g.data4)
    }

    unsafe extern "system" fn collect(instance: *mut DIDEVICEINSTANCEW, context: *mut c_void) -> BOOL {
        let list = &mut *(context as *mut Vec<DIDEVICEINSTANCEW>);
        list.push(*instance);
        DIENUM_CONTINUE as BOOL
    }

    /// The device path via `DIPROP_GUIDANDPATH` on a freshly created device
    /// object, released right after.
    unsafe fn device_path(di: *mut c_void, instance: &GUID) -> Result<String, String> {
        let vtbl = *(di as *const *const IDirectInput8WVtbl);
        let mut dev: *mut c_void = ptr::null_mut();
        let hr = ((*vtbl).create_device)(di, instance, &mut dev, ptr::null_mut());
        if hr < 0 || dev.is_null() {
            return Err(format!("CreateDevice failed: 0x{:08x}", hr as u32));
        }
        let dvtbl = *(dev as *const *const IDirectInputDevice8WVtbl);
        let mut prop = DIPROPGUIDANDPATH {
            diph: DIPROPHEADER {
                dwSize: std::mem::size_of::<DIPROPGUIDANDPATH>() as u32,
                dwHeaderSize: std::mem::size_of::<DIPROPHEADER>() as u32,
                dwObj: 0,
                dwHow: DIPH_DEVICE,
            },
            guidClass: GUID::from_u128(0),
            wszPath: [0; 260],
        };
        let hr = ((*dvtbl).get_property)(dev, DIPROP_GUIDANDPATH_PTR, &mut prop.diph);
        ((*dvtbl).release)(dev);
        if hr < 0 {
            return Err(format!("GetProperty(DIPROP_GUIDANDPATH) failed: 0x{:08x}", hr as u32));
        }
        Ok(wide(&prop.wszPath))
    }

    /// Every attached game controller in DirectInput's enumeration order.
    /// A device whose path cannot be read is kept with an empty path (and
    /// so counts as a joystick); the failure is logged.
    pub fn list() -> Result<Vec<DiDevice>, String> {
        unsafe {
            let hinst = GetModuleHandleW(ptr::null());
            let mut di: *mut c_void = ptr::null_mut();
            let hr = DirectInput8Create(hinst, DIRECTINPUT_VERSION, &IID_IDIRECTINPUT8W, &mut di, ptr::null_mut());
            if hr < 0 || di.is_null() {
                return Err(format!("DirectInput8Create failed: 0x{:08x}", hr as u32));
            }
            let vtbl = *(di as *const *const IDirectInput8WVtbl);

            let mut raw: Vec<DIDEVICEINSTANCEW> = Vec::new();
            let hr = ((*vtbl).enum_devices)(
                di,
                DI8DEVCLASS_GAMECTRL,
                collect,
                &mut raw as *mut Vec<DIDEVICEINSTANCEW> as *mut c_void,
                DIEDFL_ATTACHEDONLY,
            );
            if hr < 0 {
                ((*vtbl).release)(di);
                return Err(format!("EnumDevices failed: 0x{:08x}", hr as u32));
            }

            let devices = raw
                .iter()
                .map(|d| DiDevice {
                    product_name: wide(&d.tszProductName),
                    instance_name: wide(&d.tszInstanceName),
                    product_guid: guid(&d.guidProduct),
                    instance_guid: guid(&d.guidInstance),
                    dev_type: d.dwDevType,
                    usage_page: d.wUsagePage,
                    usage: d.wUsage,
                    path: device_path(di, &d.guidInstance).unwrap_or_else(|e| {
                        log::warn!("DirectInput path of {}: {e}", wide(&d.tszProductName));
                        String::new()
                    }),
                })
                .collect();
            ((*vtbl).release)(di);
            Ok(devices)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(name: &str, guid: &str, path: &str) -> DiDevice {
        DiDevice {
            product_name: name.into(),
            instance_name: name.into(),
            product_guid: guid.into(),
            instance_guid: String::new(),
            dev_type: 0x10318,
            usage_page: 1,
            usage: 4,
            path: path.into(),
        }
    }

    const VKB_L: &str = "{0201231D-0000-0000-0000-504944564944}";
    const VKB_R: &str = "{0200231D-0000-0000-0000-504944564944}";
    const XBOX: &str = "{02FD045E-0000-0000-0000-504944564944}";

    #[test]
    fn guid_is_sc_style() {
        // VID 231D / PID 0201 the way DirectInput packs it, PIDVID at the end.
        assert_eq!(guid_string(0x0201231d, 0, 0, [0, 0, 0x50, 0x49, 0x44, 0x56, 0x49, 0x44]), VKB_L);
    }

    #[test]
    fn xinput_is_told_by_ig_in_the_path_any_case() {
        assert!(is_xinput_path(r"\\?\hid#vid_045e&pid_02fd&ig_00#8&2b0d7f5&0&0000#{4d1e...}"));
        assert!(is_xinput_path(r"\\?\HID#VID_045E&PID_02FD&IG_00#..."));
        assert!(!is_xinput_path(r"\\?\hid#vid_231d&pid_0201#a&1fd7c509&0&0000#{4d1e...}"));
        assert!(!is_xinput_path(""));
    }

    #[test]
    fn order_ranks_joysticks_and_skips_pads() {
        let devices = [
            dev(" VKBsim Gladiator EVO  L  ", VKB_L, r"\\?\hid#vid_231d&pid_0201#a&1#{x}"),
            dev("Controller (Xbox One For Windows)", XBOX, r"\\?\hid#vid_045e&pid_02fd&ig_00#8&2#{x}"),
            dev(" VKBsim Gladiator EVO  R  ", VKB_R, r"\\?\hid#vid_231d&pid_0200#a&2#{x}"),
        ];
        let o = to_order(&devices);
        assert_eq!(o.joysticks.len(), 2);
        assert_eq!(o.joysticks[0].instance, 1);
        assert_eq!(o.joysticks[0].product_name, "VKBsim Gladiator EVO  L");
        assert_eq!(o.joysticks[0].product_guid.as_deref(), Some(VKB_L));
        // The pad takes no slot: the right stick is js2, not js3.
        assert_eq!(o.joysticks[1].instance, 2);
        assert_eq!(o.joysticks[1].product_guid.as_deref(), Some(VKB_R));
        assert_eq!(o.timestamp, None);
    }

    #[test]
    fn nothing_attached_is_an_empty_order() {
        assert!(to_order(&[]).joysticks.is_empty());
    }
}
