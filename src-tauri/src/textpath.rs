//! Text -> SVG path data for the image-map editor's text tool.
//!
//! A text placed on an image-map is stored as a `path` shape (see
//! `imagemap::Geometry::Path`): the glyph outlines of a font installed on
//! this machine, rendered into path data once in the editor. The map then
//! carries no text and needs no font anywhere it is drawn, so any system
//! font will do (`list_fonts` names them; the system database is scanned
//! once on first use).
//!
//! The path is normalized into the symbol box: the text's line box (advance
//! width by ascender-to-descender, lines stacked by the font's line height)
//! maps to 0..100 on both axes, and `aspect` tells the editor how wide the
//! shape is per unit of height. Glyphs the font lacks are skipped.

use std::sync::OnceLock;

use fontdb::{Database, Family, Query, Weight};
use serde::Serialize;
use ttf_parser::{Face, GlyphId, OutlineBuilder};

/// The longest text the tool accepts, in characters.
pub const MAX_TEXT_LEN: usize = 256;

#[derive(Debug, Clone, Serialize)]
pub struct TextPath {
    /// SVG path data in a 100 x 100 box (absolute commands, 2 decimals).
    pub d: String,
    /// Width per unit of height of the text's line box.
    pub aspect: f64,
}

fn fonts() -> &'static Database {
    static DB: OnceLock<Database> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = Database::new();
        db.load_system_fonts();
        log::info!("fonts: {} face(s) on this system", db.len());
        db
    })
}

/// The font families installed on this machine, sorted, without repeats.
pub fn font_families() -> Vec<String> {
    let mut out: Vec<String> = fonts()
        .faces()
        .filter_map(|f| f.families.first().map(|(name, _)| name.clone()))
        .collect();
    out.sort_by_key(|a| a.to_lowercase());
    out.dedup();
    out
}

/// Collects one glyph's outline in font units, translated by (`dx`, `dy`)
/// with the y axis flipped (fonts point up, SVG down).
struct Collector<'a> {
    out: &'a mut Vec<Seg>,
    dx: f64,
    dy: f64,
}

#[derive(Debug, Clone, Copy)]
enum Seg {
    M(f64, f64),
    L(f64, f64),
    Q(f64, f64, f64, f64),
    C(f64, f64, f64, f64, f64, f64),
    Z,
}

impl Collector<'_> {
    fn p(&self, x: f32, y: f32) -> (f64, f64) {
        (self.dx + f64::from(x), self.dy - f64::from(y))
    }
}

impl OutlineBuilder for Collector<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.p(x, y);
        self.out.push(Seg::M(x, y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.p(x, y);
        self.out.push(Seg::L(x, y));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (x1, y1) = self.p(x1, y1);
        let (x, y) = self.p(x, y);
        self.out.push(Seg::Q(x1, y1, x, y));
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (x1, y1) = self.p(x1, y1);
        let (x2, y2) = self.p(x2, y2);
        let (x, y) = self.p(x, y);
        self.out.push(Seg::C(x1, y1, x2, y2, x, y));
    }
    fn close(&mut self) {
        self.out.push(Seg::Z);
    }
}

/// The text as the tool takes it: control characters dropped (line breaks
/// kept), at most `MAX_TEXT_LEN` characters, not blank.
fn clean_text(text: &str) -> Result<String, String> {
    let cleaned: String = text.chars().filter(|c| *c == '\n' || !c.is_control()).collect();
    if cleaned.trim().is_empty() {
        return Err("text is empty".into());
    }
    if cleaned.chars().count() > MAX_TEXT_LEN {
        return Err(format!("text longer than {MAX_TEXT_LEN} characters"));
    }
    Ok(cleaned)
}

/// Kerning between two glyphs from the font's `kern` table, if any.
fn kerning(face: &Face, left: GlyphId, right: GlyphId) -> f64 {
    let Some(kern) = face.tables().kern else { return 0.0 };
    for sub in kern.subtables {
        if !sub.horizontal || sub.variable {
            continue;
        }
        if let Some(k) = sub.glyphs_kerning(left, right) {
            return f64::from(k);
        }
    }
    0.0
}

