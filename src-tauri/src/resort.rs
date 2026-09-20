//! Apply joystick slot swaps to `actionmaps.xml` — the out-of-game counterpart
//! of SC's `pp_resortdevices joystick A B` console command, replicating what
//! that command does to the file (observed 2026-09-20, SC 4.10):
//!
//! - The **contents** of the two slots swap: every `jsA_` token in an
//!   `input="..."` attribute becomes `jsB_` and vice versa (rebinds, blank
//!   `jsA_ ` rebinds that switch a default off, modifier combos alike), and
//!   the child elements of the two `<options type="joystick">` elements
//!   (invert / exponent settings) swap.
//! - The **device assignment stays**: the `instance` / `Product` attributes
//!   of the `<options>` elements and their order in the file are untouched.
//!   That map is the game's own per-session record and no business of the
//!   command.
//! - Not replicated, not understood: in the observed file the game left two
//!   blank rebinds on the source slot (`turret_toggle_mouse_mode`,
//!   `v_cycle_pitch_ladder_mode`, both without a joystick default) while it
//!   moved 350 blanks of the same shape. Every token moves here.
//!
//! Several swaps are applied one after the other, in the order the console
//! commands would be entered. The rewrite is textual: whitespace, line
//! endings, attribute order, comments and `<deviceoptions>` (keyed by product
//! name, not slot) are left byte for byte. Every swap re-parses its output and
//! checks it against the intent before the next one.

use crate::scdata::{parse_actionmaps, ActionMapsFile};
use crate::xmltext::{attr, find_attr, insert_attr, mask_markup, remove_attr, set_attr, tag_end};

/// Apply `swaps` (pairs of slots, `pp_resortdevices joystick A B` each) in
/// order to `xml`.
pub fn rewrite_actionmaps(xml: &str, swaps: &[(u32, u32)]) -> Result<String, String> {
    if swaps.is_empty() {
        return Err("nothing to resort".into());
    }
    let mut out = xml.to_string();
    for &(a, b) in swaps {
        if a == 0 || b == 0 {
            return Err("joystick slots start at js1".into());
        }
        if a == b {
            return Err(format!("cannot swap js{a} with itself"));
        }
        let before = parse_actionmaps(&out)?;
        let swapped = swap_options_children(&out, a, b)?;
        let swapped = swap_inputs(&swapped, a, b);
        let after = parse_actionmaps(&swapped).map_err(|e| format!("rewrite produced unreadable XML: {e}"))?;
        verify_applied(&before, &after, a, b)?;
        verify_children_swapped(&out, &swapped, a, b)?;
        out = swapped;
    }
    Ok(out)
}

/// Check that `after` is `before` with the slots `a` and `b` swapped in the
/// rebinds and nothing else: every rebind in place with its tokens swapped,
/// attributes untouched, the joystick device map identical.
pub fn verify_applied(before: &ActionMapsFile, after: &ActionMapsFile, a: u32, b: u32) -> Result<(), String> {
    if before.rebinds.len() != after.rebinds.len() {
        return Err("rewrite check failed: the number of rebinds changed".into());
    }
    for (x, y) in before.rebinds.iter().zip(&after.rebinds) {
        let same = x.actionmap == y.actionmap && x.action == y.action && x.attrs == y.attrs && swap_tokens(&x.input, a, b) == y.input;
        if !same {
            return Err(format!("rewrite check failed: {}/{} {:?} became {:?}", x.actionmap, x.action, x.input, y.input));
        }
    }
    let devices = |f: &ActionMapsFile| -> Vec<(u32, String, Option<String>)> {
        f.joysticks.iter().map(|j| (j.instance, j.product_name.clone(), j.product_guid.clone())).collect()
    };
    if devices(before) != devices(after) {
        return Err("rewrite check failed: the joystick device map changed".into());
    }
    Ok(())
}

