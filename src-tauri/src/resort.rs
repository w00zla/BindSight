//! Apply a slot permutation to `actionmaps.xml` — the out-of-game counterpart
//! of SC's `pp_resortdevices` console command.
//!
//! The file is SC's live config, so the rewrite is textual and touches only
//! two things: the `instance` of every `<options type="joystick">` element
//! (the element moves whole, deadzone/invert children included, and the
//! joystick blocks are re-emitted in ascending slot order as SC writes them)
//! and the `jsN_` prefix of every token inside an `input="..."` attribute
//! (rebinds, blank `js1_ ` rebinds and modifier combos alike). Everything
//! else — whitespace, line endings, attribute order, `<deviceoptions>` (keyed
//! by product name, not slot) — is left byte for byte.

use std::collections::BTreeMap;

use crate::bindings::ResortMove;
use crate::scdata::{parse_actionmaps, ActionMapsFile};
use crate::xmltext::{attr, find_attr, mask_markup, tag_end};

/// Rewrite `xml` so that every binding saved under `js{from}` lives under
/// `js{to}` for each move, and the joystick `<options>` slots follow. `moves`
/// must be a bijection (each slot at most once as source and as target),
/// as [`crate::bindings::plan_resort`] produces.
pub fn rewrite_actionmaps(xml: &str, moves: &[ResortMove]) -> Result<String, String> {
    let map = slot_map(moves)?;
    if map.is_empty() {
        return Err("nothing to resort".into());
    }
    let before = parse_actionmaps(xml)?;
    let renumbered = renumber_options(xml, &map)?;
    let out = renumber_inputs(&renumbered, &map);
    let after = parse_actionmaps(&out).map_err(|e| format!("rewrite produced unreadable XML: {e}"))?;
    verify_applied(&before, &after, &map)?;
    Ok(out)
}

/// Check that `after` is `before` with the slots renumbered and nothing else:
/// every rebind in place with its tokens mapped, attributes untouched, the
/// joystick devices the same set on their new slots.
pub fn verify_applied(before: &ActionMapsFile, after: &ActionMapsFile, map: &BTreeMap<u32, u32>) -> Result<(), String> {
    if before.rebinds.len() != after.rebinds.len() {
        return Err("rewrite check failed: the number of rebinds changed".into());
    }
    for (b, a) in before.rebinds.iter().zip(&after.rebinds) {
        let same = b.actionmap == a.actionmap && b.action == a.action && b.attrs == a.attrs && renumber_tokens(&b.input, map) == a.input;
        if !same {
            return Err(format!("rewrite check failed: {}/{} {:?} became {:?}", b.actionmap, b.action, b.input, a.input));
        }
    }
    let devices = |f: &ActionMapsFile, mapped: bool| -> Vec<(u32, String, Option<String>)> {
        let mut v: Vec<_> = f
            .joysticks
            .iter()
            .map(|j| {
                let n = if mapped { map.get(&j.instance).copied().unwrap_or(j.instance) } else { j.instance };
                (n, j.product_name.clone(), j.product_guid.clone())
            })
            .collect();
        v.sort();
        v
    };
    if devices(before, true) != devices(after, false) {
        return Err("rewrite check failed: the joystick devices do not match the moves".into());
    }
    Ok(())
}

/// `from -> to` for the non-identity moves, checked for bijectivity.
fn slot_map(moves: &[ResortMove]) -> Result<BTreeMap<u32, u32>, String> {
    let mut map = BTreeMap::new();
    for m in moves {
        if map.insert(m.from, m.to).is_some() {
            return Err(format!("slot js{} moved twice", m.from));
        }
    }
    let mut targets: Vec<u32> = map.values().copied().collect();
    targets.sort_unstable();
    targets.dedup();
    let sources: Vec<u32> = map.keys().copied().collect();
    if targets != sources {
        return Err("resort is not a permutation of the slots".into());
    }
    Ok(map)
}

/// A joystick `<options>` element in the text: its byte span and slot.
struct OptionsBlock {
    start: usize,
    end: usize,
    instance: u32,
}

