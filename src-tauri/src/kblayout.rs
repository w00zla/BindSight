//! The OS keyboard layout as an xkb-style code (`de`, `us`, `gb`, …), used
//! to pick the default keyboard image-map. `None` when it cannot be told.

use log::debug;

/// The active keyboard layout, lowercase xkb name, or `None`.
#[tauri::command]
pub fn keyboard_layout() -> Option<String> {
    let layout = detect();
    debug!("keyboard layout: {layout:?}");
    layout
}

#[cfg(target_os = "linux")]
fn detect() -> Option<String> {
    // systemd-localed knows the X11 layout (a comma list, first one wins);
    // the console keymap is the fallback for boxes without it.
    if let Ok(out) = std::process::Command::new("localectl").arg("status").output() {
        let text = String::from_utf8_lossy(&out.stdout);
        if let Some(l) = parse_localectl(&text) {
            return Some(l);
        }
    }
    let vconsole = std::fs::read_to_string("/etc/vconsole.conf").ok()?;
    parse_vconsole(&vconsole)
}

#[cfg(target_os = "linux")]
fn parse_localectl(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find_map(|l| l.strip_prefix("X11 Layout:"))
        .and_then(|v| v.split(',').next())
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| !v.is_empty() && v != "n/a")
}

#[cfg(target_os = "linux")]
fn parse_vconsole(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find_map(|l| l.strip_prefix("KEYMAP="))
        .map(|v| v.trim_matches('"').to_ascii_lowercase())
        // `de-latin1-nodeadkeys` -> `de`
        .and_then(|v| v.split('-').next().map(str::to_string))
        .filter(|v| !v.is_empty())
}

#[cfg(windows)]
fn detect() -> Option<String> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayoutNameW;
    // KL_NAMELENGTH = 9: eight hex digits plus the terminator, e.g. "00000407".
    let mut buf = [0u16; 9];
    // SAFETY: the buffer has the documented KL_NAMELENGTH size.
    if unsafe { GetKeyboardLayoutNameW(buf.as_mut_ptr()) } == 0 {
        return None;
    }
    let klid = String::from_utf16_lossy(&buf).trim_end_matches('\0').to_string();
    layout_from_klid(&klid)
}

#[cfg(not(any(target_os = "linux", windows)))]
fn detect() -> Option<String> {
    None
}

/// Windows keyboard layout id (the hex KLID string) -> xkb layout name. The
/// low 16 bits are the language id; unknown ones fall back to the primary
/// language's usual layout.
#[allow(dead_code)]
fn layout_from_klid(klid: &str) -> Option<String> {
    let id = u32::from_str_radix(klid.trim(), 16).ok()?;
    let lang = id & 0xffff;
    let known = match lang {
        0x0407 => "de",
        0x0807 => "ch",
        0x0c07 => "at",
        0x0409 => "us",
        0x0809 => "gb",
        0x0c09 => "au",
        0x1009 => "ca",
        0x040c => "fr",
        0x080c => "be",
        0x0c0c => "ca",
        0x100c => "ch",
        0x0410 => "it",
        0x040a | 0x0c0a => "es",
        0x0413 => "nl",
        0x0415 => "pl",
        0x0419 => "ru",
        0x041d => "se",
        0x0414 => "no",
        0x0406 => "dk",
        0x040b => "fi",
        0x0416 => "br",
        0x0816 => "pt",
        0x040e => "hu",
        0x0405 => "cz",
        0x041b => "sk",
        0x0424 => "si",
        0x041a => "hr",
        0x0408 => "gr",
        0x041f => "tr",
        0x0411 => "jp",
        0x0412 => "kr",
        _ => "",
    };
    if !known.is_empty() {
        return Some(known.to_string());
    }
    let primary = match lang & 0xff {
        0x07 => "de",
        0x09 => "us",
        0x0c => "fr",
        0x0a => "es",
        0x10 => "it",
        _ => return None,
    };
    Some(primary.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn klid_maps_known_and_primary_languages() {
        assert_eq!(layout_from_klid("00000407").as_deref(), Some("de"));
        assert_eq!(layout_from_klid("00000409").as_deref(), Some("us"));
        assert_eq!(layout_from_klid("00000809").as_deref(), Some("gb"));
        assert_eq!(layout_from_klid("00010407").as_deref(), Some("de")); // German IBM variant
        assert_eq!(layout_from_klid("00001409").as_deref(), Some("us")); // en-NZ: primary English
        assert_eq!(layout_from_klid("0000042a"), None); // Vietnamese: no guess
        assert_eq!(layout_from_klid("zz"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_sources_parse() {
        let out = "   System Locale: LANG=en_US.utf8\n       VC Keymap: de\n      X11 Layout: de,us\n       X11 Model: pc105\n";
        assert_eq!(parse_localectl(out).as_deref(), Some("de"));
        assert_eq!(parse_localectl("X11 Layout: n/a\n"), None);
        assert_eq!(parse_vconsole("KEYMAP=de-latin1-nodeadkeys\nFONT=eurlatgr\n").as_deref(), Some("de"));
        assert_eq!(parse_vconsole("KEYMAP=\"us\"\n").as_deref(), Some("us"));
        assert_eq!(parse_vconsole("FONT=x\n"), None);
    }
}
