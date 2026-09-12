//! Write rebinds into `actionmaps.xml` — the out-of-game counterpart of the
//! in-game keybinding screen.
//!
//! Like `resort.rs` the rewrite is textual: only the `<rebind>` elements of
//! the actions being changed are touched, everything else (whitespace, line
//! endings, `<options>`, `<deviceoptions>`, other actions) stays byte for
//! byte. SC keeps one binding per action and device kind (joystick, keyboard,
//! gamepad), so a change replaces every existing rebind of that kind under
//! the action — the first one keeps its other attributes (`activationMode`,
//! `multiTap`, …) and only its `input` changes, unless the change carries
//! its own attribute list (an apply copying a source's rebind), which then
//! replaces the element as a whole; a missing `<action>` or `<actionmap>`
//! element is created in SC's own layout (one-space indent, the file's line
//! endings).

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::scdata::{parse_actionmaps, parse_rebind, ActionMapsFile, DeviceKind};
use crate::xmltext::{attr, mask_markup, set_attr, tag_end};

/// One rebind to write: the action and the full SC input as SC stores it
/// (`js2_button5`, `kb1_lalt+x`, `gp1_a`, or a blank `js1_ ` to unbind). An
/// empty `input` removes the action's rebinds of that kind instead, so the
/// shipped default applies again. `attrs` are the element's other
/// attributes to write (`activationMode`, …): `None` keeps whatever the
/// replaced element had, `Some` writes exactly these.
#[derive(Debug, Clone, Deserialize)]
pub struct RebindChange {
    pub actionmap: String,
    pub action: String,
    pub kind: DeviceKind,
    pub input: String,
    #[serde(default)]
    pub attrs: Option<Vec<(String, String)>>,
}

/// Apply every change to `xml` and return the new text. Every change is
/// validated first ([`validate`]); the result is parsed once more and checked
/// against the intent ([`verify_applied`]), so neither a broken nor a
/// wrong-meaning rewrite ever reaches the disk.
pub fn apply_rebinds(xml: &str, changes: &[RebindChange]) -> Result<String, String> {
    if changes.is_empty() {
        return Err("nothing to rebind".into());
    }
    for change in changes {
        validate(change)?;
    }
    let before = parse_actionmaps(xml)?;
    let mut out = xml.to_string();
    for change in changes {
        out = apply_one(&out, change)?;
    }
    let after = parse_actionmaps(&out).map_err(|e| format!("rewrite produced unreadable XML: {e}"))?;
    verify_applied(&before, &after, changes)?;
    Ok(out)
}

/// Longest name, token or attribute value accepted from a change.
const MAX_LEN: usize = 128;

/// An SC identifier as it appears in `actionmaps.xml`: actionmap and action
/// names (`spaceship_general`, `v_eject`), attribute names.
fn is_identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_LEN
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        && s.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
}

/// Refuse anything that is not an SC token targeting the change's kind, a
/// blank rebind of that kind, or (for a removal) empty — and anything that
/// could not be written into an attribute verbatim. The frontend derives
/// kind and input from the same token, but the command takes what it gets.
pub fn validate(change: &RebindChange) -> Result<(), String> {
    if !is_identifier(&change.actionmap) || !is_identifier(&change.action) {
        return Err(format!("invalid action name {:?}/{:?}", change.actionmap, change.action));
    }
    let input = change.input.as_str();
    if !input.is_empty() {
        if input.len() > MAX_LEN || !input.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | ' ')) {
            return Err(format!("invalid input {input:?}"));
        }
        let target = parse_rebind(input).ok_or_else(|| format!("invalid input {input:?}"))?;
        if target.kind != change.kind {
            return Err(format!("input {input:?} does not target the {:?} device", change.kind));
        }
        if target.instance == 0 || target.instance > 99 {
            return Err(format!("invalid device instance in {input:?}"));
        }
        match &target.token {
            // A blank rebind is exactly `<prefix><instance>_` plus spaces.
            None => {
                let core = format!("{}{}_", change.kind.token_prefix(), target.instance);
                if input.trim_end_matches(' ') != core {
                    return Err(format!("invalid blank input {input:?}"));
                }
            }
            // A bound token has no spaces and well-formed `+` parts; a
            // joystick token has no `+` at all (SC has no joystick modifiers).
            Some(_) => {
                if input.contains(' ') || input.split('+').any(str::is_empty) {
                    return Err(format!("invalid input {input:?}"));
                }
                if change.kind == DeviceKind::Joystick && input.contains('+') {
                    return Err(format!("invalid joystick input {input:?}"));
                }
            }
        }
    }
    let attrs = change.attrs.as_deref().unwrap_or(&[]);
    if attrs.len() > 8 {
        return Err("too many rebind attributes".into());
    }
    for (i, (name, value)) in attrs.iter().enumerate() {
        let name_ok = is_identifier(name) && name.len() <= 32 && name != "input";
        let value_ok = value.len() <= MAX_LEN && value.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
        if !name_ok || !value_ok || attrs[..i].iter().any(|(n, _)| n == name) {
            return Err(format!("invalid rebind attribute {name:?}={value:?}"));
        }
    }
    Ok(())
}

