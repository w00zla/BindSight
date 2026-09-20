//! SC's joystick order — where every `jsN` comes from.
//!
//! SC assigns its `jsN` slots at start, and the rule (test series v2,
//! Windows, SC 4.10, 2026-09-20) is:
//!
//! - **The saved device map decides.** While the set of attached joysticks
//!   is the one recorded in `actionmaps.xml`'s `<options type="joystick">`
//!   elements, every joystick takes the `instance` saved for its Product
//!   GUID. The enumeration order plays no part: swapping two `Product`
//!   attributes in the file swaps the two sticks in the cockpit.
//! - **A joystick gone:** the saved devices keep their relative order, the
//!   gap closes (`js4` becomes `js3`). The bindings do not move with them,
//!   so every stick behind the gap loses its bindings — the clash.
//! - **A joystick new:** one observation (a stick enumerated second, three
//!   saved: it took `js3`, the saved third moved to `js4`). Replicated here
//!   as "insert at its enumeration index plus one" — an assumption until
//!   more cases are seen, marked as such in the app log.
//!
//! The game writes the map it used at its next save (exit, keybinding
//! export); until then the file still shows the old map, which is what
//! [`assign`] reconciles with the enumeration from `Game.log`
//! (`gamelog.rs`: the game's own list of what it saw at its last start).
//! The live enumeration (`dinput.rs` / `wineorder.rs`) is diagnostics only.

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

/// Where an [`assign`]ed order came from.
#[derive(Debug, Clone, PartialEq)]
pub enum OrderSource {
    /// The attached set is the saved one: the slots are the file's.
    File,
    /// The set changed since the file was saved: the slots are derived
    /// (gap closed, new devices inserted — the latter an assumption).
    SetChanged { added: Vec<JoystickDevice>, removed: Vec<JoystickDevice> },
}

/// The slots SC assigns plus where they came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Assignment {
    pub order: DeviceOrder,
    pub source: OrderSource,
}

impl Assignment {
    /// One line for the app log naming the source and, on a set change,
    /// the devices that came and went.
    pub fn describe_source(&self) -> String {
        match &self.source {
            OrderSource::File => "saved device map (attached set unchanged)".into(),
            OrderSource::SetChanged { added, removed } => {
                let names = |v: &[JoystickDevice]| v.iter().map(|d| d.product_name.clone()).collect::<Vec<_>>().join(", ");
                format!(
                    "device set changed since the last save (added: [{}], removed: [{}]); slots derived, new devices by assumption",
                    names(added),
                    names(removed)
                )
            }
        }
    }
}

/// The logged enumeration reduced to the joysticks attached now (`attached`:
/// Product GUIDs, any case), keeping the log's order, plus the devices
/// dropped — the log is the game's last start, an unplugged stick is not
/// what it enumerates next time. A logged device without a GUID is kept.
pub fn attached_only(log: &DeviceOrder, attached: &[String]) -> (DeviceOrder, Vec<JoystickDevice>) {
    let is_attached = |d: &JoystickDevice| match &d.product_guid {
        Some(g) => attached.iter().any(|a| a.eq_ignore_ascii_case(g)),
        None => true,
    };
    let (kept, gone): (Vec<JoystickDevice>, Vec<JoystickDevice>) = log.joysticks.iter().cloned().partition(is_attached);
    (DeviceOrder { joysticks: kept, timestamp: log.timestamp.clone() }, gone)
}

