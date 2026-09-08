//! Verification for the SC data parser: read the real defaultProfile.xml and
//! global.ini, parse them, and print a summary plus a sample. Run with:
//!
//!     cargo run --example parse_scdata -- <defaultProfile.xml> <global.ini>

use bindsight_lib::scdata;

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let profile_path = args.next().ok_or("usage: parse_scdata <defaultProfile.xml> <global.ini>")?;
    let loc_path = args.next().ok_or("usage: parse_scdata <defaultProfile.xml> <global.ini>")?;

    let profile_xml = std::fs::read_to_string(&profile_path).map_err(|e| e.to_string())?;
    let loc_text = std::fs::read_to_string(&loc_path).map_err(|e| e.to_string())?;

    let loc = scdata::parse_localization(&loc_text);
    let maps = scdata::parse_default_profile(&profile_xml, &loc)?;

    let total_actions: usize = maps.iter().map(|m| m.actions.len()).sum();
    let unresolved = maps
        .iter()
        .flat_map(|m| &m.actions)
        .filter(|a| a.label.is_none())
        .count();
    println!("localization entries: {}", loc.len());
    println!("actionmaps: {}", maps.len());
    println!("actions: {total_actions} ({unresolved} without a resolved label)\n");

    for map in maps.iter().take(2) {
        println!("== {} [{}]", map.label.as_deref().unwrap_or(&map.name), map.name);
        for action in map.actions.iter().take(5) {
            println!(
                "   {:<32} {:<28} joy_default={}",
                action.name,
                action.label.as_deref().unwrap_or("<no label>"),
                action.joystick_default.as_deref().unwrap_or("-"),
            );
        }
    }

    Ok(())
}