/// One rebind as the verification sees it.
type Entry<'a> = (&'a str, &'a str, &'a str, &'a [(String, String)]);

/// Check that `after` is `before` with exactly `changes` applied: every
/// changed `(actionmap, action, kind)` holds the change's rebind (or none for
/// a removal) and nothing else of that kind, every other rebind and every
/// `<options>` device is untouched. Catches a textual rewrite that landed in
/// the wrong place (a comment or CDATA holding a look-alike tag, say) even
/// when its output is valid XML.
pub fn verify_applied(before: &ActionMapsFile, after: &ActionMapsFile, changes: &[RebindChange]) -> Result<(), String> {
    let mut wanted: BTreeMap<(&str, &str, DeviceKind), &RebindChange> = BTreeMap::new();
    for c in changes {
        wanted.insert((c.actionmap.as_str(), c.action.as_str(), c.kind), c);
    }
    let kind_of = |input: &str| parse_rebind(input).map(|t| t.kind);
    let is_target = |r: &crate::scdata::Rebind| {
        kind_of(&r.input).is_some_and(|k| wanted.contains_key(&(r.actionmap.as_str(), r.action.as_str(), k)))
    };

    for (&(actionmap, action, kind), change) in &wanted {
        let found: Vec<&crate::scdata::Rebind> = after
            .rebinds
            .iter()
            .filter(|r| r.actionmap == actionmap && r.action == action && kind_of(&r.input) == Some(kind))
            .collect();
        let expect = format!("{actionmap}/{action} ({kind:?})");
        if change.input.is_empty() {
            if !found.is_empty() {
                return Err(format!("rewrite check failed: {expect} still has a rebind"));
            }
        } else {
            let ok = found.len() == 1
                && found[0].input == change.input
                && change.attrs.as_ref().is_none_or(|a| found[0].attrs == *a);
            if !ok {
                return Err(format!("rewrite check failed: {expect} did not end up as {:?}", change.input));
            }
        }
    }

    let mut others_before: Vec<Entry> =
        before.rebinds.iter().filter(|r| !is_target(r)).map(|r| (r.actionmap.as_str(), r.action.as_str(), r.input.as_str(), r.attrs.as_slice())).collect();
    let mut others_after: Vec<Entry> =
        after.rebinds.iter().filter(|r| !is_target(r)).map(|r| (r.actionmap.as_str(), r.action.as_str(), r.input.as_str(), r.attrs.as_slice())).collect();
    others_before.sort();
    others_after.sort();
    if others_before != others_after {
        return Err("rewrite check failed: a rebind outside the change set differs".into());
    }
    let devices = |f: &ActionMapsFile| -> Vec<(u32, String, Option<String>)> {
        f.joysticks.iter().map(|j| (j.instance, j.product_name.clone(), j.product_guid.clone())).collect()
    };
    if devices(before) != devices(after) {
        return Err("rewrite check failed: the device options differ".into());
    }
    Ok(())
}

/// The file's line ending and one indent unit (SC writes one space per level).
struct Layout {
    eol: &'static str,
    unit: &'static str,
}

fn layout_of(xml: &str) -> Layout {
    let eol = if xml.contains("\r\n") {
        "\r\n"
    } else if xml.contains('\n') || !xml.contains('\r') {
        "\n"
    } else {
        "\r"
    };
    Layout { eol, unit: if xml.contains("\n\t") || xml.contains("\r\t") { "\t" } else { " " } }
}

/// A start tag and, for a non-empty element, the span of its children and
/// its end tag.
struct Element {
    start: usize,
    /// Just past the start tag's `>`.
    open_end: usize,
    /// Start of the `</tag>` (== `end` for a self-closing element).
    close_start: usize,
    end: usize,
    self_closing: bool,
}

