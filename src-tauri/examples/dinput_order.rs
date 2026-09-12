//! Enumerate game controllers the way Star Citizen does — DirectInput 8
//! `EnumDevices(DI8DEVCLASS_GAMECTRL, DIEDFL_ATTACHEDONLY)` — and print them
//! in that order, with the `guidProduct` SC writes as `options/@Product`.
//! Windows only; run headless with:
//!
//!     cargo run --example dinput_order
//!
//! Thin wrapper around `bindsight_lib::dinput::list`, the app's order source
//! on Windows. Compare the output with `Game.log`'s `Connected joystickN:`
//! lines after a game start: rank and GUIDs must agree. An `[XInput]` device
//! (path carrying `ig_`) is what SC logs as `xinput` and takes no slot.

#[cfg(not(windows))]
fn main() {
    eprintln!("dinput_order: DirectInput exists on Windows only");
}

#[cfg(windows)]
fn main() {
    let devices = match bindsight_lib::dinput::list() {
        Ok(devices) => devices,
        Err(e) => {
            eprintln!("dinput_order: {e}");
            std::process::exit(1);
        }
    };
    println!("{} DirectInput game controller(s), in EnumDevices order\n", devices.len());
    for d in &devices {
        let xinput = if d.is_xinput() { " [XInput]" } else { "" };
        println!("{}{xinput}", d.product_name);
        println!("    guidProduct:  {}", d.product_guid);
        println!("    guidInstance: {}", d.instance_guid);
        println!("    instance:     {}", d.instance_name);
        println!(
            "    devType:      0x{:08x} (type {}, subtype {})",
            d.dev_type,
            d.dev_type & 0xff,
            (d.dev_type >> 8) & 0xff
        );
        println!("    HID usage:    page 0x{:02x} usage 0x{:02x}", d.usage_page, d.usage);
        println!("    path:         {}", if d.path.is_empty() { "<unavailable>" } else { &d.path });
        println!();
    }
    println!("As the game ranks them:");
    for j in bindsight_lib::dinput::to_order(&devices).joysticks {
        println!("  js{}: {} {}", j.instance, j.product_name, j.product_guid.unwrap_or_default());
    }
}
