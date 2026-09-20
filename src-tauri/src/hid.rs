//! HID report descriptor → SC axis name per SDL axis index.
//!
//! SC (DirectInput) names joystick axes by their HID usage: X/Y/Z → `x`/`y`/
//! `z`, Rx/Ry/Rz → `rotx`/`roty`/`rotz`, Slider/Dial → `slider1`/`slider2`
//! in report order. SDL numbers axes in the *canonical* usage order, not the
//! report order — on Linux `SDL_sysjoystick.c` walks the evdev ABS codes
//! ascending (X=0 … RZ=5, THROTTLE=6, RUDDER=7; hats skipped), which the
//! kernel's `hid-input.c` assigns from the same usages; on Windows SDL's
//! DirectInput backend sorts objects by data offset (X, Y, Z, RX, RY, RZ,
//! sliders). So SDL index = rank of the axis in [X Y Z Rx Ry Rz Slider Slider]
//! among the axes the device has.
//!
//! Verified 2026-09-09 on a VKB Gladiator EVO R whose report order is
//! X Y Rz Z Rx Ry: SDL reported X=0, Y=1, slider(Z)=2, twist(Rz)=5, exactly the
//! canonical rank. Anything this module cannot place with that rule is an
//! error, never a guess; callers also check the count against SDL's.
//!
//! A descriptor may hold several top-level collections (a Keychron K2 HE's
//! interface 0: keyboard, mouse, joystick, …); the kernel, SDL and the game
//! see the joystick collection as a device of its own, so only the fields of
//! the joystick-class collections (usage joystick / gamepad / multi-axis)
//! count — the mouse's X/Y are not a second pair of axes. A descriptor
//! without such a collection is taken whole.

/// The HID usages that are axes here: Generic Desktop (page 1) X..Dial.
const PAGE_GENERIC_DESKTOP: u16 = 0x01;
const USAGE_X: u16 = 0x30;
const USAGE_RZ: u16 = 0x35;
const USAGE_SLIDER: u16 = 0x36;
const USAGE_DIAL: u16 = 0x37;

/// A HID `(usage page, usage)` pair.
type Usage = (u16, u16);

/// Top-level collection usages that make a HID device a joystick to the
/// game (and to `input.rs`'s interface pick): joystick, gamepad, multi-axis.
fn is_joystick_class(usage: Usage) -> bool {
    matches!(usage, (PAGE_GENERIC_DESKTOP, 0x04 | 0x05 | 0x08))
}

