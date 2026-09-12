//! SC's joystick order as logged in `Game.log`.
//!
//! At startup SC logs every joystick it sees, in its own enumeration order:
//!
//! ```text
//! <2026-09-08T21:06:00.762Z> - Connected joystick0:  VKBsim Gladiator EVO  R    {0200231D-0000-0000-0000-504944564944}
//! ```
//!
//! `joystickN` is SC's `js(N+1)`, and the rest of the line is the same
//! `Product` string SC writes into `actionmaps.xml`'s `<options>` (name plus
//! `{GUID}`, spacing quirks included). It reflects the last game start only:
//! a device plugged in afterwards is not in it, and a running game keeps this
//! order until it restarts (see `order.rs` for what that is used for).
//!
//! Gamepad lines (`Connected xinput0: Gamepad`) are not read: pads take no
//! slot. The format is SC-internal and may change with a patch; parsing
//! fails soft and the caller reports why.

use std::path::Path;

use crate::order::DeviceOrder;
use crate::scdata::{split_product, JoystickDevice};

const MARKER: &str = "Connected joystick";

/// The `N` and the rest of a `Connected joystickN: <rest>` line, or `None`
/// if the line does not carry the marker or has no numeric index.
fn device_line(line: &str) -> Option<(u32, &str)> {
    let pos = line.find(MARKER)?;
    let (index, rest) = line[pos + MARKER.len()..].split_once(':')?;
    Some((index.trim().parse().ok()?, rest))
}

/// Parse `Game.log` text. Returns `None` if no joystick line is found. If the
/// same `joystickN` appears more than once (re-enumeration later in the
/// session), the last occurrence wins.
pub fn parse(text: &str) -> Option<DeviceOrder> {
    let mut joysticks: Vec<JoystickDevice> = Vec::new();
    let mut timestamp = None;

    for line in text.lines() {
        // "0:  VKBsim ... {GUID}" -> index, then the Product string.
        let Some((index, product)) = device_line(line) else { continue };
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
    Some(DeviceOrder { joysticks, timestamp })
}

/// The `<...>` timestamp a log line starts with, if any.
fn line_timestamp(line: &str) -> Option<&str> {
    let inner = line.strip_prefix('<')?;
    let end = inner.find('>')?;
    Some(&inner[..end])
}

/// Read and parse a `Game.log`. The error names the file and says what is
/// wrong with it (missing, unreadable, or without a joystick line).
pub fn read(path: &Path) -> Result<DeviceOrder, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    // Not valid UTF-8 on Windows: SC writes OS strings (e.g. audio device
    // names) in the ANSI code page. The device lines are ASCII, so lossy
    // decoding only garbles lines we do not read.
    let text = String::from_utf8_lossy(&bytes);
    parse(&text).ok_or_else(|| format!("{}: no joystick lines", path.display()))
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
    fn instance_for_guid_follows_the_log_order() {
        let log = parse(LOG).unwrap();
        assert_eq!(log.instance_for_guid("{0200231d-0000-0000-0000-504944564944}"), Some(1));
        assert_eq!(log.instance_for_guid("{0201231D-0000-0000-0000-504944564944}"), Some(2));
        assert_eq!(log.instance_for_guid("{0E213434-0000-0000-0000-504944564944}"), None);
    }

    #[test]
    fn xinput_lines_are_not_part_of_the_order() {
        // Verbatim shape from the user's Game.log; no GUID, and the pad line
        // comes well after the joystick lines.
        let log = format!("{LOG}<2026-09-08T23:52:34.891Z> - Connected xinput0: Gamepad\n");
        let e = parse(&log).unwrap();
        assert_eq!(e.joysticks.len(), 2);
        // The pad line never becomes the order's timestamp.
        assert_eq!(e.timestamp.as_deref(), Some("2026-09-08T21:06:00.789Z"));
        // A pad alone is no joystick order.
        assert!(parse("<t> - Connected xinput0: Gamepad\n").is_none());
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
    fn no_joystick_lines_at_all_is_none() {
        assert!(parse("<t> nothing here\n").is_none());
        assert!(parse("").is_none());
        // A malformed index is skipped, not a crash.
        assert!(parse("<t> - Connected joystickX: Foo {…}\n").is_none());
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
    fn read_errors_name_the_file() {
        let err = read(Path::new("/definitely/not/here/Game.log")).unwrap_err();
        assert!(err.contains("Game.log"), "{err}");

        let path = std::env::temp_dir().join(format!("bindsight-gamelog-{}.log", uuid::Uuid::new_v4()));
        std::fs::write(&path, "<t> - Connected xinput0: Gamepad\n").unwrap();
        let err = read(&path).unwrap_err();
        std::fs::remove_file(&path).unwrap();
        assert!(err.ends_with("no joystick lines"), "{err}");
    }
}
