//! Compare the bindings of two sources — the live `actionmaps.xml`, an
//! exported binding profile, or a backup — by SC token (`js1_button5`,
//! `js2_rotz`, `kb1_lalt+x`, `gp1_a`, …). For each token, the set of actions
//! bound to it (identified by `(actionmap, action)`, a label difference alone
//! never counts) is compared between the two sides; only tokens that differ
//! are reported. Rows are grouped joystick first, then keyboard, then gamepad.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::bindings::{self, ResolvedBinding};
use crate::scdata::DeviceKind;
use crate::{backups, binding_profiles, config, scdata, AppData};

/// One action a token is bound to, for display in a diff row. Equality for
/// diffing purposes is `(actionmap, action)` only — see [`action_keys`]; the
/// label is carried along for the UI and never causes a row by itself.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct ActionRef {
    pub actionmap: String,
    pub action: String,
    pub label: Option<String>,
}

/// Why a token's row is reported.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DiffKind {
    /// Bound in A only.
    Added,
    /// Bound in B only.
    Removed,
    /// Bound on both sides, with different action sets.
    Changed,
}

/// One SC token whose bound actions differ between A and B.
#[derive(Debug, Clone, Serialize)]
pub struct DiffRow {
    pub token: String,
    /// The `N` in `jsN_...`, `1` for a `kb1_`/`gp1_` token, or `None` for a
    /// token with no recognisable device prefix.
    pub instance: Option<u32>,
    /// Which device the token belongs to (joystick for an unrecognised one).
    pub device_kind: DeviceKind,
    pub kind: DiffKind,
    /// Actions bound to this token in A, sorted by `(actionmap, action)`.
    pub a: Vec<ActionRef>,
    /// Actions bound to this token in B, sorted by `(actionmap, action)`.
    pub b: Vec<ActionRef>,
}

/// Result of comparing two binding sets.
#[derive(Debug, Clone, Serialize, Default)]
pub struct DiffReport {
    /// Sorted by token: joystick rows first, then keyboard, then gamepad;
    /// within a device by instance ascending, then by input kind/number
    /// (numeric-aware, so `button2` sorts before `button10`).
    pub rows: Vec<DiffRow>,
    pub added: usize,
    pub removed: usize,
    pub changed: usize,
}

/// One side of a comparison, as chosen in the Tools UI.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Source {
    /// The live, currently loaded `actionmaps.xml`.
    Current,
    /// An exported binding profile in the binding profiles folder; `file` is
    /// a bare `.xml` name.
    Profile { file: String },
    /// A backup by id (see `backups.rs`).
    Backup { id: String },
}

/// `(actionmap, action)` keys of a sorted, deduplicated action list — the
/// equality check that ignores label differences.
fn action_keys(v: &[ActionRef]) -> Vec<(&str, &str)> {
    v.iter().map(|r| (r.actionmap.as_str(), r.action.as_str())).collect()
}

/// Flatten resolved bindings into token -> deduplicated, sorted action list.
fn group(bindings: &[ResolvedBinding]) -> BTreeMap<String, Vec<ActionRef>> {
    let mut map: BTreeMap<String, Vec<ActionRef>> = BTreeMap::new();
    for b in bindings {
        let list = map.entry(b.token.clone()).or_default();
        if !list.iter().any(|r| r.actionmap == b.actionmap && r.action == b.action) {
            list.push(ActionRef { actionmap: b.actionmap.clone(), action: b.action.clone(), label: b.label.clone() });
        }
    }
    for list in map.values_mut() {
        list.sort_by(|x, y| (x.actionmap.as_str(), x.action.as_str()).cmp(&(y.actionmap.as_str(), y.action.as_str())));
    }
    map
}

/// Split a full SC token into its device, instance and the rest: `js2_rotz` ->
/// `(Joystick, 2, "rotz")`, `kb1_lalt+x` -> `(Keyboard, 1, "lalt+x")`. `None`
/// for a token with no recognisable device prefix.
fn split_token(token: &str) -> Option<(DeviceKind, u32, &str)> {
    for kind in [DeviceKind::Joystick, DeviceKind::Keyboard, DeviceKind::Gamepad] {
        let Some(rest) = token.strip_prefix(kind.token_prefix()) else { continue };
        let Some((num, rest)) = rest.split_once('_') else { continue };
        if let Ok(instance) = num.parse::<u32>() {
            return Some((kind, instance, rest));
        }
    }
    None
}

