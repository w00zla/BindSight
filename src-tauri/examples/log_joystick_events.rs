//! Step 2 of the input layer: log live joystick events (buttons, axes, hats)
//! via the SDL raw joystick API. Run headless with:
//!
//!     cargo run --example log_joystick_events
//!
//! Press buttons / move axes / work the hat and watch which SDL index fires.
//! This is what settles the open question from CONCEPT.md: whether SDL's
//! button order under Wine lines up with SC's `js_buttonN` numbering. Ctrl+C
//! to quit.

use std::collections::HashMap;

use sdl2::event::Event;

/// Only print an axis event once it has moved more than this since the last
/// printed value, so continuous jitter does not drown out the discrete events.
const AXIS_PRINT_THRESHOLD: i32 = 3000;

fn main() -> Result<(), String> {
    let sdl = sdl2::init()?;
    let joystick = sdl.joystick()?;

    // Joysticks must stay open to receive their events; keep them alive and
    // remember instance-id -> name for readable output.
    let count = joystick.num_joysticks()?;
    let mut open_sticks = Vec::new();
    let mut names: HashMap<u32, String> = HashMap::new();
    for index in 0..count {
        if let Ok(stick) = joystick.open(index) {
            names.insert(stick.instance_id(), stick.name());
            open_sticks.push(stick);
        }
    }
    println!("Listening on {} joystick(s). Press inputs, Ctrl+C to quit.\n", open_sticks.len());

    let name_of = |which: u32| -> String {
        names.get(&which).cloned().unwrap_or_else(|| format!("id {which}"))
    };

    let mut last_axis: HashMap<(u32, u8), i16> = HashMap::new();
    let mut event_pump = sdl.event_pump()?;

    for event in event_pump.wait_iter() {
        match event {
            Event::JoyButtonDown { which, button_idx, .. } => {
                println!("{:40} button {button_idx:>3} down", name_of(which));
            }
            Event::JoyButtonUp { which, button_idx, .. } => {
                println!("{:40} button {button_idx:>3} up", name_of(which));
            }
            Event::JoyHatMotion { which, hat_idx, state, .. } => {
                println!("{:40} hat {hat_idx:>2} -> {state:?}", name_of(which));
            }
            Event::JoyAxisMotion { which, axis_idx, value, .. } => {
                let prev = last_axis.get(&(which, axis_idx)).copied().unwrap_or(0);
                if (value as i32 - prev as i32).abs() > AXIS_PRINT_THRESHOLD {
                    last_axis.insert((which, axis_idx), value);
                    println!("{:40} axis {axis_idx:>2} = {value:>6}", name_of(which));
                }
            }
            Event::Quit { .. } => break,
            _ => {}
        }
    }

    Ok(())
}
