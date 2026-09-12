//! SC's own device enumeration, read from `Game.log`.
//!
//! At startup SC logs every joystick it sees, in its own enumeration order:
//!
//! ```text
//! <2026-09-08T21:06:00.762Z> - Connected joystick0:  VKBsim Gladiator EVO  R    {0200231D-0000-0000-0000-504944564944}
//! ```
//!
//! `joystickN` is SC's `js(N+1)`, and the rest of the line is the same
//! `Product` string SC writes into `actionmaps.xml`'s `<options>` (name plus
//! `{GUID}`, spacing quirks included). This is the ground truth for which
//! devices SC sees and in which order — always written by the SC that ran on
//! *this* platform, unlike an imported `actionmaps.xml`. It only reflects the
//! last game start, so a device plugged in afterwards is not in it.
//!
//! Gamepads are logged separately and without a GUID:
//!
//! ```text
//! <2026-09-08T23:52:34.891Z> - Connected xinput0: Gamepad
//! ```
//!
//! SC binds exactly one pad (`gp1`), so these lines only answer "does SC see a
//! pad at all" — they carry no order and no slot. The keyboard is never logged
//! and always counts as seen.
//!
//! The format is SC-internal and may change with a patch; parsing fails soft
//! (returns `None`) and callers report that — there is no other order source.

use std::path::Path;

use serde::Serialize;

use crate::scdata::{split_product, JoystickDevice};

/// SC's device enumeration from the last game start.
#[derive(Debug, Clone)]
pub struct LogEnumeration {
    /// Joysticks in SC order, `instance` being the `jsN` number.
    pub joysticks: Vec<JoystickDevice>,
    /// Gamepad names in log order (`Connected xinput0: Gamepad`). SC binds
    /// only the first (`gp1`); the list exists to answer "did SC see a pad".
    pub gamepads: Vec<String>,
    /// Raw log timestamp of the most recent `Connected joystick` line, e.g.
    /// `2026-09-08T21:06:00.762Z`.
    pub timestamp: Option<String>,
}

const MARKER: &str = "Connected joystick";
const PAD_MARKER: &str = "Connected xinput";

/// The `N` and the rest of a `Connected <device>N: <rest>` line, or `None` if
/// the line does not carry the marker or has no numeric index.
fn device_line<'a>(line: &'a str, marker: &str) -> Option<(u32, &'a str)> {
    let pos = line.find(marker)?;
    let (index, rest) = line[pos + marker.len()..].split_once(':')?;
    Some((index.trim().parse().ok()?, rest))
}

/// Parse `Game.log` text. Returns `None` only if neither a joystick nor an
/// xinput line is found. If the same `joystickN`/`xinputN` appears more than
/// once (re-enumeration later in the session), the last occurrence wins.
pub fn parse(text: &str) -> Option<LogEnumeration> {
    let mut joysticks: Vec<JoystickDevice> = Vec::new();
    let mut pads: Vec<(u32, String)> = Vec::new();
    let mut timestamp = None;

    for line in text.lines() {
        // "0:  VKBsim ... {GUID}" -> index, then the Product string.
        if let Some((index, product)) = device_line(line, MARKER) {
            let (product_name, product_guid) = split_product(product);
            let device = JoystickDevice { instance: index + 1, product_name, product_guid };
            if let Some(existing) = joysticks.iter_mut().find(|d| d.instance == device.instance) {
                *existing = device;
            } else {
                joysticks.push(device);
            }
            timestamp = line_timestamp(line).map(str::to_string);
        } else if let Some((index, name)) = device_line(line, PAD_MARKER) {
            let name = name.trim().to_string();
            match pads.iter_mut().find(|(i, _)| *i == index) {
                Some(existing) => existing.1 = name,
                None => pads.push((index, name)),
            }
        }
    }

    if joysticks.is_empty() && pads.is_empty() {
        return None;
    }
    joysticks.sort_by_key(|d| d.instance);
    pads.sort_by_key(|(i, _)| *i);
    Some(LogEnumeration { joysticks, gamepads: pads.into_iter().map(|(_, n)| n).collect(), timestamp })
}

/// The `<...>` timestamp a log line starts with, if any.
fn line_timestamp(line: &str) -> Option<&str> {
    let inner = line.strip_prefix('<')?;
    let end = inner.find('>')?;
    Some(&inner[..end])
}

/// Why no enumeration could be taken from `Game.log`. Carried into the clash
/// report so the GUI can say precisely what is wrong instead of silently
/// falling back.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GameLogError {
    /// The file is missing or unreadable at `path`; `reason` is the OS error.
    NotFound { path: String, reason: String },
    /// The file was read but holds neither a `Connected joystickN` nor a
    /// `Connected xinputN` line — SC saw no input device at its last start, or
    /// the log format changed with a patch.
    NoDeviceLines { path: String },
    /// The file lists a gamepad but no joystick: SC saw none at its last
    /// start, so there is no joystick order (raised by the clash analysis).
    NoJoystickLines,
}