/// The `N` in a `jsN_`/`kb1_`/`gp1_` token, or `None` if it has no device
/// prefix.
fn instance_of(token: &str) -> Option<u32> {
    split_token(token).map(|(_, instance, _)| instance)
}

/// The device a token belongs to; an unrecognised token counts as a joystick
/// one so it keeps sorting with the joystick rows as it always did.
fn kind_of(token: &str) -> DeviceKind {
    split_token(token).map_or(DeviceKind::Joystick, |(kind, _, _)| kind)
}

/// The part of a token after its device prefix, or the whole token if it has
/// none.
fn rest_of(token: &str) -> &str {
    split_token(token).map_or(token, |(_, _, rest)| rest)
}

/// Numeric-aware string compare: alternating digit/non-digit runs, digit runs
/// compared by value, so `"button2"` sorts before `"button10"`.
fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut a = a.chars().peekable();
    let mut b = b.chars().peekable();
    loop {
        return match (a.peek(), b.peek()) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Less,
            (Some(_), None) => Ordering::Greater,
            (Some(ca), Some(cb)) if ca.is_ascii_digit() && cb.is_ascii_digit() => {
                let na = take_digits(&mut a);
                let nb = take_digits(&mut b);
                match na.cmp(&nb) {
                    Ordering::Equal => continue,
                    ord => ord,
                }
            }
            _ => {
                let ca = a.next().unwrap();
                let cb = b.next().unwrap();
                match ca.cmp(&cb) {
                    Ordering::Equal => continue,
                    ord => ord,
                }
            }
        };
    }
}

fn take_digits(it: &mut std::iter::Peekable<std::str::Chars>) -> u64 {
    let mut s = String::new();
    while let Some(&c) = it.peek() {
        if !c.is_ascii_digit() {
            break;
        }
        s.push(c);
        it.next();
    }
    s.parse().unwrap_or(0)
}

/// Row order: joystick rows first, then keyboard, then gamepad; within a
/// device by instance ascending (tokens without one sort last), then
/// numeric-aware by the rest of the token.
fn compare_tokens(a: &str, b: &str) -> Ordering {
    kind_of(a)
        .cmp(&kind_of(b))
        .then_with(|| match (instance_of(a), instance_of(b)) {
            (Some(x), Some(y)) if x != y => x.cmp(&y),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            _ => natural_cmp(rest_of(a), rest_of(b)),
        })
}

/// Compare two resolved binding sets by token. Only tokens whose action sets
/// differ are reported (see the module doc for the exact semantics).
pub fn diff_bindings(a: &[ResolvedBinding], b: &[ResolvedBinding]) -> DiffReport {
    let map_a = group(a);
    let map_b = group(b);

    let mut tokens: Vec<&String> = map_a.keys().chain(map_b.keys()).collect();
    tokens.sort();
    tokens.dedup();

    let mut rows = Vec::new();
    let mut added = 0usize;
    let mut removed = 0usize;
    let mut changed = 0usize;

    for token in tokens {
        let a_actions = map_a.get(token).cloned().unwrap_or_default();
        let b_actions = map_b.get(token).cloned().unwrap_or_default();

        if action_keys(&a_actions) == action_keys(&b_actions) {
            continue;
        }

        let kind = if b_actions.is_empty() {
            added += 1;
            DiffKind::Added
        } else if a_actions.is_empty() {
            removed += 1;
            DiffKind::Removed
        } else {
            changed += 1;
            DiffKind::Changed
        };

        rows.push(DiffRow {
            token: token.clone(),
            instance: instance_of(token),
            device_kind: kind_of(token),
            kind,
            a: a_actions,
            b: b_actions,
        });
    }

    rows.sort_by(|x, y| compare_tokens(&x.token, &y.token));

    DiffReport { rows, added, removed, changed }
}