/// The text's outline in `face`, normalized into the 100x100 box.
fn outline(face: &Face, text: &str) -> Result<TextPath, String> {
    let ascender = f64::from(face.ascender());
    let descender = f64::from(face.descender());
    let line_gap = f64::from(face.line_gap());
    let line_height = ascender - descender;
    let mut segs: Vec<Seg> = Vec::new();
    let mut width: f64 = 0.0;
    let mut lines = 0usize;
    for (i, line) in text.split('\n').enumerate() {
        lines = i + 1;
        let baseline = ascender + (i as f64) * (line_height + line_gap);
        let mut x = 0.0;
        let mut prev: Option<GlyphId> = None;
        for c in line.chars() {
            let Some(gid) = face.glyph_index(c) else { continue };
            if let Some(p) = prev {
                x += kerning(face, p, gid);
            }
            let mut collector = Collector { out: &mut segs, dx: x, dy: baseline };
            face.outline_glyph(gid, &mut collector);
            x += f64::from(face.glyph_hor_advance(gid).unwrap_or(0));
            prev = Some(gid);
        }
        width = width.max(x);
    }
    let height = (lines as f64) * line_height + (lines.saturating_sub(1) as f64) * line_gap;
    if width <= 0.0 || height <= 0.0 || segs.is_empty() {
        return Err("text has no outline in this font".into());
    }
    let sx = 100.0 / width;
    let sy = 100.0 / height;
    let mut d = String::with_capacity(segs.len() * 16);
    let f = |v: f64| -> String {
        let s = format!("{v:.2}");
        // Trim the decimals nobody needs (`12.00` -> `12`, `1.50` -> `1.5`).
        let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
        if s == "-0" { "0".into() } else { s }
    };
    for seg in &segs {
        match *seg {
            Seg::M(x, y) => d.push_str(&format!("M{} {}", f(x * sx), f(y * sy))),
            Seg::L(x, y) => d.push_str(&format!("L{} {}", f(x * sx), f(y * sy))),
            Seg::Q(x1, y1, x, y) => {
                d.push_str(&format!("Q{} {} {} {}", f(x1 * sx), f(y1 * sy), f(x * sx), f(y * sy)))
            }
            Seg::C(x1, y1, x2, y2, x, y) => d.push_str(&format!(
                "C{} {} {} {} {} {}",
                f(x1 * sx),
                f(y1 * sy),
                f(x2 * sx),
                f(y2 * sy),
                f(x * sx),
                f(y * sy)
            )),
            Seg::Z => d.push('Z'),
        }
    }
    Ok(TextPath { d, aspect: width / height })
}

/// `text` set in the system font `family` (bold when asked; fontdb picks
/// the closest weight the family has), as path data in the 100x100 box.
pub fn text_to_path(text: &str, family: &str, bold: bool) -> Result<TextPath, String> {
    let text = clean_text(text)?;
    if family.trim().is_empty() {
        return Err("no font chosen".into());
    }
    let db = fonts();
    let query = Query {
        families: &[Family::Name(family)],
        weight: if bold { Weight::BOLD } else { Weight::NORMAL },
        ..Query::default()
    };
    let id = db.query(&query).ok_or_else(|| format!("font {family:?} not found on this system"))?;
    db.with_face_data(id, |data, index| {
        let face = Face::parse(data, index).map_err(|e| format!("font {family:?}: {e}"))?;
        outline(&face, &text)
    })
    .ok_or_else(|| format!("font {family:?} could not be read"))?
}

#[tauri::command]
pub fn list_fonts() -> Vec<String> {
    font_families()
}