/// A joystick `<options>` element in the text: the byte span of the whole
/// element, the end of its opening tag (index of its `>`), whether it is
/// self-closing, and its slot.
struct OptionsBlock {
    start: usize,
    end: usize,
    head_end: usize,
    self_closing: bool,
    instance: u32,
}

impl OptionsBlock {
    /// The text between the opening tag and `</options>` — the children with
    /// their surrounding whitespace; empty for a self-closing element.
    fn inner<'a>(&self, xml: &'a str) -> &'a str {
        if self.self_closing {
            ""
        } else {
            &xml[self.head_end + 1..self.end - "</options>".len()]
        }
    }
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
        let head_end = tag_end(&masked, start, masked.len()).ok_or("unterminated <options> tag")?;
        let tag = &xml[start..=head_end];
        let self_closing = tag.ends_with("/>");
        let end = if self_closing {
            head_end + 1
        } else {
            let close = masked[head_end..].find("</options>").ok_or("<options> without </options>")?;
            head_end + close + "</options>".len()
        };
        if attr(tag, "type") == Some("joystick") {
            let instance = attr(tag, "instance")
                .ok_or("joystick <options> without instance")?
                .parse::<u32>()
                .map_err(|e| format!("bad joystick <options> instance: {e}"))?;
            blocks.push(OptionsBlock { start, end, head_end, self_closing, instance });
        }
        pos = end;
    }
    Ok(blocks)
}

/// Swap the children of the joystick `<options>` elements of slots `a` and
/// `b`, in place; an element that ends up empty is written self-closing, one
/// that receives children is opened, the way the game writes them. A slot
/// without an element is fine as long as nothing has to move into it.
fn swap_options_children(xml: &str, a: u32, b: u32) -> Result<String, String> {
    let blocks = find_joystick_options(xml)?;
    if blocks.is_empty() {
        return Err("no <options type=\"joystick\"> in actionmaps.xml".into());
    }
    let find = |slot: u32| blocks.iter().filter(|x| x.instance == slot).collect::<Vec<_>>();
    let (ba, bb) = (find(a), find(b));
    if ba.len() > 1 || bb.len() > 1 {
        return Err(format!("more than one <options> element for js{a} or js{b}"));
    }
    let (ba, bb) = (ba.first().copied(), bb.first().copied());
    let inner_a = ba.map(|x| x.inner(xml)).unwrap_or("");
    let inner_b = bb.map(|x| x.inner(xml)).unwrap_or("");
    if ba.is_none() && !inner_b.is_empty() {
        return Err(format!("no <options> element for js{a} to take the settings of js{b}"));
    }
    if bb.is_none() && !inner_a.is_empty() {
        return Err(format!("no <options> element for js{b} to take the settings of js{a}"));
    }
    let mut edits: Vec<(&OptionsBlock, &str)> = Vec::new();
    if let Some(x) = ba {
        edits.push((x, inner_b));
    }
    if let Some(x) = bb {
        edits.push((x, inner_a));
    }
    edits.sort_by_key(|(x, _)| x.start);

    let mut out = String::with_capacity(xml.len());
    let mut last = 0;
    for (block, inner) in edits {
        out.push_str(&xml[last..block.start]);
        out.push_str(&with_inner(&xml[block.start..block.end], block.head_end - block.start, block.self_closing, inner));
        last = block.end;
    }
    out.push_str(&xml[last..]);
    Ok(out)
}

/// Re-emit one `<options>` element with `inner` as its content: `<tag .../>`
/// when `inner` is empty, `<tag ...>` + inner + `</options>` otherwise.
fn with_inner(element: &str, head_end: usize, self_closing: bool, inner: &str) -> String {
    let head = &element[..=head_end];
    // The opening tag without its closing `/>` or `>`.
    let open = if self_closing { head[..head.len() - 2].trim_end() } else { &head[..head.len() - 1] };
    if inner.is_empty() {
        format!("{open}/>")
    } else {
        format!("{open}>{inner}</options>")
    }
}