/// Load and resolve one comparison side. `current` is the already-resolved
/// live profile (empty when none is loaded — that is not an error, so
/// `Source::Current` never fails); `Profile`/`Backup` read and parse their
/// XML file, `Profile` from `<binding_profiles_dir>/<file>`, `Backup` via
/// `backups::path_of`. Kept free of `AppData`/`AppHandle` so it is testable
/// standalone.
fn load_source(
    source: &Source,
    current: &[ResolvedBinding],
    base_path: &str,
    backups_root: &Path,
    actions: &[scdata::ActionMap],
) -> Result<Vec<ResolvedBinding>, String> {
    match source {
        Source::Current => Ok(current.to_vec()),
        Source::Profile { file } => {
            if !binding_profiles::is_bare_xml_name(file) {
                return Err(format!("{file:?} is not a valid binding profile file name"));
            }
            let path = config::binding_profiles_dir(base_path).join(file);
            let xml = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let profile = scdata::parse_user_profile(&xml)?;
            Ok(bindings::resolve_bindings(actions, &profile))
        }
        Source::Backup { id } => {
            let path = backups::path_of(backups_root, id)?;
            let xml = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let profile = scdata::parse_user_profile(&xml)?;
            Ok(bindings::resolve_bindings(actions, &profile))
        }
    }
}

// ---------------------------------------------------------------------------
// Tauri commands
//
// Crate-visible: `generate_handler!` in lib.rs is the only caller.
// ---------------------------------------------------------------------------