/// Ordered `(usage page, usage)` of every non-constant Input field in the
/// descriptor's joystick-class top-level collections (see the module doc;
/// the whole descriptor when it has none) that is at least 2 bits wide
/// (buttons are 1-bit arrays and never axes), in report order. Ranges are
/// expanded; an array field with fewer usages than its report count repeats
/// its last usage, as HID does. Long items are skipped.
pub fn axis_usages(desc: &[u8]) -> Vec<(u16, u16)> {
    // Every field with the usage of the top-level collection it sits in.
    let mut fields: Vec<(Option<Usage>, Usage)> = Vec::new();
    let mut i = 0;
    let mut usage_page: u16 = 0;
    let mut report_size: u32 = 0;
    let mut report_count: u32 = 0;
    let mut usages: Vec<(u16, u16)> = Vec::new();
    let mut usage_min: Option<(u16, u16)> = None;
    // Collection nesting: the usage of the open top-level collection.
    let mut depth: u32 = 0;
    let mut top: Option<(u16, u16)> = None;

    while i < desc.len() {
        let prefix = desc[i];
        if prefix == 0xFE {
            // Long item: [0xFE, size, tag, data...]
            let len = desc.get(i + 1).copied().unwrap_or(0) as usize;
            i += 3 + len;
            continue;
        }
        let size = match prefix & 0x3 {
            3 => 4,
            s => s as usize,
        };
        let typ = (prefix >> 2) & 0x3;
        let tag = prefix >> 4;
        let mut data: u32 = 0;
        for k in 0..size {
            data |= (desc.get(i + 1 + k).copied().unwrap_or(0) as u32) << (8 * k);
        }
        i += 1 + size;
        // A 4-byte usage carries its page in the high half.
        let split = |d: u32| -> (u16, u16) {
            if size == 4 {
                ((d >> 16) as u16, d as u16)
            } else {
                (usage_page, d as u16)
            }
        };
        match (typ, tag) {
            (1, 0x0) => usage_page = data as u16,
            (1, 0x7) => report_size = data,
            (1, 0x9) => report_count = data,
            (2, 0x0) => usages.push(split(data)),
            (2, 0x1) => usage_min = Some(split(data)),
            (2, 0x2) => {
                if let Some((page, lo)) = usage_min.take() {
                    let (_, hi) = split(data);
                    usages.extend((lo..=hi).map(|u| (page, u)));
                }
            }
            (0, 0x8) => {
                // Input main item; bit 0 = constant (padding).
                if data & 1 == 0 && report_size >= 2 {
                    for n in 0..report_count as usize {
                        let Some(&u) = usages.get(n).or(usages.last()) else {
                            break;
                        };
                        fields.push((top, u));
                    }
                }
                usages.clear();
                usage_min = None;
            }
            (0, 0xA) => {
                // Collection: a top-level one is named by its usage.
                if depth == 0 {
                    top = usages.first().copied();
                }
                depth += 1;
                usages.clear();
                usage_min = None;
            }
            (0, 0xC) => {
                // End Collection.
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    top = None;
                }
                usages.clear();
                usage_min = None;
            }
            (0, _) => {
                // Other main items (output/feature) consume locals.
                usages.clear();
                usage_min = None;
            }
            _ => {}
        }
    }
    let joystick_only = fields.iter().any(|(t, _)| t.is_some_and(is_joystick_class));
    fields
        .into_iter()
        .filter(|(t, _)| !joystick_only || t.is_some_and(is_joystick_class))
        .map(|(_, u)| u)
        .collect()
}

/// SC axis names in SDL index order for a device's axis usages (see the
/// module doc). Unrecognised usages are ignored — the caller compares the
/// resulting count with SDL's axis count, which catches anything that slipped
/// through. Errors are cases the rule cannot order.
pub fn sc_axis_names(usages: &[(u16, u16)]) -> Result<Vec<String>, String> {
    const NAMES: [&str; 6] = ["x", "y", "z", "rotx", "roty", "rotz"];
    let mut present = [false; 6];
    let mut sliders = Vec::new();
    for &(page, usage) in usages {
        if page != PAGE_GENERIC_DESKTOP {
            continue;
        }
        if (USAGE_X..=USAGE_RZ).contains(&usage) {
            let slot = (usage - USAGE_X) as usize;
            if present[slot] {
                return Err(format!("duplicate {} axis in HID descriptor", NAMES[slot]));
            }
            present[slot] = true;
        } else if usage == USAGE_SLIDER || usage == USAGE_DIAL {
            sliders.push(usage);
        }
    }
    if sliders.len() > 2 {
        return Err(format!("{} sliders/dials in HID descriptor, the game knows two", sliders.len()));
    }
    // Linux orders Slider (ABS_THROTTLE) before Dial (ABS_RUDDER) regardless of
    // report order, DirectInput keeps report order: only unambiguous when they
    // agree.
    if sliders == [USAGE_DIAL, USAGE_SLIDER] {
        return Err("dial before slider in HID descriptor: SDL order differs per platform".into());
    }
    let mut names: Vec<String> = (0..6).filter(|&i| present[i]).map(|i| NAMES[i].to_string()).collect();
    names.extend((1..=sliders.len()).map(|n| format!("slider{n}")));
    Ok(names)
}

/// Read a device's report descriptor (raw bytes).
pub fn read_descriptor(device: &hidapi::HidDevice) -> Result<Vec<u8>, String> {
    let mut buf = vec![0u8; 4096];
    let n = device.get_report_descriptor(&mut buf).map_err(|e| format!("HID descriptor: {e}"))?;
    buf.truncate(n);
    Ok(buf)
}

