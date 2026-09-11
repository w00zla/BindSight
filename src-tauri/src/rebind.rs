//! Write rebinds into `actionmaps.xml` — the out-of-game counterpart of the
//! in-game keybinding screen.
//!
//! Like `resort.rs` the rewrite is textual: only the `<rebind>` elements of
//! the actions being changed are touched, everything else (whitespace, line
//! endings, `<options>`, `<deviceoptions>`, other actions) stays byte for
//! byte. SC keeps one binding per action and device kind (joystick, keyboard,
//! gamepad), so a change replaces every existing rebind of that kind under
//! the action — the first one keeps its other attributes (`activationMode`,
//! `multiTap`, …), only its `input` changes; a missing `<action>` or
//! `<actionmap>` element is created in SC's own layout (one-space indent,
//! the file's line endings).

use serde::Deserialize;

use crate::scdata::{parse_rebind, parse_actionmaps, DeviceKind};

/// One rebind to write: the action and the full SC input as SC stores it
/// (`js2_button5`, `kb1_lalt+x`, `gp1_a`, or a blank `js1_ ` to unbind). An
/// empty `input` removes the action's rebinds of that kind instead, so the
/// shipped default applies again.
#[derive(Debug, Clone, Deserialize)]
pub struct RebindChange {
    pub actionmap: String,
    pub action: String,
    pub kind: DeviceKind,
    pub input: String,
}

/// Apply every change to `xml` and return the new text. The result is parsed
/// once more so a broken rewrite never reaches the disk.
pub fn apply_rebinds(xml: &str, changes: &[RebindChange]) -> Result<String, String> {
    if changes.is_empty() {
        return Err("nothing to rebind".into());
    }
    let mut out = xml.to_string();
    for change in changes {
        if change.actionmap.is_empty() || change.action.is_empty() {
            return Err("rebind without action".into());
        }
        if change.input.contains('"') || change.input.contains('<') || change.input.contains('&') {
            return Err(format!("invalid input {:?}", change.input));
        }
        out = apply_one(&out, change)?;
    }
    parse_actionmaps(&out).map_err(|e| format!("rewrite produced unreadable XML: {e}"))?;
    Ok(out)
}

/// The file's line ending and one indent unit (SC writes one space per level).
struct Layout {
    eol: &'static str,
    unit: &'static str,
}

fn layout_of(xml: &str) -> Layout {
    Layout {
        eol: if xml.contains("\r\n") { "\r\n" } else { "\n" },
        unit: if xml.contains("\n\t") { "\t" } else { " " },
    }
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
/// `xml[from..to]`. `tag` is matched as a whole word so `<action` never
/// matches `<actionmap`.
fn find_element(xml: &str, from: usize, to: usize, tag: &str, name: &str) -> Result<Option<Element>, String> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut pos = from;
    while let Some(rel) = xml[pos..to].find(&open) {
        let start = pos + rel;
        let after = xml[start + open.len()..].chars().next();
        if !matches!(after, Some(c) if c.is_whitespace() || c == '/' || c == '>') {
            pos = start + 1;
            continue;
        }
        let tag_end = xml[start..to].find('>').ok_or_else(|| format!("unterminated <{tag}> tag"))? + start;
        let head = &xml[start..=tag_end];
        let self_closing = head.ends_with("/>");
        let (close_start, end) = if self_closing {
            (tag_end + 1, tag_end + 1)
        } else {
            let c = xml[tag_end..to].find(&close).ok_or_else(|| format!("<{tag}> without {close}"))? + tag_end;
            (c, c + close.len())
        };
        if attr(head, "name") == Some(name) {
            return Ok(Some(Element { start, open_end: tag_end + 1, close_start, end, self_closing }));
        }
        pos = end;
    }
    Ok(None)
}

/// The value of `name="..."` inside a single tag, if present.
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(" {name}=\"");
    let vstart = tag.find(&key)? + key.len();
    let vend = tag[vstart..].find('"')? + vstart;
    Some(&tag[vstart..vend])
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
    let rebind = format!("<rebind input=\"{}\"/>", change.input);

    let profiles_end = xml.find("</ActionProfiles>").ok_or("no <ActionProfiles> in actionmaps.xml")?;
    let Some(mut map) = find_element(&xml, 0, profiles_end, "actionmap", &change.actionmap)? else {
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

    let Some(mut action) = find_element(&xml, map.open_end, map.close_start, "action", &change.action)? else {
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
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut pos = action.open_end;
    while let Some(rel) = xml[pos..action.close_start].find("<rebind") {
        let start = pos + rel;
        let tag_end = xml[start..action.close_start].find('>').ok_or("unterminated <rebind> tag")? + start;
        let head = &xml[start..=tag_end];
        let end = if head.ends_with("/>") {
            tag_end + 1
        } else {
            xml[tag_end..action.close_start].find("</rebind>").ok_or("<rebind> without </rebind>")? + tag_end + "</rebind>".len()
        };
        let same_kind = attr(head, "input")
            .and_then(parse_rebind)
            .is_some_and(|t| t.kind == change.kind);
        if same_kind {
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
            // (`activationMode`, `multiTap`, …).
            for &(start, end) in spans.iter().skip(1).rev() {
                let line_start = start - indent_before(&xml, start).len();
                let line_end = if xml[end..].starts_with(layout.eol) { end + layout.eol.len() } else { end };
                xml.replace_range(line_start..line_end, "");
            }
            let swapped = set_attr(&xml[first_start..first_end], "input", &change.input)
                .ok_or("<rebind> without an input attribute")?;
            xml.replace_range(first_start..first_end, &swapped);
        }
    }
    Ok(xml)
}

/// `tag` with the value of its `name="..."` attribute replaced; `None` when
/// the attribute is missing.
fn set_attr(tag: &str, name: &str, value: &str) -> Option<String> {
    let key = format!(" {name}=\"");
    let vstart = tag.find(&key)? + key.len();
    let vend = tag[vstart..].find('"')? + vstart;
    Some(format!("{}{value}{}", &tag[..vstart], &tag[vend..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(actionmap: &str, action: &str, kind: DeviceKind, input: &str) -> RebindChange {
        RebindChange { actionmap: actionmap.into(), action: action.into(), kind, input: input.into() }
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
}
