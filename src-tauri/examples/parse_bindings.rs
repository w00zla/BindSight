//! Verification for slice 3: join the action master list (defaultProfile.xml +
//! global.ini) with the user's rebinds (actionmaps.xml) and print every action
//! that has a real joystick binding, resolved to a label and a device.
//!
//!     cargo run --example parse_bindings -- <defaultProfile.xml> <global.ini> <actionmaps.xml>

use std::collections::HashMap;

use bindsight_lib::scdata;

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let profile_path = args.next().ok_or("usage: parse_bindings <defaultProfile.xml> <global.ini> <actionmaps.xml>")?;
    let loc_path = args.next().ok_or("usage: parse_bindings <defaultProfile.xml> <global.ini> <actionmaps.xml>")?;
    let user_path = args.next().ok_or("usage: parse_bindings <defaultProfile.xml> <global.ini> <actionmaps.xml>")?;

    let loc = scdata::parse_localization(&read(&loc_path)?);
    let maps = scdata::parse_default_profile(&read(&profile_path)?, &loc)?;
    let user = scdata::parse_user_profile(&read(&user_path)?)?;

    // (actionmap, action) -> label (or the action name if unlabeled)
    let mut label_of: HashMap<(&str, &str), &str> = HashMap::new();
    for map in &maps {
        for action in &map.actions {
            label_of.insert(
                (map.name.as_str(), action.name.as_str()),
                action.label.as_deref().unwrap_or(action.name.as_str()),
            );
        }
    }

    // instance -> device name from the <options> block
    let device_of: HashMap<u32, &str> = user
        .joysticks
        .iter()
        .map(|d| (d.instance, d.product_name.as_str()))
        .collect();

    println!(
        "joystick devices: {}   |   rebinds total: {}\n",
        user.joysticks.len(),
        user.rebinds.len()
    );

    let mut bound = 0;
    for rebind in &user.rebinds {
        let Some((instance, token)) = scdata::parse_js_binding(&rebind.input) else {
            continue; // not a bound joystick input
        };
        bound += 1;
        let label = label_of
            .get(&(rebind.actionmap.as_str(), rebind.action.as_str()))
            .copied()
            .unwrap_or(rebind.action.as_str());
        let device = device_of.get(&instance).copied().unwrap_or("<unknown device>");
        println!("js{instance}_{token:<14} {label:<34} [{device}]");
    }

    println!("\n{bound} joystick binding(s)");
    Ok(())
}

fn read(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))
}
