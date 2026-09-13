//! Enumerate game controllers the way Star Citizen sees them under Wine —
//! the replication of Wine's DirectInput `EnumDevices` order in
//! `bindsight_lib::wineorder` — and print Wine's view of every device plus
//! the ranking. Linux only; run headless with:
//!
//!     cargo run --example wine_order
//!
//! Compare the ranking with `Game.log`'s `Connected joystickN:` lines after
//! a game start: rank and GUIDs must agree. A `[gamepad]` device is what SC
//! logs as `xinput` and takes no slot.

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("wine_order: the Wine replication exists on Linux only");
}

#[cfg(target_os = "linux")]
fn main() {
    let sdl = match bindsight_lib::input::enumerate() {
        Ok(devices) => devices,
        Err(e) => {
            eprintln!("wine_order: {e}");
            std::process::exit(1);
        }
    };
    let devices = match bindsight_lib::wineorder::wine_devices(&sdl) {
        Ok(devices) => devices,
        Err(e) => {
            eprintln!("wine_order: {e}");
            std::process::exit(1);
        }
    };
    println!("{} device(s) as winebus exposes them\n", devices.len());
    for d in &devices {
        let gamepad = if d.is_gamepad { " [gamepad]" } else { "" };
        println!("{}{gamepad}", d.product_name);
        println!("    vid:pid:      {:04x}:{:04x}", d.vid, d.pid);
        println!("    backend:      {}", d.interface.map_or("SDL".to_string(), |mi| format!("hidraw (MI_{mi:02})")));
        println!("    version:      {} (0x{:04x})", d.version, d.version);
        println!("    serial:       {}", d.serial);
        println!("    sysfs:        {}", d.syspath);
        println!();
    }
    println!("As the game ranks them:");
    for j in bindsight_lib::wineorder::rank(&devices).joysticks {
        println!("  js{}: {} {}", j.instance, j.product_name, j.product_guid.unwrap_or_default());
    }
}
