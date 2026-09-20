//! Helpers for the textual, byte-preserving edits of `actionmaps.xml`
//! (`rebind.rs`, `resort.rs`, `binding_profiles.rs`).
//!
//! The editors search the text for tags and attributes, which is exactly
//! where a comment, a CDATA section or a processing instruction holding a
//! look-alike tag would send them astray. [`mask_markup`] returns a copy of
//! the text with those regions blanked out — same length, same byte offsets
//! — so every search runs on the mask and every edit on the original.
//! [`find_attr`] / [`attr`] / [`set_attr`] read and rewrite one attribute of a
//! tag head the way XML allows it to be written: double or single quotes,
//! whitespace around the `=`, any whitespace between attributes.

/// `xml` with every `<!-- -->`, `<![CDATA[ ]]>` and `<? ?>` region replaced
/// by spaces, byte for byte. An unterminated region is blanked to the end.
/// Byte offsets found in the result are valid in `xml`.
pub fn mask_markup(xml: &str) -> String {
    const REGIONS: [(&str, &str); 3] = [("<!--", "-->"), ("<![CDATA[", "]]>"), ("<?", "?>")];
    let mut out = xml.as_bytes().to_vec();
    let mut pos = 0;
    while let Some(rel) = xml[pos..].find('<') {
        let start = pos + rel;
        let Some((open, close)) = REGIONS.iter().find(|(open, _)| xml[start..].starts_with(open)) else {
            pos = start + 1;
            continue;
        };
        let body = start + open.len();
        let end = xml[body..].find(close).map_or(xml.len(), |i| body + i + close.len());
        out[start..end].fill(b' ');
        pos = end;
    }
    // Only ASCII bytes were replaced, so the result is still valid UTF-8.
    String::from_utf8(out).unwrap_or_else(|_| " ".repeat(xml.len()))
}

/// The byte offset of the `>` that ends the tag head starting at
/// `masked[start]` (a `<`), quotes honoured, searching no further than `to`.
pub fn tag_end(masked: &str, start: usize, to: usize) -> Option<usize> {
    let bytes = masked.as_bytes();
    let mut quote: Option<u8> = None;
    let mut i = start + 1;
    while i < to.min(bytes.len()) {
        let b = bytes[i];
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None if b == b'"' || b == b'\'' => quote = Some(b),
            None if b == b'>' => return Some(i),
            None => {}
        }
        i += 1;
    }
    None
}

/// Where an attribute's value sits inside a tag head (offsets into the head,
/// quotes excluded).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AttrSpan {
    pub value_start: usize,
    pub value_end: usize,
}

/// Locate `name = "…"` (or `'…'`) in a tag head `<tag …>` / `<tag …/>`.
pub fn find_attr(head: &str, name: &str) -> Option<AttrSpan> {
    let bytes = head.as_bytes();
    let is_name = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'_' | b':' | b'-' | b'.');
    // Skip `<` and the tag name.
    let mut i = 1;
    while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'/' && bytes[i] != b'>' {
        i += 1;
    }
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] == b'/' || bytes[i] == b'>' {
            return None;
        }
        let name_start = i;
        while i < bytes.len() && is_name(bytes[i]) {
            i += 1;
        }
        if i == name_start {
            return None;
        }
        let this = &head[name_start..i];
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            return None;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || (bytes[i] != b'"' && bytes[i] != b'\'') {
            return None;
        }
        let quote = bytes[i];
        let value_start = i + 1;
        let value_end = head[value_start..].find(quote as char)? + value_start;
        if this == name {
            return Some(AttrSpan { value_start, value_end });
        }
        i = value_end + 1;
    }
}

/// The raw (still escaped) value of an attribute in a tag head.
pub fn attr<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    find_attr(head, name).map(|s| &head[s.value_start..s.value_end])
}

/// The tag head with the attribute's value replaced; `None` when the head
/// has no such attribute.
pub fn set_attr(head: &str, name: &str, value: &str) -> Option<String> {
    let s = find_attr(head, name)?;
    Some(format!("{}{value}{}", &head[..s.value_start], &head[s.value_end..]))
}

