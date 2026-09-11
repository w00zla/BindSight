//! User-given names (image-maps, binding profiles, device names later): a
//! character set that needs no escaping in a file name or URL. Mirrored by
//! `src/names.ts`.

/// Characters a user-given name may contain: letters, digits, space, `_`,
/// `-` and brackets of any kind — nothing that needs escaping in a file
/// name or a URL. The same rule is meant for device names later on.
pub fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '-' | '(' | ')' | '[' | ']' | '{' | '}')
}

pub const NAME_MAX: usize = 64;

/// A valid name: only name characters, no leading / trailing space, not
/// empty, at most `NAME_MAX` characters.
pub fn is_safe_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= NAME_MAX && s.trim() == s && s.chars().all(is_name_char)
}

/// Make any string a valid name: other characters become spaces, runs of
/// spaces collapse, the ends are trimmed, the length capped; an empty
/// result falls back to `fallback`.
pub fn sanitize_name(s: &str, fallback: &str) -> String {
    let mut out = String::new();
    let mut space = true;
    for c in s.chars() {
        let c = if is_name_char(c) { c } else { ' ' };
        if c == ' ' {
            if !space {
                out.push(' ');
            }
            space = true;
        } else {
            out.push(c);
            space = false;
        }
        if out.len() >= NAME_MAX {
            break;
        }
    }
    let out = out.trim_end().to_string();
    if out.is_empty() {
        fallback.to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_name_makes_names_safe() {
        assert_eq!(sanitize_name("  VKB-Sim  Gladiator/NXT (EVO) ", "x"), "VKB-Sim Gladiator NXT (EVO)");
        assert_eq!(sanitize_name("Keyboard/Mouse", "x"), "Keyboard Mouse");
        assert_eq!(sanitize_name("///", "fallback"), "fallback");
        assert_eq!(sanitize_name("", "fallback"), "fallback");
        assert!(sanitize_name(&"ab ".repeat(40), "x").len() <= NAME_MAX);
        assert!(is_safe_name(&sanitize_name("café & co", "x")));
    }
}