/// After a swap, the children of `a` must be the former children of `b` and
/// vice versa, every other joystick element byte for byte the same.
fn verify_children_swapped(before: &str, after: &str, a: u32, b: u32) -> Result<(), String> {
    let (x, y) = (find_joystick_options(before)?, find_joystick_options(after)?);
    if x.len() != y.len() {
        return Err("rewrite check failed: the number of joystick <options> elements changed".into());
    }
    for (p, q) in x.iter().zip(&y) {
        if p.instance != q.instance {
            return Err("rewrite check failed: the joystick <options> order changed".into());
        }
        let expected = match p.instance {
            n if n == a => x.iter().find(|o| o.instance == b).map(|o| o.inner(before)).unwrap_or(""),
            n if n == b => x.iter().find(|o| o.instance == a).map(|o| o.inner(before)).unwrap_or(""),
            _ => p.inner(before),
        };
        if q.inner(after) != expected {
            return Err(format!("rewrite check failed: the settings of js{} are not what the swap leaves", p.instance));
        }
    }
    Ok(())
}

/// Record a joystick device map in the `<options type="joystick">` elements:
/// slot `instance` of each `map` entry gets that raw `Product` string,
/// every other joystick element loses its `Product` — what the game writes
/// at its next save once it assigned the slots (see `order::assign`). The
/// element order, the `instance` attributes and the children stay; a slot
/// in `map` without an element is an error (the game keeps `js1`–`js8`),
/// and so is a product string that would need escaping. Re-parsed and
/// checked before it is returned.
pub fn rewrite_device_map(xml: &str, map: &[(u32, &str)]) -> Result<String, String> {
    for (slot, product) in map {
        if *slot == 0 {
            return Err("joystick slots start at js1".into());
        }
        if product.chars().any(|c| matches!(c, '"' | '<' | '>' | '&') || c.is_control()) {
            return Err(format!("js{slot}: the device name cannot be written as it is"));
        }
        if map.iter().filter(|(s, _)| s == slot).count() > 1 {
            return Err(format!("js{slot} listed twice"));
        }
    }
    let blocks = find_joystick_options(xml)?;
    if blocks.is_empty() {
        return Err("no <options type=\"joystick\"> in actionmaps.xml".into());
    }
    if let Some((slot, _)) = map.iter().find(|(s, _)| !blocks.iter().any(|b| b.instance == *s)) {
        return Err(format!("no <options> element for js{slot}"));
    }

    let mut out = String::with_capacity(xml.len());
    let mut last = 0;
    for block in &blocks {
        let head = &xml[block.start..=block.head_end];
        let new_head = match map.iter().find(|(s, _)| *s == block.instance) {
            Some((_, product)) => set_attr(head, "Product", product).unwrap_or_else(|| insert_attr(head, "Product", product)),
            None => remove_attr(head, "Product").unwrap_or_else(|| head.to_string()),
        };
        out.push_str(&xml[last..block.start]);
        out.push_str(&new_head);
        last = block.head_end + 1;
    }
    out.push_str(&xml[last..]);

    let after = parse_actionmaps(&out).map_err(|e| format!("rewrite produced unreadable XML: {e}"))?;
    let mut expected: Vec<(u32, String)> = map.iter().map(|(s, p)| (*s, (*p).to_string())).collect();
    expected.sort();
    let mut got: Vec<(u32, String)> = after.joysticks.iter().map(|j| (j.instance, j.product.clone())).collect();
    got.sort();
    if got != expected {
        return Err("rewrite check failed: the joystick device map is not what was asked".into());
    }
    let before = parse_actionmaps(xml)?;
    if before.rebinds.len() != after.rebinds.len() || before.rebinds.iter().zip(&after.rebinds).any(|(x, y)| x.input != y.input) {
        return Err("rewrite check failed: the rebinds changed".into());
    }
    Ok(out)
}