/// Locate every `<options type="joystick" instance="N" ...>` element,
/// self-closing or with children up to its `</options>`. Searches the
/// masked text, so one inside a comment does not count.
fn find_joystick_options(xml: &str) -> Result<Vec<OptionsBlock>, String> {
    let masked = mask_markup(xml);
    let mut blocks = Vec::new();
    let mut pos = 0;
    while let Some(rel) = masked[pos..].find("<options") {
        let start = pos + rel;
        // Make sure it is the tag and not e.g. `<optionsfoo`.
        let after = masked[start + "<options".len()..].chars().next();
        if !matches!(after, Some(c) if c.is_whitespace() || c == '/' || c == '>') {
            pos = start + 1;
            continue;
        }
        let tag_end = tag_end(&masked, start, masked.len()).ok_or("unterminated <options> tag")?;
        let tag = &xml[start..=tag_end];
        let self_closing = tag.ends_with("/>");
        let end = if self_closing {
            tag_end + 1
        } else {
            let close = masked[tag_end..].find("</options>").ok_or("<options> without </options>")?;
            tag_end + close + "</options>".len()
        };
        if attr(tag, "type") == Some("joystick") {
            let instance = attr(tag, "instance")
                .ok_or("joystick <options> without instance")?
                .parse::<u32>()
                .map_err(|e| format!("bad joystick <options> instance: {e}"))?;
            blocks.push(OptionsBlock { start, end, instance });
        }
        pos = end;
    }
    Ok(blocks)
}

/// Renumber the joystick `<options>` elements and re-emit them, sorted by
/// their new slot, into the byte spans the old ones occupied (so the
/// surrounding whitespace stays put).
fn renumber_options(xml: &str, map: &BTreeMap<u32, u32>) -> Result<String, String> {
    let blocks = find_joystick_options(xml)?;
    if blocks.is_empty() {
        return Err("no <options type=\"joystick\"> in actionmaps.xml".into());
    }
    let mut renumbered: Vec<(u32, String)> = blocks
        .iter()
        .map(|b| {
            let new = map.get(&b.instance).copied().unwrap_or(b.instance);
            let text = &xml[b.start..b.end];
            // Only the opening tag carries the attribute; children never do.
            let head_end = tag_end(text, 0, text.len()).unwrap_or(text.len() - 1);
            let head = &text[..=head_end];
            let renumbered = find_attr(head, "instance")
                .map(|s| format!("{}{new}{}", &head[..s.value_start], &head[s.value_end..]))
                .unwrap_or_else(|| head.to_string());
            (new, format!("{renumbered}{}", &text[head_end + 1..]))
        })
        .collect();
    renumbered.sort_by_key(|(n, _)| *n);

    let mut out = String::with_capacity(xml.len());
    let mut last = 0;
    for (block, (_, text)) in blocks.iter().zip(renumbered) {
        out.push_str(&xml[last..block.start]);
        out.push_str(&text);
        last = block.end;
    }
    out.push_str(&xml[last..]);
    Ok(out)
}

/// Replace the `jsN_` prefix of every token inside the `input` attribute of
/// every tag (comments and CDATA skipped, either quote style read).
fn renumber_inputs(xml: &str, map: &BTreeMap<u32, u32>) -> String {
    let masked = mask_markup(xml);
    let mut out = String::with_capacity(xml.len());
    // `copied`: how far `xml` has been copied into `out`; `search`: where
    // the next tag is looked for.
    let mut copied = 0;
    let mut search = 0;
    while let Some(rel) = masked[search..].find('<') {
        let start = search + rel;
        if masked[start + 1..].starts_with('/') {
            search = start + 1;
            continue;
        }
        let Some(end) = tag_end(&masked, start, masked.len()) else {
            break;
        };
        let head = &xml[start..=end];
        if let Some(s) = find_attr(head, "input") {
            out.push_str(&xml[copied..start + s.value_start]);
            out.push_str(&renumber_tokens(&head[s.value_start..s.value_end], map));
            copied = start + s.value_end;
        }
        search = end + 1;
    }
    out.push_str(&xml[copied..]);
    out
}