/// The `jsN` SC assigns each joystick of `enumerated` (the `Game.log`
/// order), given the device map `saved` (the file's `<options>`), by the
/// rule in the module docs. Devices are matched by Product GUID, a GUID
/// saved twice matches its log occurrences in slot order (two identical
/// sticks; SC's own behaviour there is unverified). Known devices carry the
/// file's raw `product`, new ones the log's.
pub fn assign(enumerated: &DeviceOrder, saved: &[JoystickDevice]) -> Assignment {
    // Saved entries with a GUID, in slot order, each usable once.
    let mut pool: Vec<(&JoystickDevice, bool)> = saved.iter().filter(|d| d.product_guid.is_some()).map(|d| (d, false)).collect();
    pool.sort_by_key(|(d, _)| d.instance);

    // (log index, log device, its saved entry if any)
    let mut matched: Vec<(usize, &JoystickDevice, Option<&JoystickDevice>)> = Vec::new();
    for (i, dev) in enumerated.joysticks.iter().enumerate() {
        let hit = pool.iter_mut().find(|(s, used)| {
            !*used && matches!((&s.product_guid, &dev.product_guid), (Some(a), Some(b)) if a.eq_ignore_ascii_case(b))
        });
        let entry = hit.map(|(s, used)| {
            *used = true;
            *s
        });
        matched.push((i, dev, entry));
    }

    let added: Vec<JoystickDevice> = matched.iter().filter(|(_, _, e)| e.is_none()).map(|(_, d, _)| (*d).clone()).collect();
    let removed: Vec<JoystickDevice> = pool.iter().filter(|(_, used)| !used).map(|(d, _)| (*d).clone()).collect();

    let with_slot = |dev: &JoystickDevice, entry: Option<&JoystickDevice>, instance: u32| JoystickDevice {
        instance,
        product_name: dev.product_name.clone(),
        product_guid: dev.product_guid.clone(),
        product: entry.map(|e| e.product.clone()).unwrap_or_else(|| dev.product.clone()),
    };

    let mut joysticks: Vec<JoystickDevice>;
    let source;
    if added.is_empty() && removed.is_empty() {
        joysticks = matched.iter().map(|(_, d, e)| with_slot(d, *e, e.map(|e| e.instance).unwrap_or(0))).collect();
        joysticks.sort_by_key(|j| j.instance);
        source = OrderSource::File;
    } else {
        // Known devices in the file's slot order (ties by enumeration), gap
        // closed; then each new device inserted at enumeration index + 1
        // (0-based), capped at the end — the one observed case.
        let mut known: Vec<&(usize, &JoystickDevice, Option<&JoystickDevice>)> = matched.iter().filter(|(_, _, e)| e.is_some()).collect();
        known.sort_by_key(|(i, _, e)| (e.map(|e| e.instance).unwrap_or(0), *i));
        let mut list: Vec<(&JoystickDevice, Option<&JoystickDevice>)> = known.iter().map(|(_, d, e)| (*d, *e)).collect();
        for (i, dev, _) in matched.iter().filter(|(_, _, e)| e.is_none()) {
            let at = (*i + 1).min(list.len());
            list.insert(at, (dev, None));
        }
        joysticks = list.iter().enumerate().map(|(n, (d, e))| with_slot(d, *e, n as u32 + 1)).collect();
        source = OrderSource::SetChanged { added, removed };
    }

    Assignment { order: DeviceOrder { joysticks, timestamp: enumerated.timestamp.clone() }, source }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order(devices: &[(u32, &str)]) -> DeviceOrder {
        DeviceOrder {
            joysticks: devices
                .iter()
                .map(|(i, g)| JoystickDevice {
                    instance: *i,
                    product_name: format!("dev{i}"),
                    product_guid: Some(g.to_string()),
                    product: format!("dev{i} {g}"),
                })
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

    // The test series v2 setup: L, Link, K2 HE, R by GUID; the game
    // enumerates L, K2HE, Link, R (`Game.log`) in every step.
    const L: &str = "{0201231D-0000-0000-0000-504944564944}";
    const LINK: &str = "{D0303434-0000-0000-0000-504944564944}";
    const K2HE: &str = "{0E213434-0000-0000-0000-504944564944}";
    const R: &str = "{0200231D-0000-0000-0000-504944564944}";

    /// The log's enumeration, `js1..` in the given order.
    fn log(guids: &[&str]) -> DeviceOrder {
        let mut o = order(&guids.iter().enumerate().map(|(i, g)| (i as u32 + 1, *g)).collect::<Vec<_>>());
        o.timestamp = Some("2026-09-20T13:04:52.214Z".into());
        o
    }

    /// The file's `<options>` with these (instance, guid) entries.
    fn saved(entries: &[(u32, &str)]) -> Vec<JoystickDevice> {
        order(entries).joysticks.into_iter().map(|mut j| {
            j.product = format!(" saved{} {}", j.instance, j.product_guid.clone().unwrap());
            j
        }).collect()
    }

    fn slots(a: &Assignment) -> Vec<(u32, String)> {
        a.order.joysticks.iter().map(|j| (j.instance, j.product_guid.clone().unwrap())).collect()
    }

    #[test]
    fn stable_set_takes_the_saved_slots_not_the_enumeration() {
        // Step A: file L1 Link2 K2HE3 R4, log L K2HE Link R -> export 1,3,2,4.
        let a = assign(&log(&[L, K2HE, LINK, R]), &saved(&[(1, L), (2, LINK), (3, K2HE), (4, R)]));
        assert_eq!(a.source, OrderSource::File);
        assert_eq!(slots(&a), vec![(1, L.into()), (2, LINK.into()), (3, K2HE.into()), (4, R.into())]);
        // Known devices carry the file's raw Product string.
        assert_eq!(a.order.joysticks[1].product, format!(" saved2 {LINK}"));
        assert_eq!(a.order.timestamp.as_deref(), Some("2026-09-20T13:04:52.214Z"));

        // Step B2: L and R swapped in the file -> swapped in the cockpit.
        let b2 = assign(&log(&[L, K2HE, LINK, R]), &saved(&[(1, R), (2, LINK), (3, K2HE), (4, L)]));
        assert_eq!(b2.source, OrderSource::File);
        assert_eq!(slots(&b2), vec![(1, R.into()), (2, LINK.into()), (3, K2HE.into()), (4, L.into())]);
    }

    #[test]
    fn a_removed_device_closes_the_gap() {
        // Step C: K2HE unplugged, file still L1 Link2 K2HE3 R4 -> L1 Link2 R3.
        let c = assign(&log(&[L, LINK, R]), &saved(&[(1, L), (2, LINK), (3, K2HE), (4, R)]));
        assert_eq!(slots(&c), vec![(1, L.into()), (2, LINK.into()), (3, R.into())]);
        match &c.source {
            OrderSource::SetChanged { added, removed } => {
                assert!(added.is_empty());
                assert_eq!(removed.len(), 1);
                assert_eq!(removed[0].product_guid.as_deref(), Some(K2HE));
            }
            other => panic!("{other:?}"),
        }
        // The saved order counts, not the enumeration: file R1 L2, R gone -> L1.
        let c2 = assign(&log(&[LINK, L]), &saved(&[(1, R), (2, L), (3, LINK)]));
        assert_eq!(slots(&c2), vec![(1, L.into()), (2, LINK.into())]);
    }

    #[test]
    fn a_new_device_is_inserted_after_its_enumeration_index() {
        // Step D: file L1 Link2 R3, K2HE back at enumeration index 1 -> L1 Link2 K2HE3 R4.
        let d = assign(&log(&[L, K2HE, LINK, R]), &saved(&[(1, L), (2, LINK), (3, R)]));
        assert_eq!(slots(&d), vec![(1, L.into()), (2, LINK.into()), (3, K2HE.into()), (4, R.into())]);
        match &d.source {
            OrderSource::SetChanged { added, removed } => {
                assert!(removed.is_empty());
                assert_eq!(added[0].product_guid.as_deref(), Some(K2HE));
            }
            other => panic!("{other:?}"),
        }
        // The new device carries the log's raw Product string.
        assert_eq!(d.order.joysticks[2].product, format!("dev2 {K2HE}"));
        // Enumerated last -> appended.
        let last = assign(&log(&[L, LINK, R, K2HE]), &saved(&[(1, L), (2, LINK), (3, R)]));
        assert_eq!(slots(&last), vec![(1, L.into()), (2, LINK.into()), (3, R.into()), (4, K2HE.into())]);
        assert!(d.describe_source().contains("assumption"));
    }

    #[test]
    fn unplugged_since_the_game_start_is_a_set_change() {
        // The user's case: the log still lists K2 HE, it is unplugged now
        // (SDL and hidapi lack it); the file is step A's. The next start
        // will look like step C: R becomes js3.
        let full = log(&[L, K2HE, LINK, R]);
        let (now, gone) = attached_only(&full, &[L.to_lowercase(), LINK.into(), R.into()]);
        assert_eq!(gone.len(), 1);
        assert_eq!(gone[0].product_guid.as_deref(), Some(K2HE));
        assert_eq!(now.timestamp, full.timestamp);
        let a = assign(&now, &saved(&[(1, L), (2, LINK), (3, K2HE), (4, R)]));
        assert_eq!(slots(&a), vec![(1, L.into()), (2, LINK.into()), (3, R.into())]);
        assert!(matches!(a.source, OrderSource::SetChanged { .. }));
        // Everything attached: nothing dropped, and a logged device without
        // a GUID cannot be checked, so it stays.
        let mut o = log(&[L, R]);
        o.joysticks.push(JoystickDevice { instance: 3, product_name: "odd".into(), product_guid: None, product: "odd".into() });
        let (kept, gone) = attached_only(&o, &[L.into(), R.into()]);
        assert_eq!(kept.joysticks.len(), 3);
        assert!(gone.is_empty());
    }

    #[test]
    fn nothing_saved_means_enumeration_order() {
        let a = assign(&log(&[R, L]), &[]);
        assert_eq!(slots(&a), vec![(1, R.into()), (2, L.into())]);
        assert!(matches!(a.source, OrderSource::SetChanged { .. }));
        assert_eq!(assign(&DeviceOrder::default(), &[]).source, OrderSource::File);
        assert!(assign(&DeviceOrder::default(), &[]).order.joysticks.is_empty());
    }

    #[test]
    fn nothing_attached_removes_everything() {
        let a = assign(&DeviceOrder::default(), &saved(&[(1, L), (2, R)]));
        assert!(a.order.joysticks.is_empty());
        match a.source {
            OrderSource::SetChanged { removed, .. } => assert_eq!(removed.len(), 2),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn identical_sticks_match_in_slot_order() {
        // Two sticks with the same Product GUID saved as js2 and js3.
        let a = assign(&log(&[R, R, L]), &saved(&[(1, L), (2, R), (3, R)]));
        assert_eq!(a.source, OrderSource::File);
        assert_eq!(slots(&a), vec![(1, L.into()), (2, R.into()), (3, R.into())]);
        // One of them gone: set changed, the other keeps its place.
        let b = assign(&log(&[R, L]), &saved(&[(1, L), (2, R), (3, R)]));
        assert_eq!(slots(&b), vec![(1, L.into()), (2, R.into())]);
        assert!(matches!(b.source, OrderSource::SetChanged { .. }));
    }

    #[test]
    fn guid_match_ignores_case_and_a_saved_slot_without_guid() {
        let mut file = saved(&[(1, L), (2, R)]);
        file[0].product_guid = Some(L.to_lowercase());
        file.push(JoystickDevice { instance: 3, product_name: "junk".into(), product_guid: None, product: "junk".into() });
        let a = assign(&log(&[R, L]), &file);
        assert_eq!(a.source, OrderSource::File);
        // The assigned entry carries the log's spelling of the GUID.
        assert_eq!(slots(&a), vec![(1, L.into()), (2, R.into())]);
    }
}