/// The tag head without the attribute (its name, `=`, quotes and value, plus
/// the whitespace before it); `None` when the head has no such attribute.
pub fn remove_attr(head: &str, name: &str) -> Option<String> {
    let s = find_attr(head, name)?;
    let bytes = head.as_bytes();
    // Back from the opening quote over `=` and whitespace to the name…
    let mut i = s.value_start - 1;
    while i > 0 && (bytes[i - 1].is_ascii_whitespace() || bytes[i - 1] == b'=') {
        i -= 1;
    }
    i = i.checked_sub(name.len())?;
    if &head[i..i + name.len()] != name {
        return None;
    }
    // …and over the whitespace in front of it.
    while i > 0 && bytes[i - 1].is_ascii_whitespace() {
        i -= 1;
    }
    Some(format!("{}{}", &head[..i], &head[s.value_end + 1..]))
}

/// The tag head with ` name="value"` appended after the last attribute
/// (before `/>` or `>`); `value` must already be attribute-safe.
pub fn insert_attr(head: &str, name: &str, value: &str) -> String {
    let trimmed = head.trim_end_matches('>');
    let (body, tail) = match trimmed.strip_suffix('/') {
        Some(body) => (body, "/>"),
        None => (trimmed, ">"),
    };
    format!("{} {name}=\"{value}\"{tail}", body.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remove_and_insert_attr() {
        assert_eq!(remove_attr(r#"<o type="joystick" instance="4" Product="x y"/>"#, "Product").as_deref(), Some(r#"<o type="joystick" instance="4"/>"#));
        assert_eq!(remove_attr(r#"<o Product = 'x' type="joystick">"#, "Product").as_deref(), Some(r#"<o type="joystick">"#));
        assert_eq!(remove_attr(r#"<o type="joystick"/>"#, "Product"), None);
        assert_eq!(insert_attr(r#"<o instance="4"/>"#, "Product", "x {G}"), r#"<o instance="4" Product="x {G}"/>"#);
        assert_eq!(insert_attr(r#"<o instance="4" >"#, "Product", "x"), r#"<o instance="4" Product="x">"#);
    }

    #[test]
    fn mask_blanks_comments_cdata_and_pis_keeping_offsets() {
        let xml = "<?xml version=\"1.0\"?>\n<a><!-- <b x=\"1\"/> ä --><![CDATA[<c/>]]><b/></a>";
        let m = mask_markup(xml);
        assert_eq!(m.len(), xml.len());
        assert_eq!(m.find("<b/>"), xml.rfind("<b/>"));
        assert!(!m.contains("<b x="));
        assert!(!m.contains("<c/>"));
        assert!(!m.contains("<?xml"));
        assert!(m.starts_with("                     \n<a>"));
        // Unterminated: blanked to the end, nothing after it is found.
        let m = mask_markup("<a/><!-- <b/>");
        assert_eq!(m, "<a/>         ");
        // Still valid text with multi-byte content around the regions.
        assert_eq!(mask_markup("ä<!--ö-->ü"), format!("ä{}ü", " ".repeat("<!--ö-->".len())));
    }

    #[test]
    fn tag_end_honours_quotes() {
        let s = "<rebind input=\"a>b\" x='c>d'/><next/>";
        assert_eq!(tag_end(s, 0, s.len()), Some(s.find("/>").unwrap() + 1));
        assert_eq!(tag_end("<a", 0, 2), None);
    }

    #[test]
    fn attributes_in_every_spelling_xml_allows() {
        let heads = [
            "<rebind input=\"js1_button1\"/>",
            "<rebind input='js1_button1'/>",
            "<rebind\tinput=\"js1_button1\" />",
            "<rebind input = \"js1_button1\"/>",
            "<rebind\n  activationMode=\"press\"\n  input=\"js1_button1\">",
            "<rebind activationMode='x\"y' input=\"js1_button1\"/>",
        ];
        for h in heads {
            assert_eq!(attr(h, "input"), Some("js1_button1"), "{h}");
            let set = set_attr(h, "input", "js2_x").unwrap();
            assert_eq!(attr(&set, "input"), Some("js2_x"), "{h}");
            assert_eq!(set.len(), h.len() - "js1_button1".len() + "js2_x".len());
        }
        assert_eq!(attr("<rebind inputs=\"x\" input=\"y\"/>", "input"), Some("y"));
        assert_eq!(attr("<rebind input=\"a>b\"/>", "input"), Some("a>b"));
        assert_eq!(attr("<rebind/>", "input"), None);
        assert_eq!(attr("<rebind input=js1_x/>", "input"), None, "unquoted is not read");
        assert_eq!(set_attr("<rebind/>", "input", "x"), None);
        // The tag name itself is never an attribute.
        assert_eq!(attr("<input input=\"y\"/>", "input"), Some("y"));
    }
}