/// Read and parse a `Game.log`.
pub fn read(path: &Path) -> Result<LogEnumeration, GameLogError> {
    let display = path.display().to_string();
    let bytes = std::fs::read(path)
        .map_err(|e| GameLogError::NotFound { path: display.clone(), reason: e.to_string() })?;
    // Not valid UTF-8 on Windows: SC writes OS strings (e.g. audio device
    // names) in the ANSI code page. The device lines are ASCII, so lossy
    // decoding only garbles lines we do not read.
    let text = String::from_utf8_lossy(&bytes);
    parse(&text).ok_or(GameLogError::NoDeviceLines { path: display })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Verbatim from the user's Linux/Wine Game.log.
    const LOG: &str = "\
<2026-09-08T21:05:59.000Z> Some unrelated line
<2026-09-08T21:06:00.762Z> - Connected joystick0:  VKBsim Gladiator EVO  R    {0200231D-0000-0000-0000-504944564944}
<2026-09-08T21:06:00.789Z> - Connected joystick1:  VKBsim Gladiator EVO  L    {0201231D-0000-0000-0000-504944564944}
<2026-09-08T21:06:03.614Z> Found 13 audio input devices'
";

    #[test]
    fn parses_real_startup_lines() {
        let e = parse(LOG).unwrap();
        assert_eq!(e.joysticks.len(), 2);
        assert_eq!(e.joysticks[0].instance, 1); // joystick0 -> js1
        assert_eq!(e.joysticks[0].product_name, "VKBsim Gladiator EVO  R");
        assert_eq!(e.joysticks[0].product_guid.as_deref(), Some("{0200231D-0000-0000-0000-504944564944}"));
        assert_eq!(e.joysticks[1].instance, 2);
        assert_eq!(e.joysticks[1].product_guid.as_deref(), Some("{0201231D-0000-0000-0000-504944564944}"));
        assert_eq!(e.timestamp.as_deref(), Some("2026-09-08T21:06:00.789Z"));
        assert!(e.gamepads.is_empty());
    }

    #[test]
    fn parses_xinput_lines_as_gamepads() {
        // Verbatim shape from the user's Game.log; no GUID, and the pad line
        // comes well after the joystick lines.
        let log = format!("{LOG}<2026-09-08T23:52:34.891Z> - Connected xinput0: Gamepad\n");
        let e = parse(&log).unwrap();
        assert_eq!(e.gamepads, vec!["Gamepad"]);
        assert_eq!(e.joysticks.len(), 2);
        // The pad line never becomes the joystick-order timestamp.
        assert_eq!(e.timestamp.as_deref(), Some("2026-09-08T21:06:00.789Z"));

        // A pad alone is still an enumeration: joysticks empty, no error.
        let e = parse("<t> - Connected xinput0: Gamepad\n<t> - Connected xinput1: Other Pad\n").unwrap();
        assert!(e.joysticks.is_empty());
        assert_eq!(e.gamepads, vec!["Gamepad", "Other Pad"]);
        assert_eq!(e.timestamp, None);

        // Re-enumeration of the same slot: the last line wins.
        let e = parse("<t1> - Connected xinput0: Old Pad\n<t2> - Connected xinput0: New Pad\n").unwrap();
        assert_eq!(e.gamepads, vec!["New Pad"]);
    }

    #[test]
    fn later_enumeration_of_same_slot_wins() {
        let log = "\
<t1> - Connected joystick0: Old Stick {AAAA0000-0000-0000-0000-504944564944}
<t2> - Connected joystick0: New Stick {BBBB0000-0000-0000-0000-504944564944}
";
        let e = parse(log).unwrap();
        assert_eq!(e.joysticks.len(), 1);
        assert_eq!(e.joysticks[0].product_name, "New Stick");
        assert_eq!(e.timestamp.as_deref(), Some("t2"));
    }

    #[test]
    fn no_device_lines_at_all_is_none() {
        assert!(parse("<t> nothing here\n").is_none());
        assert!(parse("").is_none());
        // A malformed index is skipped, not a crash.
        assert!(parse("<t> - Connected joystickX: Foo {…}\n").is_none());
        assert!(parse("<t> - Connected xinputX: Gamepad\n").is_none());
    }

    #[test]
    fn read_tolerates_non_utf8_bytes() {
        // Windows SC logs device names in the ANSI code page: `ö` is the single
        // byte 0xF6 (verbatim from a real Windows Game.log).
        let mut bytes = b"<t> FriendlyName='Kopfh".to_vec();
        bytes.push(0xF6);
        bytes.extend_from_slice(b"rermikrofon (A50 Mic)'\n");
        bytes.extend_from_slice(LOG.as_bytes());
        let path = std::env::temp_dir().join(format!("bindsight-gamelog-{}.log", uuid::Uuid::new_v4()));
        std::fs::write(&path, &bytes).unwrap();
        let result = read(&path);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(result.unwrap().joysticks.len(), 2);
    }

    #[test]
    fn read_reports_a_missing_file_with_its_path() {
        let err = read(Path::new("/definitely/not/here/Game.log")).unwrap_err();
        match err {
            GameLogError::NotFound { path, reason } => {
                assert!(path.ends_with("Game.log"));
                assert!(!reason.is_empty());
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
    }
}