#[tauri::command]
pub fn text_path(text: String, family: String, bold: bool) -> Result<TextPath, String> {
    text_to_path(&text, &family, bold)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Some family this machine has whose regular face draws a digit; the
    /// tests are about the outline maths, not a particular font. `None`
    /// when the system has no usable font at all — the outline tests then
    /// have nothing to run on and pass vacuously.
    fn any_family() -> Option<String> {
        font_families().into_iter().find(|f| text_to_path("1", f, false).is_ok())
    }

    fn numbers(d: &str) -> Vec<f64> {
        d.split(|c: char| c.is_ascii_alphabetic() || c == ' ')
            .filter(|s| !s.is_empty())
            .map(|s| s.parse::<f64>().unwrap())
            .collect()
    }

    #[test]
    fn a_digit_renders_into_the_box() {
        let Some(family) = any_family() else { return };
        let p = text_to_path("1", &family, false).unwrap();
        assert!(p.d.starts_with('M'), "{family}: {}", p.d);
        assert!(p.d.contains('Z'));
        assert!(p.aspect > 0.1 && p.aspect < 1.5, "{family}: aspect {}", p.aspect);
        for v in numbers(&p.d) {
            assert!(v.is_finite() && (-50.0..=150.0).contains(&v), "{family}: {v} out of range");
        }
    }

    #[test]
    fn wider_text_has_a_larger_aspect() {
        let Some(family) = any_family() else { return };
        let one = text_to_path("1", &family, false).unwrap();
        let three = text_to_path("111", &family, false).unwrap();
        assert!(three.aspect > one.aspect * 2.5, "{} vs {}", one.aspect, three.aspect);
    }

    #[test]
    fn lines_stack_and_widen_by_the_longest() {
        let Some(family) = any_family() else { return };
        let one = text_to_path("AB", &family, false).unwrap();
        let two = text_to_path("AB\nA", &family, false).unwrap();
        assert!(two.aspect < one.aspect * 0.6, "{} vs {}", one.aspect, two.aspect);
        let ys: Vec<f64> = numbers(&two.d).chunks(2).map(|p| p[1]).collect();
        assert!(ys.iter().cloned().fold(0.0, f64::max) > 60.0, "second line below the first");
    }

    #[test]
    fn output_fills_the_box() {
        let Some(family) = any_family() else { return };
        let p = text_to_path("H", &family, false).unwrap();
        let xs: Vec<f64> = numbers(&p.d).chunks(2).map(|c| c[0]).collect();
        let ys: Vec<f64> = numbers(&p.d).chunks(2).map(|c| c[1]).collect();
        let (x0, x1) = (xs.iter().cloned().fold(f64::MAX, f64::min), xs.iter().cloned().fold(f64::MIN, f64::max));
        let (y0, y1) = (ys.iter().cloned().fold(f64::MAX, f64::min), ys.iter().cloned().fold(f64::MIN, f64::max));
        // Ink inside the line box, wider than a third of it.
        assert!(x0 >= -1.0 && x1 <= 101.0 && x1 - x0 > 33.0, "x {x0}..{x1}");
        assert!(y0 >= -1.0 && y1 <= 101.0 && y1 - y0 > 33.0, "y {y0}..{y1}");
    }

    #[test]
    fn path_data_uses_only_svg_path_characters() {
        let Some(family) = any_family() else { return };
        let p = text_to_path("Boost 1/2 (gear)", &family, false).unwrap();
        assert!(p.d.chars().all(|c| "MLQCZ0123456789.- ".contains(c)), "{}", p.d);
    }

    #[test]
    fn rejects_bad_input() {
        let family = any_family().unwrap_or_default();
        assert!(text_to_path("", &family, false).is_err());
        assert!(text_to_path("   ", &family, false).is_err());
        assert!(text_to_path("\u{1}\u{2}", &family, false).is_err());
        assert!(text_to_path(&"x".repeat(MAX_TEXT_LEN + 1), &family, false).is_err());
        assert!(text_to_path("1", "", false).is_err());
        assert!(text_to_path("1", "No Such Font Family 4711", false).is_err());
    }

    #[test]
    fn control_characters_are_dropped_not_drawn() {
        let Some(family) = any_family() else { return };
        let plain = text_to_path("A", &family, false).unwrap();
        let noisy = text_to_path("\u{7}A\t", &family, false).unwrap();
        assert_eq!(plain.d, noisy.d);
    }

    #[test]
    fn families_are_sorted_and_unique() {
        let f = font_families();
        let mut sorted = f.clone();
        sorted.sort_by_key(|a| a.to_lowercase());
        sorted.dedup();
        assert_eq!(f, sorted);
    }
}
