//! Diagnostic: dump the HID report descriptor's input axis usages for every
//! joystick-class HID interface, in report order. Run with:
//!
//!     cargo run --example hid_axes
//!
//! This is the raw material for mapping SDL axis indices to SC axis names:
//! both SDL (evdev / DirectInput) and SC (DirectInput) derive their axis
//! order and names from these usages (see `hid.rs`). Pass `--hex` to also
//! print the raw descriptor bytes (for test fixtures).

/// Walk the descriptor and print every non-constant Input item with its
/// usages (ranges expanded, arrays collapsed).
fn dump(desc: &[u8]) {
    let mut i = 0;
    let mut usage_page: u16 = 0;
    let mut report_size: u32 = 0;
    let mut report_count: u32 = 0;
    let mut report_id: u32 = 0;
    let mut usages: Vec<(u16, u16)> = Vec::new();
    let mut usage_min: Option<(u16, u16)> = None;
    let mut bit_offset: std::collections::HashMap<u32, u32> = Default::default();

    while i < desc.len() {
        let prefix = desc[i];
        if prefix == 0xFE {
            let len = desc.get(i + 1).copied().unwrap_or(0) as usize;
            i += 3 + len;
            continue;
        }
        let size = match prefix & 0x3 { 3 => 4, s => s as usize };
        let typ = (prefix >> 2) & 0x3;
        let tag = prefix >> 4;
        let mut data: u32 = 0;
        for k in 0..size {
            data |= (desc.get(i + 1 + k).copied().unwrap_or(0) as u32) << (8 * k);
        }
        i += 1 + size;
        // Extended usages carry the page in the high 16 bits.
        let split = |d: u32| -> (u16, u16) { if size == 4 { ((d >> 16) as u16, d as u16) } else { (usage_page, d as u16) } };
        match (typ, tag) {
            (1, 0x0) => usage_page = data as u16,
            (1, 0x7) => report_size = data,
            (1, 0x9) => report_count = data,
            (1, 0x8) => report_id = data,
            (2, 0x0) => usages.push(split(data)),
            (2, 0x1) => usage_min = Some(split(data)),
            (2, 0x2) => {
                if let Some((p, lo)) = usage_min.take() {
                    let (_, hi) = split(data);
                    for u in lo..=hi { usages.push((p, u)); }
                }
            }
            (0, 0x8) | (0, 0x9) | (0, 0xB) => {
                let kind = match tag { 0x8 => "Input", 0x9 => "Output", _ => "Feature" };
                let offset = bit_offset.entry(report_id * 4 + tag as u32).or_insert(0);
                let constant = data & 1 != 0;
                if tag == 0x8 && !constant {
                    let names: Vec<String> = usages.iter().map(|&(p, u)| bindsight_lib::hid::usage_name(p, u)).collect();
                    let shown = if names.len() > 8 { format!("{}..{} ({} usages)", names[0], names[names.len() - 1], names.len()) } else { names.join(" ") };
                    println!("    {kind} id={report_id} bit {offset:>3}: {report_count} x {report_size} bit  {shown}");
                }
                *offset += report_count * report_size;
                usages.clear();
                usage_min = None;
            }
            (0, _) => { usages.clear(); usage_min = None; }
            _ => {}
        }
    }
}

fn main() -> Result<(), String> {
    let hex = std::env::args().any(|a| a == "--hex");
    let api = hidapi::HidApi::new().map_err(|e| e.to_string())?;
    for dev in api.device_list() {
        // Generic Desktop joystick (4), gamepad (5) or multi-axis (8) only.
        if dev.usage_page() != 0x01 || !matches!(dev.usage(), 4 | 5 | 8) {
            continue;
        }
        println!("{:04x}:{:04x} {:?} usage={} path={}", dev.vendor_id(), dev.product_id(), dev.product_string().unwrap_or(""), dev.usage(), dev.path().to_string_lossy());
        match dev.open_device(&api) {
            Ok(h) => {
                let mut buf = vec![0u8; 4096];
                match h.get_report_descriptor(&mut buf) {
                    Ok(n) => {
                        if hex {
                            let bytes: Vec<String> = buf[..n].iter().map(|b| format!("{b:02x}")).collect();
                            println!("    descriptor ({n} bytes): {}", bytes.join(""));
                        }
                        dump(&buf[..n]);
                        match bindsight_lib::hid::sc_axis_names(&bindsight_lib::hid::axis_usages(&buf[..n])) {
                            Ok(names) => println!("    SC axes by SDL index: {}", names.join(" ")),
                            Err(e) => println!("    SC axes: {e}"),
                        }
                    }
                    Err(e) => println!("    descriptor: {e}"),
                }
            }
            Err(e) => println!("    open: {e}"),
        }
    }
    Ok(())
}