#[tauri::command]
pub(crate) fn compare_bindings(
    a: Source,
    b: Source,
    app: AppHandle,
    data: State<Mutex<AppData>>,
) -> Result<DiffReport, String> {
    let data = data.lock().unwrap();
    let current = crate::current_bindings(&data);
    let base_path = data.config.base_path();
    let backups_root = backups::backups_root(&app)?;
    let actions = &data.sc.data.actions;

    let a = load_source(&a, &current, base_path, &backups_root, actions)?;
    let b = load_source(&b, &current, base_path, &backups_root, actions)?;
    Ok(diff_bindings(&a, &b))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A resolved binding with just the fields `diff_bindings` cares about.
    fn rb(token: &str, actionmap: &str, action: &str) -> ResolvedBinding {
        ResolvedBinding {
            token: token.to_string(),
            device: None,
            device_guid: None,
            device_kind: kind_of(token),
            instance: instance_of(token).unwrap_or(1),
            actionmap: actionmap.to_string(),
            action: action.to_string(),
            label: None,
            is_default: false,
        }
    }

    fn rb_labeled(token: &str, actionmap: &str, action: &str, label: &str) -> ResolvedBinding {
        let mut b = rb(token, actionmap, action);
        b.label = Some(label.to_string());
        b
    }

    #[test]
    fn identical_sets_yield_an_empty_report() {
        let a = vec![rb("js1_button1", "m", "fire"), rb("js2_x", "m", "throttle")];
        let b = a.clone();
        let report = diff_bindings(&a, &b);
        assert!(report.rows.is_empty());
        assert_eq!((report.added, report.removed, report.changed), (0, 0, 0));
    }

    #[test]
    fn classifies_added_removed_and_changed() {
        let a = vec![
            rb("js1_button1", "m", "fire"),  // A only -> added
            rb("js1_button3", "m", "boost"), // both, same -> no row
            rb("js2_x", "m", "throttle_a"),  // both, different action -> changed
        ];
        let b = vec![
            rb("js1_button2", "m", "eject"), // B only -> removed
            rb("js1_button3", "m", "boost"),
            rb("js2_x", "m", "throttle_b"),
        ];
        let report = diff_bindings(&a, &b);
        assert_eq!((report.added, report.removed, report.changed), (1, 1, 1));

        let added = report.rows.iter().find(|r| r.token == "js1_button1").unwrap();
        assert!(matches!(added.kind, DiffKind::Added));
        assert_eq!(added.a.len(), 1);
        assert!(added.b.is_empty());

        let removed = report.rows.iter().find(|r| r.token == "js1_button2").unwrap();
        assert!(matches!(removed.kind, DiffKind::Removed));
        assert!(removed.a.is_empty());
        assert_eq!(removed.b.len(), 1);

        let changed = report.rows.iter().find(|r| r.token == "js2_x").unwrap();
        assert!(matches!(changed.kind, DiffKind::Changed));
        assert_eq!(changed.a[0].action, "throttle_a");
        assert_eq!(changed.b[0].action, "throttle_b");

        assert!(!report.rows.iter().any(|r| r.token == "js1_button3"));
    }

    #[test]
    fn changed_carries_both_full_action_lists() {
        let a = vec![rb("js2_button1", "m", "fire"), rb("js2_button1", "ui", "ready")];
        let b = vec![rb("js2_button1", "m", "fire")];
        let report = diff_bindings(&a, &b);
        assert_eq!(report.rows.len(), 1);
        let row = &report.rows[0];
        assert!(matches!(row.kind, DiffKind::Changed));
        assert_eq!(row.a.len(), 2);
        assert_eq!(row.b.len(), 1);
    }

    #[test]
    fn a_label_only_difference_is_not_a_change() {
        let a = vec![rb_labeled("js1_button1", "m", "fire", "Fire")];
        let b = vec![rb_labeled("js1_button1", "m", "fire", "Shoot")];
        let report = diff_bindings(&a, &b);
        assert!(report.rows.is_empty());
        assert_eq!((report.added, report.removed, report.changed), (0, 0, 0));
    }

    #[test]
    fn rows_sort_naturally_by_instance_then_input() {
        let a = vec![
            rb("js2_x", "m", "throttle"),
            rb("js1_button10", "m", "b10"),
            rb("js1_button2", "m", "b2"),
        ];
        let report = diff_bindings(&a, &[]);
        let tokens: Vec<&str> = report.rows.iter().map(|r| r.token.as_str()).collect();
        assert_eq!(tokens, vec!["js1_button2", "js1_button10", "js2_x"]);
    }

    #[test]
    fn instance_and_kind_are_parsed_from_the_device_prefix() {
        let a = vec![
            rb("js12_button1", "m", "a"),
            rb("kb1_lalt+x", "m", "b"),
            rb("gp1_a", "m", "c"),
            rb("weird", "m", "d"),
        ];
        let report = diff_bindings(&a, &[]);
        let row = |t: &str| report.rows.iter().find(|r| r.token == t).unwrap();
        assert_eq!((row("js12_button1").instance, row("js12_button1").device_kind), (Some(12), DeviceKind::Joystick));
        assert_eq!((row("kb1_lalt+x").instance, row("kb1_lalt+x").device_kind), (Some(1), DeviceKind::Keyboard));
        assert_eq!((row("gp1_a").instance, row("gp1_a").device_kind), (Some(1), DeviceKind::Gamepad));
        // An unrecognisable token keeps the old behaviour: no instance.
        assert_eq!((row("weird").instance, row("weird").device_kind), (None, DeviceKind::Joystick));
    }

    #[test]
    fn rows_group_joystick_then_keyboard_then_gamepad() {
        let a = vec![
            rb("gp1_a", "m", "pad"),
            rb("kb1_b", "m", "key_b"),
            rb("js2_x", "m", "throttle"),
            rb("kb1_a", "m", "key_a"),
            rb("js1_button2", "m", "b2"),
            rb("js1_button10", "m", "b10"),
        ];
        let report = diff_bindings(&a, &[]);
        let tokens: Vec<&str> = report.rows.iter().map(|r| r.token.as_str()).collect();
        assert_eq!(tokens, vec!["js1_button2", "js1_button10", "js2_x", "kb1_a", "kb1_b", "gp1_a"]);
    }

    #[test]
    fn load_source_reads_a_binding_profile_file() {
        let dir = std::env::temp_dir().join(format!("bindsight-diff-test-{}", uuid::Uuid::new_v4()));
        let binding_profiles_dir = config::binding_profiles_dir(dir.to_str().unwrap());
        fs::create_dir_all(&binding_profiles_dir).unwrap();
        let xml = r#"<ActionMaps profileName="test">
          <options type="joystick" instance="1" Product="Stick {0200231D-0000-0000-0000-504944564944}"/>
          <actionmap name="seat_general">
            <action name="v_eject"><rebind input="js1_button1"/></action>
          </actionmap>
        </ActionMaps>"#;
        fs::write(binding_profiles_dir.join("layout_test_exported.xml"), xml).unwrap();

        let actions = vec![scdata::ActionMap {
            name: "seat_general".into(),
            label: None,
            actions: vec![scdata::Action {
                name: "v_eject".into(),
                label: Some("Eject".into()),
                description: None,
                joystick_default: None,
                keyboard_default: None,
                gamepad_default: None,
                mouse_default: None,
            }],
        }];

        let source = Source::Profile { file: "layout_test_exported.xml".to_string() };
        let resolved = load_source(&source, &[], dir.to_str().unwrap(), Path::new("/nonexistent"), &actions).unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].token, "js1_button1");
        assert_eq!(resolved[0].action, "v_eject");

        fs::remove_dir_all(&dir).ok();
    }
}
