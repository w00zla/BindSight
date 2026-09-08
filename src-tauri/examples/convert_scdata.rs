//! Offline converter: turn the raw SC files (defaultProfile.xml + global.ini)
//! into the compact `scdata.json` that ships bundled with the app, so the app
//! never parses the raw XML/INI (or needs them) at runtime. Regenerate per SC
//! patch.
//!
//!     cargo run --example convert_scdata -- <defaultProfile.xml> <global.ini> <out.json>

use bindsight_lib::scdata;

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let usage = "usage: convert_scdata <defaultProfile.xml> <global.ini> <out.json>";
    let profile_path = args.next().ok_or(usage)?;
    let loc_path = args.next().ok_or(usage)?;
    let out_path = args.next().ok_or(usage)?;

    let loc = scdata::parse_localization(&read(&loc_path)?);
    let mut maps = scdata::parse_default_profile(&read(&profile_path)?, &loc)?;

    // A label is mandatory (unlabeled actions are SC-internal binds that SC
    // itself does not surface); the description is optional. Drop actions
    // without a label, then drop actionmaps left empty.
    for map in &mut maps {
        map.actions.retain(|a| a.label.is_some());
    }
    maps.retain(|map| !map.actions.is_empty());

    let json = serde_json::to_string_pretty(&maps).map_err(|e| e.to_string())?;
    std::fs::write(&out_path, &json).map_err(|e| format!("{out_path}: {e}"))?;

    let actions: usize = maps.iter().map(|m| m.actions.len()).sum();
    println!(
        "wrote {out_path}: {} actionmaps, {actions} actions, {} KB",
        maps.len(),
        json.len() / 1024
    );
    Ok(())
}

fn read(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))
}