/// Derive the SC axis names from a report descriptor; `sdl_axes` is SDL's
/// axis count for the same device, which must match.
pub fn sc_axes_from(desc: &[u8], sdl_axes: u32) -> Result<Vec<String>, String> {
    let names = sc_axis_names(&axis_usages(desc))?;
    if names.len() as u32 != sdl_axes {
        return Err(format!("HID descriptor yields {} axes, SDL has {sdl_axes}", names.len()));
    }
    Ok(names)
}

/// Human name of a `(usage page, usage)` pair as it appears in the device
/// log: the Generic Desktop axes and hat, the Simulation Controls SC cares
/// about, buttons, else `page/usage` in hex.
pub fn usage_name(page: u16, usage: u16) -> String {
    match (page, usage) {
        (0x01, 0x30) => "X".into(),
        (0x01, 0x31) => "Y".into(),
        (0x01, 0x32) => "Z".into(),
        (0x01, 0x33) => "Rx".into(),
        (0x01, 0x34) => "Ry".into(),
        (0x01, 0x35) => "Rz".into(),
        (0x01, 0x36) => "Slider".into(),
        (0x01, 0x37) => "Dial".into(),
        (0x01, 0x38) => "Wheel".into(),
        (0x01, 0x39) => "Hat".into(),
        (0x02, 0xBA) => "Sim.Rudder".into(),
        (0x02, 0xBB) => "Sim.Throttle".into(),
        (0x02, 0xC4) => "Sim.Accelerator".into(),
        (0x02, 0xC5) => "Sim.Brake".into(),
        (0x09, b) => format!("Btn{b}"),
        (p, u) => format!("{p:#x}/{u:#x}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Verbatim VKB Gladiator EVO R descriptor (`hid_axes --hex`, 2026-09-09):
    // X Y Rz Z Rx Ry, two usage-0 16-bit fields, 128 buttons, one hat.
    const VKB_EVO_R: &str = "05010904a101050185010501093075109501150026ff0f46ff0f81020501093175109501150026ff0f46ff0f81020501093575109501150026ff0746ff0781020501093275109501150026ff0746ff0781020501093375109501150026ff0346ff0381020501093475109501150026ff0346ff0381020500090075109501150026ff0746ff0781020500090075109501150026ff0746ff078102050919012a8000150025017501968000810205010939150026070035004668016514550175049501814209006500550075049503810105010900751095018101050109007510950181010501090075109501810105010900750895178101850b050109007508953f8101850c050109007508953f81018508050109007508953f8101150026ff0046ff0085587508953f090091028559750895800900b102c0";

    // Verbatim Keychron K2 HE interface-0 descriptor (Device List,
    // 2026-09-20): keyboard, mouse (X Y Wheel + AC Pan), a Joystick
    // collection with a nested physical one (X Y Z Rx Ry Rz, 16 buttons),
    // system control, consumer, a second keyboard.
    const K2_HE_IF0: &str = "05010906a1018501050719e029e7150025019508750181029501750881010507190029ff150026ff0095067508810005081901290515002501950575019102950175039101c005010902a10185020901a100050919012908150025019508750181020501093009311581257f95027508810609381581257f950175088106050c0a38021581257f950175088106c0c005010904a1018507a10005010930093109320933093409351581257f95067508810205091901291015002501951075018102c0c005010980a101850319012ab700150126b700950175108100c0050c0901a101850419012aa002150126a002950175108100c005010906a1018506050719e029e7150025019508750181020507190029ef1500250195f075018102050819012905950575019102950175039101c0";

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn only_the_joystick_collection_counts() {
        // The mouse's X/Y (and the keyboard arrays) are other collections:
        // without the split this was "duplicate x axis".
        let usages = axis_usages(&hex(K2_HE_IF0));
        assert_eq!(usages, [(1, 0x30), (1, 0x31), (1, 0x32), (1, 0x33), (1, 0x34), (1, 0x35)]);
        assert_eq!(sc_axes_from(&hex(K2_HE_IF0), 6).unwrap(), ["x", "y", "z", "rotx", "roty", "rotz"]);
        // A plain joystick descriptor is unchanged by the rule.
        assert_eq!(axis_usages(&hex(VKB_EVO_R)).len(), axis_usages(&hex(VKB_EVO_R)).len());
        assert!(sc_axes_from(&hex(VKB_EVO_R), 6).is_ok());
        // No joystick-class collection at all: everything counts, as before.
        let mouse: Vec<u8> = vec![
            0x05, 0x01, 0x09, 0x02, 0xA1, 0x01, 0x09, 0x30, 0x09, 0x31, 0x75, 0x08, 0x95, 0x02, 0x81, 0x02, 0xC0,
        ];
        assert_eq!(axis_usages(&mouse), [(1, 0x30), (1, 0x31)]);
    }

    #[test]
    fn vkb_report_order_maps_to_canonical_sdl_order() {
        let usages = axis_usages(&hex(VKB_EVO_R));
        let gd: Vec<u16> = usages.iter().filter(|(p, _)| *p == 1).map(|(_, u)| *u).collect();
        // Report order: X Y Rz Z Rx Ry ... hat(0x39).
        assert_eq!(&gd[..6], &[0x30, 0x31, 0x35, 0x32, 0x33, 0x34]);
        // Measured on the device: X=0 Y=1 Z=2 Rz=5.
        let names = sc_axis_names(&usages).unwrap();
        assert_eq!(names, ["x", "y", "z", "rotx", "roty", "rotz"]);
    }

    #[test]
    fn sliders_follow_in_report_order_and_gaps_are_kept() {
        // X, Y, Rz, Slider, Slider -> SDL 0..4 = x y rotz slider1 slider2
        let u = [(1, 0x30), (1, 0x31), (1, 0x35), (1, 0x36), (1, 0x36)];
        assert_eq!(sc_axis_names(&u).unwrap(), ["x", "y", "rotz", "slider1", "slider2"]);
        // Slider then Dial is fine, Dial then Slider is not.
        assert_eq!(sc_axis_names(&[(1, 0x36), (1, 0x37)]).unwrap(), ["slider1", "slider2"]);
        assert!(sc_axis_names(&[(1, 0x37), (1, 0x36)]).is_err());
        assert!(sc_axis_names(&[(1, 0x36), (1, 0x36), (1, 0x36)]).is_err());
        assert!(sc_axis_names(&[(1, 0x30), (1, 0x30)]).is_err());
        // Non-axis usages are ignored here (the count check catches them).
        assert_eq!(sc_axis_names(&[(9, 1), (1, 0x39), (2, 0xBB)]).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn parses_ranges_arrays_padding_and_long_items() {
        // Usage Page GD; Usage Min X; Usage Max Y; Report Size 8; Count 2;
        // Input (data) -> X Y. Then Report Size 1, Count 8, Input (constant)
        // padding -> ignored. Then a long item, then Usage Z; Count 3; Input
        // -> Z Z Z (array repeats). Then Usage Page Button; Usage Min 1; Max
        // 4; Size 1; Count 4; Input -> 1-bit, ignored.
        let desc: Vec<u8> = vec![
            0x05, 0x01, 0x19, 0x30, 0x29, 0x31, 0x75, 0x08, 0x95, 0x02, 0x81, 0x02, //
            0x75, 0x01, 0x95, 0x08, 0x81, 0x01, //
            0xFE, 0x02, 0x00, 0xAA, 0xBB, //
            0x09, 0x32, 0x75, 0x08, 0x95, 0x03, 0x81, 0x02, //
            0x05, 0x09, 0x19, 0x01, 0x29, 0x04, 0x75, 0x01, 0x95, 0x04, 0x81, 0x02,
        ];
        assert_eq!(axis_usages(&desc), [(1, 0x30), (1, 0x31), (1, 0x32), (1, 0x32), (1, 0x32)]);
        // A 4-byte usage carries its page.
        let ext: Vec<u8> = vec![0x0B, 0x33, 0x00, 0x01, 0x00, 0x75, 0x10, 0x95, 0x01, 0x81, 0x02];
        assert_eq!(axis_usages(&ext), [(1, 0x33)]);
    }
}