/// Swap the `jsA_` / `jsB_` prefixes of every token inside the `input`
/// attribute of every `<rebind>` (comments and CDATA skipped, either quote
/// style read).
fn swap_inputs(xml: &str, a: u32, b: u32) -> String {
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
        let name = head[1..].split(|c: char| c.is_whitespace() || c == '/' || c == '>').next().unwrap_or("");
        if name == "rebind" {
            if let Some(s) = find_attr(head, "input") {
                out.push_str(&xml[copied..start + s.value_start]);
                out.push_str(&swap_tokens(&head[s.value_start..s.value_end], a, b));
                copied = start + s.value_end;
            }
        }
        search = end + 1;
    }
    out.push_str(&xml[copied..]);
    out
}

/// `js2_button5+js2_x` -> with a swap 1<->2: `js1_button5+js1_x`. Anything
/// that is not `js<digits>_` is copied through.
fn swap_tokens(value: &str, a: u32, b: u32) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(i) = rest.find("js") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let digits = after.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 && after[digits..].starts_with('_') {
            if let Ok(n) = after[..digits].parse::<u32>() {
                let m = if n == a {
                    b
                } else if n == b {
                    a
                } else {
                    n
                };
                out.push_str(&format!("js{m}_"));
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
    fn swaps_slot_contents_and_leaves_the_device_map() {
        let out = rewrite_actionmaps(XML, &[(1, 2)]).unwrap();
        // The device map and the element order stay: R is still instance 1,
        // L still instance 2 — only the settings changed hands, and the
        // elements switched between self-closing and open the way SC writes
        // them.
        assert!(out.contains("  <options type=\"joystick\" instance=\"1\" Product=\" VKB R {0200231D-0000-0000-0000-504944564944}\"/>\r\n"));
        assert!(out.contains("  <options type=\"joystick\" instance=\"2\" Product=\" VKB L {0201231D-0000-0000-0000-504944564944}\">\r\n   <flight_move_roll invert=\"1\"/>\r\n  </options>\r\n"));
        let i_r = out.find("instance=\"1\" Product=\" VKB R").unwrap();
        let i_l = out.find("instance=\"2\" Product=\" VKB L").unwrap();
        assert!(i_r < i_l);
        // Inputs swap, including the blank that switches a default off and
        // the modifier combo.
        assert!(out.contains("<action name=\"v_eject\">\r\n    <rebind input=\"js2_ \"/>"));
        assert!(out.contains("<rebind input=\"js1_button5\"/>"));
        assert!(out.contains("<rebind input=\"js1_button1+js1_x\"/>"));
        // Untouched: other slots, other devices, keyboard/gamepad options, deviceoptions.
        assert!(out.contains("<rebind input=\"js12_rotz\"/>"));
        assert!(out.contains("<rebind input=\"kb1_space\"/>"));
        assert!(out.contains("<options type=\"keyboard\" instance=\"1\""));
        assert!(out.contains("<options type=\"gamepad\" instance=\"1\" Product=\"Controller (Gamepad)\">\r\n   <flight_view exponent=\"1\"/>\r\n  </options>"));
        assert!(out.contains("<options type=\"joystick\" instance=\"3\"/>"));
        assert!(out.contains("<deviceoptions name=\" VKB L {0201231D-0000-0000-0000-504944564944}\">"));
        // Line endings and structure survive; the same swap again restores
        // the file byte for byte.
        assert_eq!(out.matches("\r\n").count(), XML.matches("\r\n").count());
        assert_eq!(rewrite_actionmaps(&out, &[(1, 2)]).unwrap(), XML);
    }

    #[test]
    fn a_swap_chain_is_applied_in_order() {
        // `pp_resortdevices joystick 1 2` then `1 3`: R's settings and
        // v_eject's blank travel 1 -> 2, L's boost bindings 2 -> 1 -> 3.
        let out = rewrite_actionmaps(XML, &[(1, 2), (1, 3)]).unwrap();
        assert!(out.contains("instance=\"1\" Product=\" VKB R {0200231D-0000-0000-0000-504944564944}\"/>"));
        assert!(out.contains("instance=\"2\" Product=\" VKB L {0201231D-0000-0000-0000-504944564944}\">\r\n   <flight_move_roll invert=\"1\"/>"));
        assert!(out.contains("<options type=\"joystick\" instance=\"3\"/>"));
        assert!(out.contains("<action name=\"v_eject\">\r\n    <rebind input=\"js2_ \"/>"));
        assert!(out.contains("<rebind input=\"js3_button5\"/>"));
        assert!(out.contains("<rebind input=\"js3_button1+js3_x\"/>"));
    }

    #[test]
    fn a_slot_without_an_element_takes_no_settings() {
        // js3 has an (empty) element, js4 has none: swapping 3 and 4 moves
        // nothing and is fine; swapping 1 (settings) and 4 has nowhere to put
        // them.
        assert_eq!(rewrite_actionmaps(XML, &[(3, 4)]).unwrap(), XML);
        let err = rewrite_actionmaps(XML, &[(1, 4)]).unwrap_err();
        assert!(err.contains("js4"), "{err}");
        // Empty on both sides swaps only the tokens.
        let out = rewrite_actionmaps(XML, &[(2, 3)]).unwrap();
        assert!(out.contains("<rebind input=\"js3_button5\"/>"));
        assert!(out.contains("<options type=\"joystick\" instance=\"2\" Product=\" VKB L {0201231D-0000-0000-0000-504944564944}\"/>"));
        assert!(out.contains("<options type=\"joystick\" instance=\"3\"/>"));
    }

    #[test]
    fn comments_and_single_quotes_do_not_confuse_the_rewrite() {
        let xml = XML
            .replace(
                "  <options type=\"keyboard\" instance=\"1\"",
                "  <!-- <options type=\"joystick\" instance=\"1\" Product=\"ghost\"><x/></options> <rebind input=\"js1_ghost\"/> -->\r\n  <options type='joystick' instance='9' Product='Nine'/>\r\n  <options type=\"keyboard\" instance=\"1\"",
            )
            .replace("<rebind input=\"js2_button5\"/>", "<rebind input='js2_button5'/>");
        let out = rewrite_actionmaps(&xml, &[(1, 2), (9, 3)]).unwrap();
        assert!(out.contains("<!-- <options type=\"joystick\" instance=\"1\" Product=\"ghost\"><x/></options> <rebind input=\"js1_ghost\"/> -->"), "comment untouched");
        assert!(out.contains("<options type='joystick' instance='9' Product='Nine'/>"), "{out}");
        assert!(out.contains("<rebind input='js1_button5'/>"));
        // Single-quoted element receiving settings.
        let out = rewrite_actionmaps(&xml, &[(1, 9)]).unwrap();
        assert!(out.contains("<options type='joystick' instance='9' Product='Nine'>\r\n   <flight_move_roll invert=\"1\"/>\r\n  </options>"), "{out}");
        assert_eq!(rewrite_actionmaps(&out, &[(1, 9)]).unwrap(), xml);
    }

    #[test]
    fn verify_applied_catches_a_wrong_rewrite() {
        let before = parse_actionmaps(XML).unwrap();
        let good = rewrite_actionmaps(XML, &[(1, 2)]).unwrap();
        verify_applied(&before, &parse_actionmaps(&good).unwrap(), 1, 2).unwrap();
        // A token left behind on its old slot.
        let stale = good.replace("<rebind input=\"js1_button5\"/>", "<rebind input=\"js2_button5\"/>");
        assert!(verify_applied(&before, &parse_actionmaps(&stale).unwrap(), 1, 2).is_err());
        // A device that moved with the settings.
        let device = good.replace("instance=\"2\" Product=\" VKB L", "instance=\"3\" Product=\" VKB L");
        assert!(verify_applied(&before, &parse_actionmaps(&device).unwrap(), 1, 2).is_err());
        // A rebind lost on the way.
        let lost = good.replace("    <rebind input=\"kb1_space\"/>\r\n", "");
        assert!(verify_applied(&before, &parse_actionmaps(&lost).unwrap(), 1, 2).is_err());
        // Settings that did not change hands.
        assert!(verify_children_swapped(XML, XML, 1, 2).is_err());
        verify_children_swapped(XML, &good, 1, 2).unwrap();
    }

    #[test]
    fn rejects_bad_swaps() {
        assert!(rewrite_actionmaps(XML, &[]).is_err());
        assert!(rewrite_actionmaps(XML, &[(1, 1)]).is_err());
        assert!(rewrite_actionmaps(XML, &[(0, 1)]).is_err());
        assert!(rewrite_actionmaps("<ActionMaps/>", &[(1, 2)]).is_err());
        let twice = XML.replace("<options type=\"joystick\" instance=\"3\"/>", "<options type=\"joystick\" instance=\"2\"/>");
        assert!(rewrite_actionmaps(&twice, &[(1, 2)]).is_err());
    }

    #[test]
    fn token_swapping_is_precise() {
        assert_eq!(swap_tokens("js1_button1", 1, 2), "js2_button1");
        assert_eq!(swap_tokens("js1_button1+js2_hat1_up", 1, 2), "js2_button1+js1_hat1_up");
        assert_eq!(swap_tokens("js3_x", 1, 2), "js3_x"); // not in the swap
        assert_eq!(swap_tokens("js12_x", 1, 2), "js12_x"); // js12 is not js1
        assert_eq!(swap_tokens("kb1_js", 1, 2), "kb1_js"); // no digits/underscore
        assert_eq!(swap_tokens("js_button1", 1, 2), "js_button1");
        assert_eq!(swap_tokens("", 1, 2), "");
    }

    // --- device map ---------------------------------------------------------

    const MAP_XML: &str = "<ActionMaps>\r\n\
 <options type=\"joystick\" instance=\"1\" Product=\" VKBsim Gladiator EVO  L    {0201231D-0000-0000-0000-504944564944}\">\r\n\
  <flight_move_strafe_vertical invert=\"1\"/>\r\n\
 </options>\r\n\
 <options type=\"joystick\" instance=\"2\" Product=\"Keychron Link   {D0303434-0000-0000-0000-504944564944}\"/>\r\n\
 <options type=\"joystick\" instance=\"3\" Product=\"Keychron K2 HE  {0E213434-0000-0000-0000-504944564944}\"/>\r\n\
 <options type=\"joystick\" instance=\"4\" Product=\" VKBsim Gladiator EVO  R    {0200231D-0000-0000-0000-504944564944}\"/>\r\n\
 <options type=\"joystick\" instance=\"5\"/>\r\n\
 <!-- <options type=\"joystick\" instance=\"6\" Product=\"ghost\"/> -->\r\n\
 <actionmap name=\"spaceship_view\">\r\n\
  <action name=\"v_view_pitch\">\r\n\
   <rebind input=\"js4_y\"/>\r\n\
  </action>\r\n\
 </actionmap>\r\n\
</ActionMaps>\r\n";

    #[test]
    fn device_map_step_c_removes_a_device_and_closes_the_gap() {
        // The map the game wrote in step C (K2 HE gone): L1 Link2 R3, 4 empty.
        let out = rewrite_device_map(
            MAP_XML,
            &[
                (1, " VKBsim Gladiator EVO  L    {0201231D-0000-0000-0000-504944564944}"),
                (2, "Keychron Link   {D0303434-0000-0000-0000-504944564944}"),
                (3, " VKBsim Gladiator EVO  R    {0200231D-0000-0000-0000-504944564944}"),
            ],
        )
        .unwrap();
        assert!(out.contains("<options type=\"joystick\" instance=\"3\" Product=\" VKBsim Gladiator EVO  R    {0200231D-0000-0000-0000-504944564944}\"/>\r\n"));
        assert!(out.contains("<options type=\"joystick\" instance=\"4\"/>\r\n"));
        assert!(out.contains("<options type=\"joystick\" instance=\"5\"/>\r\n"));
        // Children, the comment, the rebinds and the line endings stay.
        // (The string continuations of MAP_XML drop the indentation.)
        assert!(out.contains("Product=\" VKBsim Gladiator EVO  L    {0201231D-0000-0000-0000-504944564944}\">\r\n<flight_move_strafe_vertical invert=\"1\"/>\r\n</options>"));
        assert!(out.contains("<!-- <options type=\"joystick\" instance=\"6\" Product=\"ghost\"/> -->"));
        assert!(out.contains("<rebind input=\"js4_y\"/>"));
        let parsed = parse_actionmaps(&out).unwrap();
        assert_eq!(parsed.joysticks.iter().map(|j| j.instance).collect::<Vec<_>>(), vec![1, 2, 3]);
    }

    #[test]
    fn device_map_step_d_fills_an_empty_slot() {
        // From C's map back to A's: K2 HE into 3, R to 4, and 5 gets one too.
        let c = rewrite_device_map(MAP_XML, &[(1, "L {1}"), (2, "Link {2}"), (3, "R {3}")]).unwrap();
        let d = rewrite_device_map(&c, &[(1, "L {1}"), (2, "Link {2}"), (3, "K2 {4}"), (4, "R {3}"), (5, "X {5}")]).unwrap();
        assert!(d.contains("<options type=\"joystick\" instance=\"4\" Product=\"R {3}\"/>\r\n"));
        assert!(d.contains("<options type=\"joystick\" instance=\"5\" Product=\"X {5}\"/>\r\n"));
        let parsed = parse_actionmaps(&d).unwrap();
        assert_eq!(parsed.joysticks.iter().map(|j| j.product.as_str()).collect::<Vec<_>>(), vec!["L {1}", "Link {2}", "K2 {4}", "R {3}", "X {5}"]);
        // Writing the map the file already has changes nothing.
        let same = rewrite_device_map(
            MAP_XML,
            &parse_actionmaps(MAP_XML).unwrap().joysticks.iter().map(|j| (j.instance, j.product.as_str())).collect::<Vec<_>>(),
        )
        .unwrap();
        assert_eq!(same, MAP_XML);
    }

    #[test]
    fn device_map_refuses_bad_input() {
        assert!(rewrite_device_map(MAP_XML, &[(0, "x")]).unwrap_err().contains("js1"));
        assert!(rewrite_device_map(MAP_XML, &[(6, "x")]).unwrap_err().contains("no <options> element for js6"));
        assert!(rewrite_device_map(MAP_XML, &[(1, "a"), (1, "b")]).unwrap_err().contains("twice"));
        for bad in ["a\"b", "a<b", "a&b", "a>b", "a\nb"] {
            assert!(rewrite_device_map(MAP_XML, &[(1, bad)]).is_err(), "{bad:?}");
        }
        assert!(rewrite_device_map("<ActionMaps/>", &[(1, "x")]).is_err());
        // Single quotes are read and kept.
        let sq = "<ActionMaps>\n <options type='joystick' instance='1' Product='old'/>\n <options type='joystick' instance='2'/>\n</ActionMaps>\n";
        let out = rewrite_device_map(sq, &[(2, "new")]).unwrap();
        assert_eq!(out, "<ActionMaps>\n <options type='joystick' instance='1'/>\n <options type='joystick' instance='2' Product=\"new\"/>\n</ActionMaps>\n");
    }
}
