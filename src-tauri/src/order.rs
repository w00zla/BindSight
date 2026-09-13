//! SC's joystick order — where every `jsN` comes from.
//!
//! SC ranks its joysticks by DirectInput's `EnumDevices` order at start:
//! `js1` is the first device listed, `js2` the second, and so on. A device
//! plugged in or out shifts everything behind it. Gamepads (`gp1`) and the
//! keyboard (`kb1`) take no slot.
//!
//! Sources of that order:
//!
//! - **Windows**: DirectInput itself, live (`dinput.rs`) — the same call SC
//!   makes, so the rank and the `guidProduct` SC writes match byte for byte.
//! - **Linux**: Wine's DirectInput enumeration replicated (`wineorder.rs`):
//!   the key order of the HID device interfaces winebus registers, from the
//!   devices SDL and hidapi list.
//!
//! `Game.log` (`gamelog.rs`) is never the source, only the second opinion:
//! the game keeps the order it started with until it restarts, so a live
//! order that differs from the logged one means "restart the game".

use serde::Serialize;

use crate::input::DeviceInfo;
use crate::scdata::JoystickDevice;

/// This platform's live order source for the devices currently listed
/// (`devices` is the SDL list; DirectInput ignores it): `None` only on a
/// platform without one.
pub fn live(devices: &[DeviceInfo]) -> Option<Result<DeviceOrder, String>> {
    #[cfg(windows)]
    {
        let _ = devices;
        Some(crate::dinput::enumerate())
    }
    #[cfg(target_os = "linux")]
    {
        Some(crate::wineorder::enumerate(devices))
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = devices;
        None
    }
}

/// The joysticks in SC's order, `instance` being the `jsN` number.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct DeviceOrder {
    pub joysticks: Vec<JoystickDevice>,
    /// When this order was taken: the log's own timestamp of its last
    /// `Connected joystick` line, or the time a DirectInput enumeration
    /// changed. RFC 3339, `None` when unknown.
    pub timestamp: Option<String>,
}

impl DeviceOrder {
    /// The `jsN` of the device with this Product GUID, or `None` if the
    /// order does not list it. Case-insensitive on the GUID.
    pub fn instance_for_guid(&self, sc_product_guid: &str) -> Option<u32> {
        self.joysticks
            .iter()
            .find(|j| j.product_guid.as_deref().is_some_and(|g| g.eq_ignore_ascii_case(sc_product_guid)))
            .map(|j| j.instance)
    }

    /// Whether two orders rank the same GUIDs the same way. Names are not
    /// compared: the log and DirectInput trim them differently.
    pub fn same_ranking(&self, other: &DeviceOrder) -> bool {
        let key = |o: &DeviceOrder| -> Vec<(u32, String)> {
            o.joysticks
                .iter()
                .map(|j| (j.instance, j.product_guid.as_deref().unwrap_or("").to_ascii_lowercase()))
                .collect()
        };
        key(self) == key(other)
    }

    /// One line for the app log: `js1=<name> <guid>, js2=…`.
    pub fn describe(&self) -> String {
        self.joysticks
            .iter()
            .map(|j| format!("js{}={} {}", j.instance, j.product_name, j.product_guid.as_deref().unwrap_or("?")))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order(devices: &[(u32, &str)]) -> DeviceOrder {
        DeviceOrder {
            joysticks: devices
                .iter()
                .map(|(i, g)| JoystickDevice { instance: *i, product_name: format!("dev{i}"), product_guid: Some(g.to_string()) })
                .collect(),
            timestamp: None,
        }
    }

    const A: &str = "{0201231D-0000-0000-0000-504944564944}";
    const B: &str = "{0200231D-0000-0000-0000-504944564944}";
    const C: &str = "{0E213434-0000-0000-0000-504944564944}";

    #[test]
    fn instance_lookup_is_case_insensitive() {
        let o = order(&[(1, A), (2, B)]);
        assert_eq!(o.instance_for_guid(&A.to_lowercase()), Some(1));
        assert_eq!(o.instance_for_guid(B), Some(2));
        assert_eq!(o.instance_for_guid(C), None);
    }

    #[test]
    fn same_ranking_ignores_names_and_guid_case() {
        let live = order(&[(1, A), (2, B)]);
        let mut other = order(&[(1, &A.to_lowercase()), (2, B)]);
        other.joysticks[0].product_name = "VKBsim Gladiator EVO  L".into();
        assert!(live.same_ranking(&other));
        // A device in between shifts the rest; the same devices swapped differ too.
        assert!(!live.same_ranking(&order(&[(1, A), (2, C), (3, B)])));
        assert!(!live.same_ranking(&order(&[(1, B), (2, A)])));
        assert!(!live.same_ranking(&DeviceOrder::default()));
    }

    #[test]
    fn describe_lists_slots() {
        assert_eq!(order(&[(1, A)]).describe(), format!("js1=dev1 {A}"));
        assert_eq!(DeviceOrder::default().describe(), "");
    }
}
