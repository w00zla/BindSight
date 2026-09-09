//! SC's own joystick enumeration, read from `Game.log`.
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
//! The format is SC-internal and may change with a patch; parsing fails soft
//! (returns `None`) and callers report that — there is no other order source.

use std::path::Path;

use serde::Serialize;

use crate::scdata::{split_product, JoystickDevice};

/// SC's joystick enumeration from the last game start.
#[derive(Debug, Clone)]
pub struct LogEnumeration {
    /// Joysticks in SC order, `instance` being the `jsN` number.
    pub joysticks: Vec<JoystickDevice>,
    /// Raw log timestamp of the most recent `Connected joystick` line, e.g.
    /// `2026-09-08T21:06:00.762Z`.
    pub timestamp: Option<String>,
}

const MARKER: &str = "Connected joystick";

/// Parse `Game.log` text. Returns `None` if no joystick line is found. If the
/// same `joystickN` appears more than once (re-enumeration later in the
/// session), the last occurrence wins.
pub fn parse(text: &str) -> Option<LogEnumeration> {
    let mut joysticks: Vec<JoystickDevice> = Vec::new();
    let mut timestamp = None;

    for line in text.lines() {
        let Some(pos) = line.find(MARKER) else {
            continue;
        };
        // "0:  VKBsim ... {GUID}" -> index, then the Product string.
        let rest = &line[pos + MARKER.len()..];
        let Some((index, product)) = rest.split_once(':') else {
            continue;
        };
        let Ok(index) = index.trim().parse::<u32>() else {
            continue;
        };
        let (product_name, product_guid) = split_product(product);
        let device = JoystickDevice { instance: index + 1, product_name, product_guid };

        if let Some(existing) = joysticks.iter_mut().find(|d| d.instance == device.instance) {
            *existing = device;
        } else {
            joysticks.push(device);
        }
        timestamp = line_timestamp(line).map(str::to_string);
    }

    if joysticks.is_empty() {
        return None;
    }
    joysticks.sort_by_key(|d| d.instance);
    Some(LogEnumeration { joysticks, timestamp })
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
    /// The file was read but holds no `Connected joystickN` line — SC saw no
    /// joystick at its last start, or the log format changed with a patch.
    NoDeviceLines { path: String },
}

/// Read and parse a `Game.log`.
pub fn read(path: &Path) -> Result<LogEnumeration, GameLogError> {
    let display = path.display().to_string();
    let text = std::fs::read_to_string(path)
        .map_err(|e| GameLogError::NotFound { path: display.clone(), reason: e.to_string() })?;
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
    fn no_joystick_lines_is_none() {
        assert!(parse("<t> nothing here\n").is_none());
        assert!(parse("").is_none());
        // A malformed index is skipped, not a crash.
        assert!(parse("<t> - Connected joystickX: Foo {…}\n").is_none());
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
