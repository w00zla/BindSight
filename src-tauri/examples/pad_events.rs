//! Gamepads through SDL's GameController API, the way the app reads them:
//! print each pad's mapping string, then every controller-level axis and
//! button event with SDL's names. Run headless with:
//!
//!     cargo run --example pad_events
//!
//! Work the triggers and sticks; Ctrl+C to quit. Tells whether a trigger
//! reaches the controller layer as `TriggerLeft` 0..32767 (what the derived
//! `triggerl_btn` needs) or lands on another axis / range because of the
//! mapping SDL picked for the device.

use sdl2::event::Event;

fn main() -> Result<(), String> {
    let sdl = bindsight_lib::input::init_sdl()?;
    let joystick = sdl.joystick()?;
    let controllers = sdl.game_controller()?;
    let mut event_pump = sdl.event_pump()?;

    let count = joystick.num_joysticks()?;
    let mut pads = Vec::new();
    for index in 0..count {
        if !controllers.is_game_controller(index) {
            continue;
        }
        let pad = controllers.open(index).map_err(|e| e.to_string())?;
        println!("[{index}] {}", pad.name());
        println!("    mapping: {}", pad.mapping());
        pads.push(pad);
    }
    if pads.is_empty() {
        println!("no gamepad found");
        return Ok(());
    }
    println!("Listening on {} pad(s). Work the triggers, Ctrl+C to quit.\n", pads.len());

    for event in event_pump.wait_iter() {
        match event {
            Event::ControllerAxisMotion { timestamp, which, axis, value } => {
                println!("t{timestamp} pad {which} axis {axis:?} = {value}");
            }
            Event::ControllerButtonDown { timestamp, which, button } => {
                println!("t{timestamp} pad {which} button {button:?} down");
            }
            Event::ControllerButtonUp { timestamp, which, button } => {
                println!("t{timestamp} pad {which} button {button:?} up");
            }
            Event::Quit { .. } => break,
            _ => {}
        }
    }
    Ok(())
}