/// Locate `<tag ... name="value">…</tag>` (or the self-closing form) inside
/// `xml[from..to]`, searching `masked` (see [`mask_markup`]: same offsets,
/// comments and CDATA blanked) so a look-alike inside a comment is never
/// taken for the element. `tag` is matched as a whole word so `<action`
/// never matches `<actionmap`.
fn find_element(xml: &str, masked: &str, from: usize, to: usize, tag: &str, name: &str) -> Result<Option<Element>, String> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut pos = from;
    while let Some(rel) = masked[pos..to].find(&open) {
        let start = pos + rel;
        let after = masked[start + open.len()..].chars().next();
        if !matches!(after, Some(c) if c.is_whitespace() || c == '/' || c == '>') {
            pos = start + 1;
            continue;
        }
        let tag_end = tag_end(masked, start, to).ok_or_else(|| format!("unterminated <{tag}> tag"))?;
        let head = &xml[start..=tag_end];
        let self_closing = head.ends_with("/>");
        let (close_start, end) = if self_closing {
            (tag_end + 1, tag_end + 1)
        } else {
            let c = masked[tag_end..to].find(&close).ok_or_else(|| format!("<{tag}> without {close}"))? + tag_end;
            (c, c + close.len())
        };
        if attr(head, "name") == Some(name) {
            return Ok(Some(Element { start, open_end: tag_end + 1, close_start, end, self_closing }));
        }
        pos = end;
    }
    Ok(None)
}

/// The whitespace between the last line break before `pos` and `pos`.
fn indent_before(xml: &str, pos: usize) -> &str {
    let line_start = xml[..pos].rfind('\n').map_or(0, |i| i + 1);
    let candidate = &xml[line_start..pos];
    if candidate.chars().all(|c| c == ' ' || c == '\t') { candidate } else { "" }
}

/// Turn a self-closing element into an open/close pair with an empty body,
/// so children can be inserted. Returns the element anew.
fn open_up(xml: &mut String, el: &Element, layout: &Layout) -> Element {
    let indent = indent_before(xml, el.start).to_string();
    let head = xml[el.start..el.open_end].trim_end_matches("/>").to_string();
    let tag_name: String = head[1..].chars().take_while(|c| !c.is_whitespace()).collect();
    let replacement = format!("{head}>{}{indent}</{tag_name}>", layout.eol);
    xml.replace_range(el.start..el.end, &replacement);
    let open_end = el.start + head.len() + 1;
    let close_start = open_end + layout.eol.len() + indent.len();
    Element {
        start: el.start,
        open_end,
        close_start,
        end: close_start + tag_name.len() + 3,
        self_closing: false,
    }
}

/// Insert `<tag name="…">` with one child line before the end tag at
/// `close_start`, indented one level below that end tag.
fn insert_child_block(xml: &mut String, close_start: usize, tag: &str, name: &str, child: &str, layout: &Layout) {
    let indent = indent_before(xml, close_start).to_string();
    let unit = layout.unit;
    let eol = layout.eol;
    let text = format!("{unit}<{tag} name=\"{name}\">{eol}{indent}{unit}{unit}{child}{eol}{indent}{unit}</{tag}>{eol}{indent}");
    xml.insert_str(close_start, &text);
}