/// `js2_button5+js2_x` -> with 2->1: `js1_button5+js1_x`. Anything that is
/// not `js<digits>_` is copied through.
fn renumber_tokens(value: &str, map: &BTreeMap<u32, u32>) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(i) = rest.find("js") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let digits = after.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 && after[digits..].starts_with('_') {
            if let Ok(n) = after[..digits].parse::<u32>() {
                out.push_str(&format!("js{}_", map.get(&n).copied().unwrap_or(n)));
                rest = &after[digits + 1..];
                continue;
            }
        }
        out.push_str("js");
        rest = after;
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mv(from: u32, to: u32) -> ResortMove {
        ResortMove { from, to, name: None }
    }

    // Shaped like SC's own output (CRLF, one-space indent, empty slots).
    // Rust strips the indentation after a `\` continuation, so concat! it is.
    const XML: &str = concat!(
        "<ActionMaps>\r\n",
        " <ActionProfiles version=\"1\" optionsVersion=\"2\" rebindVersion=\"2\" profileName=\"default\">\r\n",
        "  <deviceoptions name=\" VKB L {0201231D-0000-0000-0000-504944564944}\">\r\n",
        "   <option input=\"x\" deadzone=\"0.05\"/>\r\n",
        "  </deviceoptions>\r\n",
        "  <options type=\"keyboard\" instance=\"1\" Product=\"Wine Keyboard  {6F1D2B61-D5A0-11CF-BFC7-444553540000}\"/>\r\n",
        "  <options type=\"gamepad\" instance=\"1\" Product=\"Controller (Gamepad)\">\r\n",
        "   <flight_view exponent=\"1\"/>\r\n",
        "  </options>\r\n",
        "  <options type=\"joystick\" instance=\"1\" Product=\" VKB R {0200231D-0000-0000-0000-504944564944}\">\r\n",
        "   <flight_move_roll invert=\"1\"/>\r\n",
        "  </options>\r\n",
        "  <options type=\"joystick\" instance=\"2\" Product=\" VKB L {0201231D-0000-0000-0000-504944564944}\"/>\r\n",
        "  <options type=\"joystick\" instance=\"3\"/>\r\n",
        "  <modifiers />\r\n",
        "  <actionmap name=\"spaceship_general\">\r\n",
        "   <action name=\"v_eject\">\r\n",
        "    <rebind input=\"js1_ \"/>\r\n",
        "   </action>\r\n",
        "   <action name=\"v_boost\">\r\n",
        "    <rebind input=\"js2_button5\"/>\r\n",
        "    <rebind input=\"js2_button1+js2_x\"/>\r\n",
        "   </action>\r\n",
        "   <action name=\"v_pitch\">\r\n",
        "    <rebind input=\"js12_rotz\"/>\r\n",
        "    <rebind input=\"kb1_space\"/>\r\n",
        "   </action>\r\n",
        "  </actionmap>\r\n",
        " </ActionProfiles>\r\n",
        "</ActionMaps>\r\n",
    );

    #[test]
    fn swaps_two_slots_options_and_inputs() {
        let out = rewrite_actionmaps(XML, &[mv(1, 2), mv(2, 1)]).unwrap();
        // The joystick blocks are re-emitted sorted by new slot, whole.
        let i_l = out.find("instance=\"1\" Product=\" VKB L").unwrap();
        let i_r = out.find("instance=\"2\" Product=\" VKB R").unwrap();
        assert!(i_l < i_r);
        assert!(out.contains("instance=\"2\" Product=\" VKB R {0200231D-0000-0000-0000-504944564944}\">\r\n   <flight_move_roll invert=\"1\"/>\r\n  </options>"));
        // Inputs follow, including blank rebinds and modifier combos.
        assert!(out.contains("<rebind input=\"js2_ \"/>"));
        assert!(out.contains("<rebind input=\"js1_button5\"/>"));
        assert!(out.contains("<rebind input=\"js1_button1+js1_x\"/>"));
        // Untouched: other slots, other devices, keyboard/gamepad options, deviceoptions.
        assert!(out.contains("<rebind input=\"js12_rotz\"/>"));
        assert!(out.contains("<rebind input=\"kb1_space\"/>"));
        assert!(out.contains("<options type=\"keyboard\" instance=\"1\""));
        assert!(out.contains("<options type=\"gamepad\" instance=\"1\""));
        assert!(out.contains("<options type=\"joystick\" instance=\"3\"/>"));
        assert!(out.contains("<deviceoptions name=\" VKB L {0201231D-0000-0000-0000-504944564944}\">"));
        // Line endings and structure survive; a second swap restores the file.
        assert_eq!(out.matches("\r\n").count(), XML.matches("\r\n").count());
        assert_eq!(rewrite_actionmaps(&out, &[mv(1, 2), mv(2, 1)]).unwrap(), XML);
    }

    #[test]
    fn cycle_over_three_slots() {
        // 1->3, 3->2, 2->1
        let out = rewrite_actionmaps(XML, &[mv(1, 3), mv(2, 1), mv(3, 2)]).unwrap();
        assert!(out.contains("instance=\"3\" Product=\" VKB R"));
        assert!(out.contains("instance=\"1\" Product=\" VKB L"));
        assert!(out.contains("<options type=\"joystick\" instance=\"2\"/>"));
        assert!(out.contains("<rebind input=\"js3_ \"/>"));
        assert!(out.contains("<rebind input=\"js1_button5\"/>"));
        let order: Vec<usize> = ["instance=\"1\" Product", "instance=\"2\"/>", "instance=\"3\" Product"]
            .iter()
            .map(|s| out.find(s).unwrap())
            .collect();
        assert!(order[0] < order[1] && order[1] < order[2]);
    }

    #[test]
    fn comments_and_single_quotes_do_not_confuse_the_rewrite() {
        let xml = XML
            .replace(
                "  <options type=\"keyboard\" instance=\"1\"",
                "  <!-- <options type=\"joystick\" instance=\"1\" Product=\"ghost\"/> <rebind input=\"js1_ghost\"/> -->\r\n  <options type='joystick' instance='9' Product='Nine'/>\r\n  <options type=\"keyboard\" instance=\"1\"",
            )
            .replace("<rebind input=\"js2_button5\"/>", "<rebind input='js2_button5'/>");
        let out = rewrite_actionmaps(&xml, &[mv(1, 2), mv(2, 1), mv(9, 3), mv(3, 9)]).unwrap();
        assert!(out.contains("<!-- <options type=\"joystick\" instance=\"1\" Product=\"ghost\"/> <rebind input=\"js1_ghost\"/> -->"), "comment untouched");
        assert!(out.contains("<options type='joystick' instance='3' Product='Nine'/>"), "{out}");
        assert!(out.contains("<rebind input='js1_button5'/>"));
        // The way back restores the meaning (blocks are re-emitted sorted by
        // slot, so with four of them the bytes rotate, the content does not).
        let back = rewrite_actionmaps(&out, &[mv(1, 2), mv(2, 1), mv(9, 3), mv(3, 9)]).unwrap();
        let (a, b) = (parse_actionmaps(&xml).unwrap(), parse_actionmaps(&back).unwrap());
        let devs = |f: &ActionMapsFile| {
            let mut v: Vec<_> = f.joysticks.iter().map(|j| (j.instance, j.product_name.clone())).collect();
            v.sort();
            v
        };
        assert_eq!(devs(&a), devs(&b));
        assert_eq!(a.rebinds.iter().map(|r| &r.input).collect::<Vec<_>>(), b.rebinds.iter().map(|r| &r.input).collect::<Vec<_>>());
        assert!(back.contains("<!-- <options type=\"joystick\" instance=\"1\" Product=\"ghost\"/>"));
    }

    #[test]
    fn verify_applied_catches_a_wrong_rewrite() {
        let before = parse_actionmaps(XML).unwrap();
        let map: BTreeMap<u32, u32> = [(1, 2), (2, 1)].into_iter().collect();
        let good = rewrite_actionmaps(XML, &[mv(1, 2), mv(2, 1)]).unwrap();
        verify_applied(&before, &parse_actionmaps(&good).unwrap(), &map).unwrap();
        // A token left behind on its old slot.
        let stale = good.replace("<rebind input=\"js1_button5\"/>", "<rebind input=\"js2_button5\"/>");
        assert!(verify_applied(&before, &parse_actionmaps(&stale).unwrap(), &map).is_err());
        // A device that did not move with its bindings.
        let device = good.replace("instance=\"2\" Product=\" VKB R", "instance=\"3\" Product=\" VKB R");
        assert!(verify_applied(&before, &parse_actionmaps(&device).unwrap(), &map).is_err());
        // A rebind lost on the way.
        let lost = good.replace("    <rebind input=\"kb1_space\"/>\r\n", "");
        assert!(verify_applied(&before, &parse_actionmaps(&lost).unwrap(), &map).is_err());
    }

    #[test]
    fn rejects_non_permutations_and_empty_plans() {
        assert!(rewrite_actionmaps(XML, &[]).is_err());
        assert!(rewrite_actionmaps(XML, &[mv(1, 2)]).is_err()); // 2 never leaves
        assert!(rewrite_actionmaps(XML, &[mv(1, 2), mv(1, 3), mv(2, 1), mv(3, 1)]).is_err());
        assert!(rewrite_actionmaps("<ActionMaps/>", &[mv(1, 2), mv(2, 1)]).is_err());
    }

    #[test]
    fn token_renumbering_is_precise() {
        let map: BTreeMap<u32, u32> = [(1, 2), (2, 1)].into_iter().collect();
        assert_eq!(renumber_tokens("js1_button1", &map), "js2_button1");
        assert_eq!(renumber_tokens("js1_button1+js2_hat1_up", &map), "js2_button1+js1_hat1_up");
        assert_eq!(renumber_tokens("js3_x", &map), "js3_x"); // not in the map
        assert_eq!(renumber_tokens("kb1_js", &map), "kb1_js"); // no digits/underscore
        assert_eq!(renumber_tokens("js_button1", &map), "js_button1");
        assert_eq!(renumber_tokens("", &map), "");
    }
}
