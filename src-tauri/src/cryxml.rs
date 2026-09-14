//! CryXmlB: CryEngine's binary XML, the form `defaultProfile.xml` and
//! `keybinding_localization.xml` take inside `Data.p4k`. Decoded back into
//! XML text (elements and attributes only, that is all the format holds).
//!
//! Ported from StarBreaker (https://github.com/diogotr7/StarBreaker) by
//! diogotr7, MIT licensed — see THIRD-PARTY-LICENSES.md. The text output is
//! byte for byte what its `starbreaker-cryxml` crate writes.

use std::fmt::Write;

use crate::p4k::Cursor;

const MAGIC: &[u8; 8] = b"CryXmlB\0";
/// Nine `u32` fields after the magic.
const HEADER_SIZE: usize = 36;

struct Node {
    tag: u32,
    attribute_count: usize,
    child_count: usize,
    first_attribute: usize,
    first_child: usize,
}

/// `true` if `data` starts with the CryXmlB magic (plain XML does not).
pub fn is_cryxmlb(data: &[u8]) -> bool {
    data.len() > MAGIC.len() && &data[..MAGIC.len()] == MAGIC
}

/// Decode a CryXmlB blob into XML text.
pub fn to_xml(data: &[u8]) -> Result<String, String> {
    if !is_cryxmlb(data) {
        return Err("not a CryXmlB file".into());
    }
    let mut r = Cursor::at(data, MAGIC.len());
    r.u32()?; // xml size
    r.u32()?; // node table position
    let node_count = r.u32()? as usize;
    r.u32()?; // attribute table position
    let attribute_count = r.u32()? as usize;
    r.u32()?; // child table position
    let child_count = r.u32()? as usize;
    r.u32()?; // string data position
    let string_size = r.u32()? as usize;
    debug_assert_eq!(r.position(), MAGIC.len() + HEADER_SIZE);

    let mut nodes = Vec::with_capacity(node_count.min(1 << 16));
    for _ in 0..node_count {
        let tag = r.u32()?;
        r.u32()?; // item type
        let attribute_count = r.u16()? as usize;
        let child_count = r.u16()? as usize;
        r.i32()?; // parent index
        let first_attribute = r.i32()?;
        let first_child = r.i32()?;
        r.i32()?; // reserved
        nodes.push(Node {
            tag,
            attribute_count,
            child_count,
            first_attribute: usize::try_from(first_attribute).map_err(|_| "negative attribute index")?,
            first_child: usize::try_from(first_child).map_err(|_| "negative child index")?,
        });
    }
    let mut children = Vec::with_capacity(child_count.min(1 << 16));
    for _ in 0..child_count {
        let idx = r.i32()?;
        children.push(usize::try_from(idx).map_err(|_| "negative child index")?);
    }
    let mut attributes = Vec::with_capacity(attribute_count.min(1 << 16));
    for _ in 0..attribute_count {
        attributes.push((r.u32()?, r.u32()?));
    }
    let strings = r.bytes(string_size)?;

    // Every index in range, every non-root node the child of exactly one
    // node and the root of none: then the walk from the root visits each
    // node at most once and cannot loop.
    if nodes.is_empty() {
        return Err("no nodes".into());
    }
    let mut parents = vec![0usize; nodes.len()];
    for n in &nodes {
        if n.first_attribute + n.attribute_count > attributes.len() {
            return Err("attribute index out of range".into());
        }
        if n.first_child + n.child_count > children.len() {
            return Err("child index out of range".into());
        }
        for &c in &children[n.first_child..n.first_child + n.child_count] {
            if c >= nodes.len() {
                return Err("child index out of range".into());
            }
            parents[c] += 1;
        }
    }
    if parents[0] != 0 || parents[1..].iter().any(|&p| p != 1) {
        return Err("node tree is not a tree".into());
    }

    let string_at = |offset: u32| -> &str {
        let rest = strings.get(offset as usize..).unwrap_or(&[]);
        let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        std::str::from_utf8(&rest[..end]).unwrap_or("")
    };

    // Iterative pre-order walk; `Close` writes the end tag after the children.
    enum Step {
        Open(usize, usize),
        Close(usize, usize),
    }
    let mut out = String::new();
    let mut stack = vec![Step::Open(0, 0)];
    while let Some(step) = stack.pop() {
        match step {
            Step::Open(i, depth) => {
                let node = &nodes[i];
                let tag = string_at(node.tag);
                let indent = "  ".repeat(depth);
                let _ = write!(out, "{indent}<{tag}");
                for &(key, value) in &attributes[node.first_attribute..node.first_attribute + node.attribute_count] {
                    let _ = write!(out, " {}=\"{}\"", string_at(key), escape_attr(string_at(value)));
                }
                if node.child_count == 0 {
                    out.push_str(" />\n");
                } else {
                    out.push_str(">\n");
                    stack.push(Step::Close(i, depth));
                    for &c in children[node.first_child..node.first_child + node.child_count].iter().rev() {
                        stack.push(Step::Open(c, depth + 1));
                    }
                }
            }
            Step::Close(i, depth) => {
                let _ = writeln!(out, "{}</{}>", "  ".repeat(depth), string_at(nodes[i].tag));
            }
        }
    }
    Ok(out)
}

fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const NODE_SIZE: usize = 28;
    const ATTRIBUTE_SIZE: usize = 8;

    struct TestNode {
        tag: &'static str,
        attributes: Vec<(&'static str, &'static str)>,
        children: Vec<usize>,
    }

    /// Build a CryXmlB blob from a node list (index 0 = root); the string
    /// pool is filled in first-use order.
    fn build(nodes: &[TestNode]) -> Vec<u8> {
        let mut strings: Vec<u8> = Vec::new();
        let mut intern = |s: &str| -> u32 {
            let offset = strings.len() as u32;
            strings.extend_from_slice(s.as_bytes());
            strings.push(0);
            offset
        };
        let mut node_bytes = Vec::new();
        let mut child_bytes = Vec::new();
        let mut attr_bytes = Vec::new();
        let mut attr_index = 0i32;
        let mut child_index = 0i32;
        for n in nodes {
            node_bytes.extend_from_slice(&intern(n.tag).to_le_bytes());
            node_bytes.extend_from_slice(&0u32.to_le_bytes());
            node_bytes.extend_from_slice(&(n.attributes.len() as u16).to_le_bytes());
            node_bytes.extend_from_slice(&(n.children.len() as u16).to_le_bytes());
            node_bytes.extend_from_slice(&(-1i32).to_le_bytes());
            node_bytes.extend_from_slice(&attr_index.to_le_bytes());
            node_bytes.extend_from_slice(&child_index.to_le_bytes());
            node_bytes.extend_from_slice(&0i32.to_le_bytes());
            for (k, v) in &n.attributes {
                attr_bytes.extend_from_slice(&intern(k).to_le_bytes());
                attr_bytes.extend_from_slice(&intern(v).to_le_bytes());
            }
            for &c in &n.children {
                child_bytes.extend_from_slice(&(c as i32).to_le_bytes());
            }
            attr_index += n.attributes.len() as i32;
            child_index += n.children.len() as i32;
        }
        let mut out = MAGIC.to_vec();
        let node_pos = 8 + HEADER_SIZE as u32;
        let child_pos = node_pos + node_bytes.len() as u32;
        let attr_pos = child_pos + child_bytes.len() as u32;
        let string_pos = attr_pos + attr_bytes.len() as u32;
        let total = string_pos + strings.len() as u32;
        for v in [
            total,
            node_pos,
            nodes.len() as u32,
            attr_pos,
            (attr_bytes.len() / ATTRIBUTE_SIZE) as u32,
            child_pos,
            (child_bytes.len() / 4) as u32,
            string_pos,
            strings.len() as u32,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&node_bytes);
        out.extend_from_slice(&child_bytes);
        out.extend_from_slice(&attr_bytes);
        out.extend_from_slice(&strings);
        out
    }

    fn profile() -> Vec<TestNode> {
        vec![
            TestNode { tag: "profile", attributes: vec![("version", "1")], children: vec![1, 3] },
            TestNode { tag: "actionmap", attributes: vec![("name", "spaceship_view")], children: vec![2] },
            TestNode {
                tag: "action",
                attributes: vec![("name", "v_view_zoom"), ("UILabel", "@ui_CIZoom"), ("joystick", "")],
                children: vec![],
            },
            TestNode { tag: "empty", attributes: vec![], children: vec![] },
        ]
    }

    #[test]
    fn decodes_a_tree_in_starbreaker_layout() {
        let xml = to_xml(&build(&profile())).unwrap();
        assert_eq!(
            xml,
            "<profile version=\"1\">\n\
             \x20 <actionmap name=\"spaceship_view\">\n\
             \x20   <action name=\"v_view_zoom\" UILabel=\"@ui_CIZoom\" joystick=\"\" />\n\
             \x20 </actionmap>\n\
             \x20 <empty />\n\
             </profile>\n"
        );
    }

    #[test]
    fn escapes_attribute_values_only() {
        let nodes = vec![TestNode { tag: "a", attributes: vec![("v", "<1 & \"2\" '3'>")], children: vec![] }];
        assert_eq!(to_xml(&build(&nodes)).unwrap(), "<a v=\"&lt;1 &amp; &quot;2&quot; &apos;3&apos;&gt;\" />\n");
    }

    #[test]
    fn plain_xml_is_not_cryxmlb() {
        assert!(!is_cryxmlb(b"<?xml version=\"1.0\"?><a/>"));
        assert!(!is_cryxmlb(b"CryXmlB\0"));
        assert!(is_cryxmlb(&build(&profile())));
        assert!(to_xml(b"<a/>").unwrap_err().contains("not a CryXmlB"));
    }

    #[test]
    fn broken_blobs_are_errors_not_panics() {
        let good = build(&profile());
        // Truncated anywhere.
        for len in (MAGIC.len()..good.len()).step_by(7) {
            assert!(to_xml(&good[..len]).is_err(), "truncated at {len}");
        }
        // A child index pointing past the nodes.
        let mut bad = build(&profile());
        let child_table = 8 + HEADER_SIZE + 4 * NODE_SIZE;
        bad[child_table..child_table + 4].copy_from_slice(&99i32.to_le_bytes());
        assert!(to_xml(&bad).unwrap_err().contains("out of range"));
        // A cycle: the root's first child is the root.
        let mut bad = build(&profile());
        bad[child_table..child_table + 4].copy_from_slice(&0i32.to_le_bytes());
        assert!(to_xml(&bad).unwrap_err().contains("not a tree"));
        // Attribute count past the table.
        let mut bad = build(&profile());
        let root_attr_count = 8 + HEADER_SIZE + 8;
        bad[root_attr_count..root_attr_count + 2].copy_from_slice(&50u16.to_le_bytes());
        assert!(to_xml(&bad).unwrap_err().contains("out of range"));
        // A string offset past the pool reads as empty, no panic.
        let mut odd = build(&profile());
        let root_tag = 8 + HEADER_SIZE;
        odd[root_tag..root_tag + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(to_xml(&odd).unwrap().starts_with("< version=\"1\">"));
    }

    #[test]
    fn deep_trees_do_not_overflow_the_stack() {
        // Each level indents two more spaces, so keep the output sane.
        let depth = 2_000;
        let mut nodes: Vec<TestNode> = (0..depth)
            .map(|i| TestNode { tag: "n", attributes: vec![], children: vec![i + 1] })
            .collect();
        nodes.push(TestNode { tag: "leaf", attributes: vec![], children: vec![] });
        let xml = to_xml(&build(&nodes)).unwrap();
        assert!(xml.contains("<leaf />"));
        assert_eq!(xml.matches("</n>").count(), depth);
    }
}