fn apply_one(xml: &str, change: &RebindChange) -> Result<String, String> {
    let layout = layout_of(xml);
    let mut xml = xml.to_string();
    let remove = change.input.is_empty();
    let mut rebind = format!("<rebind input=\"{}\"", change.input);
    for (name, value) in change.attrs.iter().flatten() {
        rebind.push_str(&format!(" {name}=\"{value}\""));
    }
    rebind.push_str("/>");

    // The mask is recomputed after every edit: offsets shift with the text.
    let masked = mask_markup(&xml);
    let profiles_end = masked.find("</ActionProfiles>").ok_or("no <ActionProfiles> in actionmaps.xml")?;
    let Some(mut map) = find_element(&xml, &masked, 0, profiles_end, "actionmap", &change.actionmap)? else {
        if remove {
            return Ok(xml);
        }
        let child = format!("<action name=\"{}\">", change.action);
        // Two levels at once: the action block goes in as the child line and
        // gets its rebind plus end tag appended right after.
        let indent = indent_before(&xml, profiles_end).to_string();
        let (unit, eol) = (layout.unit, layout.eol);
        let text = format!(
            "{unit}<actionmap name=\"{}\">{eol}{indent}{unit}{unit}{child}{eol}{indent}{unit}{unit}{unit}{rebind}{eol}{indent}{unit}{unit}</action>{eol}{indent}{unit}</actionmap>{eol}{indent}",
            change.actionmap
        );
        xml.insert_str(profiles_end, &text);
        return Ok(xml);
    };
    if map.self_closing {
        map = open_up(&mut xml, &map, &layout);
    }

    let masked = mask_markup(&xml);
    let Some(mut action) = find_element(&xml, &masked, map.open_end, map.close_start, "action", &change.action)? else {
        if remove {
            return Ok(xml);
        }
        insert_child_block(&mut xml, map.close_start, "action", &change.action, &rebind, &layout);
        return Ok(xml);
    };
    if action.self_closing {
        action = open_up(&mut xml, &action, &layout);
    }

    // Every existing rebind of the same kind goes; the first one's place
    // takes the new element, so the file keeps its shape.
    let masked = mask_markup(&xml);
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut pos = action.open_end;
    while let Some(rel) = masked[pos..action.close_start].find("<rebind") {
        let start = pos + rel;
        let after = masked[start + "<rebind".len()..].chars().next();
        if !matches!(after, Some(c) if c.is_whitespace() || c == '/' || c == '>') {
            pos = start + 1;
            continue;
        }
        let tag_end = tag_end(&masked, start, action.close_start).ok_or("unterminated <rebind> tag")?;
        let head = &xml[start..=tag_end];
        let end = if head.ends_with("/>") {
            tag_end + 1
        } else {
            masked[tag_end..action.close_start].find("</rebind>").ok_or("<rebind> without </rebind>")? + tag_end + "</rebind>".len()
        };
        // A <rebind> whose input the textual reader cannot see while the
        // XML parser can would survive as a second binding of the kind:
        // refuse rather than write a file SC reads differently.
        let input = attr(head, "input").ok_or_else(|| format!("<rebind> without a readable input attribute: {head}"))?;
        if parse_rebind(input).is_some_and(|t| t.kind == change.kind) {
            spans.push((start, end));
        }
        pos = end;
    }

    if remove {
        for &(start, end) in spans.iter().rev() {
            let line_start = start - indent_before(&xml, start).len();
            let line_end = if xml[end..].starts_with(layout.eol) { end + layout.eol.len() } else { end };
            xml.replace_range(line_start..line_end, "");
        }
        return Ok(xml);
    }

    match spans.first().copied() {
        None => {
            let indent = indent_before(&xml, action.close_start).to_string();
            let text = format!("{}{rebind}{}{indent}", layout.unit, layout.eol);
            xml.insert_str(action.close_start, &text);
        }
        Some((first_start, first_end)) => {
            // Remove the duplicates back to front (whole lines), then swap
            // the first one's input, keeping its other attributes
            // (`activationMode`, `multiTap`, …) — or, with an attribute list
            // given, replace the element as a whole.
            for &(start, end) in spans.iter().skip(1).rev() {
                let line_start = start - indent_before(&xml, start).len();
                let line_end = if xml[end..].starts_with(layout.eol) { end + layout.eol.len() } else { end };
                xml.replace_range(line_start..line_end, "");
            }
            let swapped = if change.attrs.is_some() {
                rebind.clone()
            } else {
                set_attr(&xml[first_start..first_end], "input", &change.input)
                    .ok_or("<rebind> without an input attribute")?
            };
            xml.replace_range(first_start..first_end, &swapped);
        }
    }
    Ok(xml)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(actionmap: &str, action: &str, kind: DeviceKind, input: &str) -> RebindChange {
        RebindChange { actionmap: actionmap.into(), action: action.into(), kind, input: input.into(), attrs: None }
    }

    // Shaped like SC's own output (LF here, one-space indent).
    const XML: &str = concat!(
        "<ActionMaps>\n",
        " <ActionProfiles version=\"1\" optionsVersion=\"2\" rebindVersion=\"2\" profileName=\"default\">\n",
        "  <options type=\"joystick\" instance=\"1\" Product=\" VKB R {0200231D-0000-0000-0000-504944564944}\"/>\n",
        "  <modifiers />\n",
        "  <actionmap name=\"spaceship_general\">\n",
        "   <action name=\"v_eject\">\n",
        "    <rebind input=\"js1_ \"/>\n",
        "    <rebind input=\"kb1_ralt+y\"/>\n",
        "   </action>\n",
        "   <action name=\"v_boost\">\n",
        "    <rebind input=\"js1_button5\"/>\n",
        "    <rebind input=\"js1_button6\"/>\n",
        "   </action>\n",
        "  </actionmap>\n",
        "  <actionmap name=\"spaceship_general_extra\"/>\n",
        " </ActionProfiles>\n",
        "</ActionMaps>\n",
    );

    #[test]
    fn replaces_the_rebind_of_the_same_kind_only() {
        let out = apply_rebinds(XML, &[change("spaceship_general", "v_eject", DeviceKind::Joystick, "js2_button3")]).unwrap();
        assert!(out.contains("    <rebind input=\"js2_button3\"/>\n    <rebind input=\"kb1_ralt+y\"/>\n"));
        assert!(!out.contains("js1_ \""));
        assert_eq!(out.lines().count(), XML.lines().count());
    }

    #[test]
    fn collapses_duplicates_of_the_kind() {
        let out = apply_rebinds(XML, &[change("spaceship_general", "v_boost", DeviceKind::Joystick, "js1_button9")]).unwrap();
        assert!(out.contains("   <action name=\"v_boost\">\n    <rebind input=\"js1_button9\"/>\n   </action>\n"));
        assert_eq!(out.lines().count(), XML.lines().count() - 1);
    }

    #[test]
    fn an_empty_input_removes_the_kind() {
        let out = apply_rebinds(XML, &[change("spaceship_general", "v_boost", DeviceKind::Joystick, "")]).unwrap();
        assert!(out.contains("   <action name=\"v_boost\">\n   </action>\n"));
        assert!(!out.contains("js1_button5") && !out.contains("js1_button6"));
        assert_eq!(out.lines().count(), XML.lines().count() - 2);
        // Only the kind goes: the keyboard rebind of v_eject stays.
        let out = apply_rebinds(XML, &[change("spaceship_general", "v_eject", DeviceKind::Joystick, "")]).unwrap();
        assert!(out.contains("   <action name=\"v_eject\">\n    <rebind input=\"kb1_ralt+y\"/>\n   </action>\n"));
        // Nothing to remove: the file is untouched, no action or map is created.
        let out = apply_rebinds(XML, &[change("spaceship_general", "v_nope", DeviceKind::Joystick, "")]).unwrap();
        assert_eq!(out, XML);
        let out = apply_rebinds(XML, &[change("no_such_map", "v_nope", DeviceKind::Joystick, "")]).unwrap();
        assert_eq!(out, XML);
    }

    #[test]
    fn adds_a_kind_the_action_lacks() {
        let out = apply_rebinds(XML, &[change("spaceship_general", "v_boost", DeviceKind::Keyboard, "kb1_lalt+b")]).unwrap();
        assert!(out.contains("    <rebind input=\"js1_button6\"/>\n    <rebind input=\"kb1_lalt+b\"/>\n   </action>\n"));
    }

    #[test]
    fn creates_missing_action_and_actionmap() {
        let out = apply_rebinds(
            XML,
            &[
                change("spaceship_general", "v_roll", DeviceKind::Joystick, "js1_x"),
                change("player", "pl_jump", DeviceKind::Gamepad, "gp1_a"),
                change("spaceship_general_extra", "v_extra", DeviceKind::Keyboard, "kb1_e"),
            ],
        )
        .unwrap();
        assert!(out.contains("   </action>\n   <action name=\"v_roll\">\n    <rebind input=\"js1_x\"/>\n   </action>\n  </actionmap>\n"));
        assert!(out.contains("  <actionmap name=\"player\">\n   <action name=\"pl_jump\">\n    <rebind input=\"gp1_a\"/>\n   </action>\n  </actionmap>\n </ActionProfiles>\n"));
        // The self-closing actionmap was opened up.
        assert!(out.contains("  <actionmap name=\"spaceship_general_extra\">\n   <action name=\"v_extra\">\n    <rebind input=\"kb1_e\"/>\n   </action>\n  </actionmap>\n"));
        let profile = parse_actionmaps(&out).unwrap();
        assert_eq!(profile.rebinds.len(), 7);
    }

    #[test]
    fn an_attribute_list_replaces_the_element_as_a_whole() {
        let xml = XML.replace("<rebind input=\"kb1_ralt+y\"/>", "<rebind input=\"kb1_ralt+y\" activationMode=\"hold\"/>");
        let mut c = change("spaceship_general", "v_eject", DeviceKind::Keyboard, "kb1_e");
        c.attrs = Some(vec![("multiTap".into(), "2".into())]);
        let out = apply_rebinds(&xml, &[c.clone()]).unwrap();
        assert!(out.contains("    <rebind input=\"kb1_e\" multiTap=\"2\"/>\n"));
        assert!(!out.contains("activationMode"));
        // An empty list strips the attributes; a new element gets them too.
        c.attrs = Some(Vec::new());
        let out = apply_rebinds(&xml, &[c.clone()]).unwrap();
        assert!(out.contains("    <rebind input=\"kb1_e\"/>\n"));
        c.action = "v_new".into();
        c.attrs = Some(vec![("activationMode".into(), "press".into())]);
        let out = apply_rebinds(&xml, &[c]).unwrap();
        assert!(out.contains("<rebind input=\"kb1_e\" activationMode=\"press\"/>"));
    }

    #[test]
    fn keeps_the_other_attributes_of_a_replaced_rebind() {
        let xml = XML.replace("<rebind input=\"kb1_ralt+y\"/>", "<rebind input=\"kb1_ralt+y\" activationMode=\"hold\" multiTap=\"2\"/>");
        let out = apply_rebinds(&xml, &[change("spaceship_general", "v_eject", DeviceKind::Keyboard, "kb1_e")]).unwrap();
        assert!(out.contains("<rebind input=\"kb1_e\" activationMode=\"hold\" multiTap=\"2\"/>"));
        assert!(!out.contains("kb1_ralt+y"));
    }

    #[test]
    fn unbinds_with_a_blank_rebind() {
        let out = apply_rebinds(XML, &[change("spaceship_general", "v_boost", DeviceKind::Joystick, "js1_ ")]).unwrap();
        assert!(out.contains("   <action name=\"v_boost\">\n    <rebind input=\"js1_ \"/>\n   </action>\n"));
        let profile = parse_actionmaps(&out).unwrap();
        assert!(profile.rebinds.iter().any(|r| r.action == "v_boost" && r.input == "js1_ "));
    }

    #[test]
    fn keeps_crlf() {
        let crlf = XML.replace('\n', "\r\n");
        let out = apply_rebinds(&crlf, &[change("player", "pl_jump", DeviceKind::Gamepad, "gp1_a")]).unwrap();
        assert!(!out.contains("\n\n"));
        assert!(out.contains("  <actionmap name=\"player\">\r\n   <action name=\"pl_jump\">\r\n    <rebind input=\"gp1_a\"/>\r\n   </action>\r\n  </actionmap>\r\n"));
        assert_eq!(out.matches('\n').count(), out.matches("\r\n").count());
    }

    #[test]
    fn rejects_bad_input() {
        assert!(apply_rebinds(XML, &[]).is_err());
        assert!(apply_rebinds(XML, &[change("a", "", DeviceKind::Joystick, "js1_x")]).is_err());
        assert!(apply_rebinds(XML, &[change("a", "b", DeviceKind::Joystick, "js1_x\"/><x")]).is_err());
        assert!(apply_rebinds("<ActionMaps/>", &[change("a", "b", DeviceKind::Joystick, "js1_x")]).is_err());
    }

    #[test]
    fn validate_accepts_sc_tokens_and_refuses_the_rest() {
        let ok = |kind, input| validate(&change("spaceship_general", "v_eject", kind, input)).unwrap();
        ok(DeviceKind::Joystick, "js1_button5");
        ok(DeviceKind::Joystick, "js12_rotz");
        ok(DeviceKind::Joystick, "js2_ ");
        ok(DeviceKind::Joystick, "");
        ok(DeviceKind::Keyboard, "kb1_lalt+x");
        ok(DeviceKind::Keyboard, "kb1_np_add");
        ok(DeviceKind::Keyboard, "kb1_mwheel_up");
        ok(DeviceKind::Keyboard, "kb1_ ");
        ok(DeviceKind::Gamepad, "gp1_shoulderl+thumbl_left");
        ok(DeviceKind::Gamepad, "gp1_triggerl_btn");

        let bad = |kind, input| assert!(validate(&change("spaceship_general", "v_eject", kind, input)).is_err(), "{input:?}");
        bad(DeviceKind::Keyboard, "js1_button5"); // kind mismatch
        bad(DeviceKind::Joystick, "kb1_x");
        bad(DeviceKind::Joystick, "button5"); // no prefix
        bad(DeviceKind::Joystick, "js1_button 5"); // space in a bound token
        bad(DeviceKind::Joystick, "js1_+button5"); // empty combo part
        bad(DeviceKind::Joystick, "+js1_button5");
        bad(DeviceKind::Joystick, "js1_button5+");
        bad(DeviceKind::Joystick, "lctrl+js1_button1"); // SC has no joystick modifiers
        bad(DeviceKind::Joystick, "js1_button1+js1_button2");
        bad(DeviceKind::Joystick, "lctrl+js1_ "); // blank with a modifier
        bad(DeviceKind::Joystick, "js1_\t"); // blank with a tab
        bad(DeviceKind::Joystick, "js1_büton");
        bad(DeviceKind::Joystick, "js1_x\"/><x");
        let long = format!("js1_{}", "b".repeat(200));
        bad(DeviceKind::Joystick, &long);
        bad(DeviceKind::Keyboard, "kb2_x"); // SC knows one keyboard
        bad(DeviceKind::Gamepad, "gp2_a");

        let names = |actionmap, action| validate(&change(actionmap, action, DeviceKind::Joystick, "js1_x"));
        assert!(names("spaceship_general", "v_eject").is_ok());
        assert!(names("IFCS_controls", "v_ifcs_toggle-x.y").is_ok());
        assert!(names("a\"/><action name=\"b", "v").is_err());
        assert!(names("a&amp;b", "v").is_err());
        assert!(names("1abc", "v").is_err());
        assert!(names("", "v").is_err());
        assert!(names("a b", "v").is_err());

        let mut c = change("spaceship_general", "v_eject", DeviceKind::Keyboard, "kb1_x");
        c.attrs = Some(vec![("activationMode".into(), "delayed_press".into()), ("multiTap".into(), "2".into())]);
        assert!(validate(&c).is_ok());
        for attrs in [
            vec![("input".to_string(), "kb1_y".to_string())],
            vec![("activation Mode".to_string(), "x".to_string())],
            vec![("activationMode".to_string(), "a\"b".to_string())],
            vec![("activationMode".to_string(), "x".to_string()), ("activationMode".to_string(), "y".to_string())],
            vec![("a".to_string(), "b".to_string()); 9],
        ] {
            c.attrs = Some(attrs.clone());
            assert!(validate(&c).is_err(), "{attrs:?}");
        }
    }

    #[test]
    fn verify_applied_catches_a_rewrite_with_the_wrong_meaning() {
        let before = parse_actionmaps(XML).unwrap();
        let c = change("spaceship_general", "v_boost", DeviceKind::Joystick, "js1_button9");
        // The honest rewrite passes.
        let good = apply_rebinds(XML, &[c.clone()]).unwrap();
        verify_applied(&before, &parse_actionmaps(&good).unwrap(), &[c.clone()]).unwrap();
        // The rebind landed under another action: valid XML, wrong meaning.
        let wrong = XML.replace("<rebind input=\"js1_ \"/>", "<rebind input=\"js1_button9\"/>");
        let err = verify_applied(&before, &parse_actionmaps(&wrong).unwrap(), &[c.clone()]).unwrap_err();
        assert!(err.contains("rewrite check failed"), "{err}");
        // The target is right but a bystander changed.
        let bystander = good.replace("kb1_ralt+y", "kb1_ralt+z");
        assert!(verify_applied(&before, &parse_actionmaps(&bystander).unwrap(), &[c.clone()]).is_err());
        // The target is right but a device option changed.
        let device = good.replace("instance=\"1\" Product", "instance=\"2\" Product");
        assert!(verify_applied(&before, &parse_actionmaps(&device).unwrap(), &[c]).is_err());
    }

    #[test]
    fn look_alike_tags_in_comments_and_cdata_are_ignored() {
        // A commented-out actionmap + action + rebind for the very action
        // being changed, a CDATA rebind inside the real action, and a
        // commented </ActionProfiles>.
        let xml = XML
            .replace(
                "  <actionmap name=\"spaceship_general\">\n",
                "  <!-- <actionmap name=\"spaceship_general\"><action name=\"v_boost\"><rebind input=\"js1_button1\"/></action></actionmap> </ActionProfiles> -->\n  <actionmap name=\"spaceship_general\">\n",
            )
            .replace(
                "    <rebind input=\"js1_button5\"/>\n",
                "    <![CDATA[<rebind input=\"js1_button7\"/>]]>\n    <!-- <rebind input=\"js1_button8\"/> -->\n    <rebind input=\"js1_button5\"/>\n",
            );
        let c = change("spaceship_general", "v_boost", DeviceKind::Joystick, "js1_button9");
        let out = apply_rebinds(&xml, &[c]).unwrap();
        // The real rebind changed, the comment and the CDATA survived untouched.
        assert!(out.contains("    <rebind input=\"js1_button9\"/>\n"));
        assert!(out.contains("<![CDATA[<rebind input=\"js1_button7\"/>]]>"));
        assert!(out.contains("<!-- <rebind input=\"js1_button8\"/> -->"));
        assert!(out.contains("<action name=\"v_boost\"><rebind input=\"js1_button1\"/></action></actionmap> </ActionProfiles> -->"));
        assert!(!out.contains("js1_button6"), "the duplicate of the kind went");
        // A new actionmap lands before the real </ActionProfiles>, not the commented one.
        let out = apply_rebinds(&xml, &[change("new_map", "v_new", DeviceKind::Keyboard, "kb1_q")]).unwrap();
        assert!(out.find("<actionmap name=\"new_map\">").unwrap() > out.find("<!-- <actionmap").unwrap());
        assert!(out.find("<actionmap name=\"new_map\">").unwrap() < out.rfind("</ActionProfiles>").unwrap());
    }

    #[test]
    fn attributes_in_any_spelling_are_replaced_not_duplicated() {
        for spelling in [
            "<rebind input='js1_button5'/>",
            "<rebind\tinput=\"js1_button5\" />",
            "<rebind input = \"js1_button5\"/>",
            "<rebind activationMode=\"press\" input=\"js1_button5\"></rebind>",
            "<rebind input=\"js1_button5\"><child/></rebind>",
        ] {
            let xml = XML.replace("<rebind input=\"js1_button5\"/>", spelling);
            let out = apply_rebinds(&xml, &[change("spaceship_general", "v_boost", DeviceKind::Joystick, "js1_button9")]).unwrap();
            let file = parse_actionmaps(&out).unwrap();
            let boost: Vec<&str> = file.rebinds.iter().filter(|r| r.action == "v_boost").map(|r| r.input.as_str()).collect();
            assert_eq!(boost, vec!["js1_button9"], "{spelling}");
        }
        // An input the parser reads but the text scanner cannot (unquoted):
        // refused, never a second binding.
        let xml = XML.replace("<rebind input=\"js1_button5\"/>", "<rebind input=js1_button5/>");
        assert!(apply_rebinds(&xml, &[change("spaceship_general", "v_boost", DeviceKind::Joystick, "js1_button9")]).is_err());
    }

    #[test]
    fn keeps_a_bom_and_cr_only_line_endings() {
        let bom = format!("\u{feff}{XML}");
        let out = apply_rebinds(&bom, &[change("spaceship_general", "v_boost", DeviceKind::Joystick, "js1_button9")]).unwrap();
        assert!(out.starts_with('\u{feff}'));
        assert!(out.contains("<rebind input=\"js1_button9\"/>"));
        let cr = XML.replace('\n', "\r");
        let out = apply_rebinds(&cr, &[change("new_map", "v_new", DeviceKind::Keyboard, "kb1_q")]).unwrap();
        assert!(!out.contains('\n'), "no foreign line ending introduced");
        assert!(out.contains("<actionmap name=\"new_map\">\r"));
    }

    #[test]
    fn opens_up_a_self_closing_action() {
        let xml = XML.replace("   <action name=\"v_boost\">\n    <rebind input=\"js1_button5\"/>\n    <rebind input=\"js1_button6\"/>\n   </action>\n", "   <action name=\"v_boost\"/>\n");
        let out = apply_rebinds(&xml, &[change("spaceship_general", "v_boost", DeviceKind::Joystick, "js1_button9")]).unwrap();
        assert!(out.contains("   <action name=\"v_boost\">\n    <rebind input=\"js1_button9\"/>\n   </action>\n"), "{out}");
    }
}
