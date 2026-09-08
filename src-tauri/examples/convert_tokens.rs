//! Offline converter: turn keybinding_localization.xml + global.ini into the
//! bundled `tokens.json` — display labels for joystick input tokens (button9 ->
//! "Button 9", x -> "X-Axis", …). Regenerate per SC patch.
//!
//!     cargo run --example convert_tokens -- <keybinding_localization.xml> <global.ini> <out.json>

use std::collections::BTreeMap;

use bindsight_lib::scdata;

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let usage = "usage: convert_tokens <keybinding_localization.xml> <global.ini> <out.json>";
    let kb_path = args.next().ok_or(usage)?;
    let loc_path = args.next().ok_or(usage)?;
    let out_path = args.next().ok_or(usage)?;

    let loc = scdata::parse_localization(&read(&loc_path)?);
    let tokens = scdata::parse_token_labels(&read(&kb_path)?, &loc);

    // Sort for stable, reviewable diffs.
    let sorted: BTreeMap<_, _> = tokens.into_iter().collect();
    let json = serde_json::to_string_pretty(&sorted).map_err(|e| e.to_string())?;
    std::fs::write(&out_path, &json).map_err(|e| format!("{out_path}: {e}"))?;

    println!("wrote {out_path}: {} token labels", sorted.len());
    Ok(())
}

fn read(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))
}
