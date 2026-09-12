//! Image-maps: one image of a physical device plus drawn shapes that map an
//! input to a region of it, so the Monitor can light up the physical
//! control. Joysticks use SDL-level keys (`button:5`, `hat:0:up`, `axis:2`);
//! keyboard and gamepad use SC's own names (`key:lshift`, `pad:a`).
//!
//! On disk an image-map is one folder — `imagemap.json` plus the image file it
//! references by bare file name. Two roots are searched:
//!
//! - bundled: `resources/imagemaps/<id>/` (shipped with the app, read-only),
//! - user: `<app_data_dir>/imagemaps/<id>/` (everything the editor writes).
//!
//! Bundled image-maps are read-only: `save`, `add_image`, `remove_image` and
//! `delete` all refuse a bundled id. `clone_map` makes an editable copy in the
//! user root under a fresh id. A user image-map with the same id as a bundled
//! one would shadow it in `list`/`get`, but ids are always freshly generated
//! (`create`, `clone_map`, `import`), so that never happens in practice.
//!
//! Export/import is a plain zip with `imagemap.json` and the image at the
//! root. Import refuses entries with path separators, so a zip can never
//! write outside its image-map folder.
//!
//! The pure logic works on `&Path` roots so it is testable without an
//! `AppHandle`; the `#[tauri::command]` wrappers only resolve the roots.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use base64::Engine;
use log::{error, warn};
use serde::{Deserialize, Serialize};
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Manager, State};

use crate::names::{is_safe_name, sanitize_name};
use crate::{config, AppData};

/// Current imagemap.json format. Older ones (1: several `images`; 2: square
/// `size` symbols, `variant`; 3: `areas` with a nested `shape`) are not read.
pub const FORMAT: u32 = 4;

const MAP_FILE: &str = "imagemap.json";

/// The image of the device — mandatory, an image-map is nothing without it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageFile {
    /// Bare file name inside the image-map folder.
    pub file: String,
    #[serde(default)]
    pub label: String,
}

/// Shape geometry, normalized 0..1 relative to the image's natural size
/// (`x` / `w` against the width, `y` / `h` against the height; radii against
/// the width). `rotation` is degrees clockwise around the shape's own center
/// — for an arc or wedge, where its sweep starts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Geometry {
    Rect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        #[serde(default)]
        rotation: f64,
        /// Corner radius as a fraction of the shorter side (0..0.5).
        #[serde(default)]
        radius: f64,
    },
    Ellipse {
        cx: f64,
        cy: f64,
        rx: f64,
        ry: f64,
        #[serde(default)]
        rotation: f64,
    },
    Polygon {
        points: Vec<[f64; 2]>,
    },
    Symbol {
        /// `arrow`, `arrow2` or `rotate`.
        symbol: String,
        /// Center.
        x: f64,
        y: f64,
        /// Box the 100x100 symbol path is stretched into (may be non-square).
        w: f64,
        h: f64,
        #[serde(default)]
        rotation: f64,
    },
    /// A ring segment: `angle` degrees of sweep, starting at `rotation`.
    Arc {
        cx: f64,
        cy: f64,
        /// Outer radius.
        r: f64,
        /// Inner radius as a fraction of the outer (0..1).
        inner: f64,
        angle: f64,
        #[serde(default)]
        rotation: f64,
    },
    /// A pie slice: `angle` degrees of sweep, starting at `rotation`.
    Wedge {
        cx: f64,
        cy: f64,
        r: f64,
        angle: f64,
        #[serde(default)]
        rotation: f64,
    },
    /// An image file of its own in the image-map folder, centered at x/y.
    Image {
        file: String,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        #[serde(default)]
        rotation: f64,
    },
}

impl Geometry {
    /// The extra image file an image shape needs, if any.
    fn file(&self) -> Option<&str> {
        match self {
            Geometry::Image { file, .. } => Some(file),
            _ => None,
        }
    }
}

/// A drawn shape, tied to one input key. Lit on the image while its input
/// is active; `stroke` / `fill` (CSS `#rrggbb` or `#rrggbbaa`) override the
/// app's default lit colours.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Shape {
    pub id: String,
    /// `button:<n>`, `hat:<n>:<dir>` or `axis:<n>` (joystick, SDL-level);
    /// `key:<name>` (keyboard) or `pad:<name>` (gamepad), SC's own names.
    pub input: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    pub geometry: Geometry,
}

/// The full `imagemap.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageMap {
    pub format: u32,
    pub id: String,
    pub name: String,
    /// SC Product GUID (with braces) for a joystick, or the literal
    /// `gamepad` / `keyboard`; compared case-insensitively.
    pub hardware_id: String,
    #[serde(default)]
    pub hardware_name: String,
    pub image: ImageFile,
    #[serde(default)]
    pub shapes: Vec<Shape>,
}

impl ImageMap {
    /// Every file the image-map references: the image, then the image
    /// shapes' files, without repeats.
    fn files(&self) -> Vec<&str> {
        let mut out = vec![self.image.file.as_str()];
        for f in self.shapes.iter().filter_map(|s| s.geometry.file()) {
            if !out.contains(&f) {
                out.push(f);
            }
        }
        out
    }
}

/// Where a listed image-map was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageMapSource {
    Bundled,
    User,
}

/// Listing entry — everything the UI needs to pick an image-map without
/// loading its image.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageMapSummary {
    pub id: String,
    pub name: String,
    pub hardware_id: String,
    pub hardware_name: String,
    pub source: ImageMapSource,
    pub shape_count: usize,
}

impl ImageMapSummary {
    fn of(p: &ImageMap, source: ImageMapSource) -> Self {
        Self {
            id: p.id.clone(),
            name: p.name.clone(),
            hardware_id: p.hardware_id.clone(),
            hardware_name: p.hardware_name.clone(),
            source,
            shape_count: p.shapes.len(),
        }
    }
}

// ---------------------------------------------------------------------------
// Pure logic
// ---------------------------------------------------------------------------

/// True for a plain file name that cannot escape its folder: non-empty, no
/// path separators, not `.`/`..`.
fn is_bare_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
}

/// Structural validation (no filesystem access).
pub fn validate(p: &ImageMap) -> Result<(), String> {
    if p.format != FORMAT {
        return Err(format!("unsupported image-map format {} (expected {FORMAT})", p.format));
    }
    if p.id.trim().is_empty() {
        return Err("image-map id is empty".into());
    }
    if !is_bare_name(&p.id) {
        return Err(format!("invalid image-map id {:?}", p.id));
    }
    if p.name.trim().is_empty() {
        return Err("image-map name is empty".into());
    }
    if !is_safe_name(&p.name) {
        return Err(format!("invalid image-map name {:?} (letters, digits, space, _ - and brackets only)", p.name));
    }
    if p.hardware_id.trim().is_empty() {
        return Err("hardware id is empty".into());
    }
    if !is_bare_name(&p.image.file) || !is_image_name(&p.image.file) {
        return Err(format!("invalid image file name {:?}", p.image.file));
    }
    let mut ids = std::collections::HashSet::new();
    for sh in &p.shapes {
        if sh.id.trim().is_empty() || sh.input.trim().is_empty() {
            return Err("shape without id or input".into());
        }
        if !ids.insert(sh.id.as_str()) {
            return Err(format!("duplicate shape id {:?}", sh.id));
        }
        for c in [&sh.stroke, &sh.fill].into_iter().flatten() {
            if !is_colour(c) {
                return Err(format!("invalid colour {c:?} on shape {:?}", sh.id));
            }
        }
        if let Some(f) = sh.geometry.file() {
            if !is_bare_name(f) || !is_image_name(f) {
                return Err(format!("invalid image file name {f:?} on shape {:?}", sh.id));
            }
        }
    }
    Ok(())
}

/// Image files carry one of the extensions the webview can show.
const IMAGE_EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "webp", "svg", "gif"];

fn is_image_name(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(stem, ext)| !stem.is_empty() && IMAGE_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
}

/// `#rrggbb` or `#rrggbbaa`.
fn is_colour(c: &str) -> bool {
    let hex = match c.strip_prefix('#') {
        Some(h) => h,
        None => return false,
    };
    (hex.len() == 6 || hex.len() == 8) && hex.chars().all(|ch| ch.is_ascii_hexdigit())
}

/// Check that every referenced file exists in `dir`.
fn validate_files(p: &ImageMap, dir: &Path) -> Result<(), String> {
    for f in p.files() {
        if !dir.join(f).is_file() {
            return Err(format!("image file {f:?} is missing"));
        }
    }
    Ok(())
}

/// Drop image files in `dir` that `p` no longer references (an image shape
/// deleted, an image swapped). Only image files are touched.
fn prune_files(p: &ImageMap, dir: &Path) {
    let keep = p.files();
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if name == MAP_FILE || mime_for(&name).is_none() || keep.contains(&name.as_str()) {
            continue;
        }
        if let Err(e) = fs::remove_file(entry.path()) {
            warn!("could not remove orphaned {}: {e}", entry.path().display());
        }
    }
}

fn read_map(dir: &Path) -> Result<ImageMap, String> {
    let path = dir.join(MAP_FILE);
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn write_map(dir: &Path, p: &ImageMap) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(p).map_err(|e| e.to_string())?;
    fs::write(dir.join(MAP_FILE), json).map_err(|e| e.to_string())
}

/// All readable image-maps directly under `root` (one folder each). Broken
/// folders are logged and skipped.
fn read_root(root: &Path) -> Vec<ImageMap> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.join(MAP_FILE).is_file() {
            continue;
        }
        match read_map(&dir) {
            Ok(p) => out.push(p),
            Err(e) => warn!("skipping image-map {}: {e}", dir.display()),
        }
    }
    out
}

/// Bundled + user image-maps, user shadowing bundled by id. Bundled first,
/// then by name — so the first match for a device is the shipped one.
pub fn list(bundled_root: &Path, user_root: &Path) -> Vec<ImageMapSummary> {
    let mut by_id: HashMap<String, ImageMapSummary> = HashMap::new();
    for p in read_root(bundled_root) {
        by_id.insert(p.id.clone(), ImageMapSummary::of(&p, ImageMapSource::Bundled));
    }
    for p in read_root(user_root) {
        by_id.insert(p.id.clone(), ImageMapSummary::of(&p, ImageMapSource::User));
    }
    let mut out: Vec<_> = by_id.into_values().collect();
    out.sort_by(|a, b| {
        let bundled_first = (a.source == ImageMapSource::User).cmp(&(b.source == ImageMapSource::User));
        bundled_first
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.id.cmp(&b.id))
    });
    out
}

/// The folder an image-map is read from: user first, then bundled.
fn find_dir(bundled_root: &Path, user_root: &Path, id: &str) -> Result<(PathBuf, ImageMapSource), String> {
    if !is_bare_name(id) {
        return Err(format!("invalid image-map id {id:?}"));
    }
    let user = user_root.join(id);
    if user.join(MAP_FILE).is_file() {
        return Ok((user, ImageMapSource::User));
    }
    let bundled = bundled_root.join(id);
    if bundled.join(MAP_FILE).is_file() {
        return Ok((bundled, ImageMapSource::Bundled));
    }
    Err(format!("unknown image-map {id:?}"))
}

/// The writable folder for an existing user image-map. Errors (without
/// creating anything) if `id` is bundled-only (read-only) or unknown.
fn writable_dir(bundled_root: &Path, user_root: &Path, id: &str) -> Result<PathBuf, String> {
    if !is_bare_name(id) {
        return Err(format!("invalid image-map id {id:?}"));
    }
    let user = user_root.join(id);
    if user.join(MAP_FILE).is_file() {
        return Ok(user);
    }
    if bundled_root.join(id).join(MAP_FILE).is_file() {
        return Err(format!("{id:?} is a bundled image-map (read-only)"));
    }
    Err(format!("unknown image-map {id:?}"))
}

pub fn get(bundled_root: &Path, user_root: &Path, id: &str) -> Result<ImageMap, String> {
    let (dir, _) = find_dir(bundled_root, user_root, id)?;
    read_map(&dir)
}

/// Create a user image-map around `image_source` (copied into the new
/// folder).
pub fn create(
    user_root: &Path,
    name: &str,
    hardware_id: &str,
    hardware_name: &str,
    image_source: &Path,
) -> Result<ImageMap, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let dir = user_root.join(&id);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let image = copy_image_into(&dir, image_source)?;
    let map = ImageMap {
        format: FORMAT,
        id,
        name: sanitize_name(name, "image-map"),
        hardware_id: hardware_id.to_string(),
        hardware_name: hardware_name.to_string(),
        image,
        shapes: Vec::new(),
    };
    validate(&map)?;
    write_map(&dir, &map)?;
    Ok(map)
}

/// Copy an image-map (bundled or user) into a new, editable user image-map
/// with a fresh id.
pub fn clone_map(bundled_root: &Path, user_root: &Path, id: &str, name: &str) -> Result<ImageMap, String> {
    let (src_dir, _) = find_dir(bundled_root, user_root, id)?;
    let source = read_map(&src_dir)?;
    validate_files(&source, &src_dir)?;

    let new_id = uuid::Uuid::new_v4().to_string();
    let map = ImageMap {
        format: FORMAT,
        id: new_id.clone(),
        name: sanitize_name(name, "copy"),
        hardware_id: source.hardware_id,
        hardware_name: source.hardware_name,
        image: source.image,
        shapes: source.shapes,
    };
    validate(&map)?;

    let dir = user_root.join(&new_id);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for f in map.files() {
        if let Err(e) = fs::copy(src_dir.join(f), dir.join(f)) {
            let _ = fs::remove_dir_all(&dir);
            return Err(format!("{f}: {e}"));
        }
    }
    write_map(&dir, &map)?;
    Ok(map)
}

/// Write the image-map; image files it no longer references are removed.
pub fn save(bundled_root: &Path, user_root: &Path, map: ImageMap) -> Result<ImageMap, String> {
    validate(&map)?;
    let dir = writable_dir(bundled_root, user_root, &map.id)?;
    validate_files(&map, &dir)?;
    write_map(&dir, &map)?;
    prune_files(&map, &dir);
    Ok(map)
}

/// Delete the user copy. Bundled image-maps are read-only and cannot be
/// deleted.
pub fn delete(bundled_root: &Path, user_root: &Path, id: &str) -> Result<(), String> {
    let dir = writable_dir(bundled_root, user_root, id)?;
    fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(())
}

fn mime_for(file: &str) -> Option<&'static str> {
    let ext = Path::new(file).extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// `base`, or `base-2`, `base-3`, … until `taken` says no.
fn unique(base: &str, mut taken: impl FnMut(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|c| !taken(c))
        .expect("unbounded counter")
}

/// Copy an image (a replacement device image, or one for an image shape)
/// into the image-map folder. Does not touch `imagemap.json` — the caller
/// references it; a file nothing references is pruned at the next save.
pub fn add_image(bundled_root: &Path, user_root: &Path, id: &str, source: &Path) -> Result<ImageFile, String> {
    let dir = writable_dir(bundled_root, user_root, id)?;
    copy_image_into(&dir, source)
}

/// Copy `source` into `dir` under its own (de-duplicated) file name.
fn copy_image_into(dir: &Path, source: &Path) -> Result<ImageFile, String> {
    let file_name = source
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("{}: not a file name", source.display()))?;
    if mime_for(file_name).is_none() {
        return Err(format!("{file_name}: unsupported image format (png/jpg/jpeg/webp)"));
    }
    if !source.is_file() {
        return Err(format!("{}: not a file", source.display()));
    }

    let stem = Path::new(file_name).file_stem().and_then(|s| s.to_str()).unwrap_or(file_name);
    let ext = Path::new(file_name).extension().and_then(|s| s.to_str()).unwrap_or("");
    let file = unique(stem, |s| dir.join(format!("{s}.{ext}")).exists());
    let file = format!("{file}.{ext}");

    fs::copy(source, dir.join(&file)).map_err(|e| e.to_string())?;
    Ok(ImageFile { file, label: stem.to_string() })
}

/// Delete an image file from the user folder; missing file is a no-op.
pub fn remove_image(bundled_root: &Path, user_root: &Path, id: &str, file: &str) -> Result<(), String> {
    if !is_bare_name(file) {
        return Err("invalid name".into());
    }
    let dir = writable_dir(bundled_root, user_root, id)?;
    match fs::remove_file(dir.join(file)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// An image as a `data:` URL.
pub fn read_image(bundled_root: &Path, user_root: &Path, id: &str, file: &str) -> Result<String, String> {
    if !is_bare_name(file) {
        return Err(format!("invalid image file name {file:?}"));
    }
    let mime = mime_for(file).ok_or_else(|| format!("{file}: unsupported image format"))?;
    let (dir, _) = find_dir(bundled_root, user_root, id)?;
    let bytes = fs::read(dir.join(file)).map_err(|e| format!("{file}: {e}"))?;
    Ok(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

/// Zip `imagemap.json` + every referenced image (flat, deflate) to `dest`.
pub fn export(bundled_root: &Path, user_root: &Path, id: &str, dest: &Path) -> Result<(), String> {
    let (dir, _) = find_dir(bundled_root, user_root, id)?;
    let map = read_map(&dir)?;
    validate(&map)?;
    validate_files(&map, &dir)?;

    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut zip = zip::ZipWriter::new(File::create(dest).map_err(|e| format!("{}: {e}", dest.display()))?);
    for name in std::iter::once(MAP_FILE).chain(map.files()) {
        zip.start_file(name, opts).map_err(|e| e.to_string())?;
        let bytes = fs::read(dir.join(name)).map_err(|e| format!("{name}: {e}"))?;
        zip.write_all(&bytes).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

/// Unzip into a new user image-map under a fresh id (the id in the zip is
/// ignored), so an import never collides with a bundled or existing map.
pub fn import(user_root: &Path, source: &Path) -> Result<ImageMapSummary, String> {
    let file = File::open(source).map_err(|e| format!("{}: {e}", source.display()))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;

    // Pass 1: collect file entries, refusing anything that is not a bare name.
    let mut entries = Vec::new();
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| e.to_string())?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        if !is_bare_name(&name) {
            return Err(format!("refusing zip entry {name:?}"));
        }
        entries.push((i, name));
    }
    let &(json_idx, _) = entries
        .iter()
        .find(|(_, n)| n == MAP_FILE)
        .ok_or("zip contains no imagemap.json")?;
    let mut text = String::new();
    archive
        .by_index(json_idx)
        .map_err(|e| e.to_string())?
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    let mut map: ImageMap = serde_json::from_str(&text).map_err(|e| format!("imagemap.json: {e}"))?;
    // A name from elsewhere is made valid rather than refused.
    map.name = sanitize_name(&map.name, "image-map");
    validate(&map)?;
    for f in map.files() {
        if !entries.iter().any(|(_, n)| n == f) {
            return Err(format!("image file {f:?} is missing from the zip"));
        }
    }
    map.id = uuid::Uuid::new_v4().to_string();

    // Pass 2: extract only the files the map references (the json is
    // rewritten with the new id); anything else in the zip stays there.
    let dir = user_root.join(&map.id);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let wanted: Vec<String> = map.files().iter().map(|f| f.to_string()).collect();
    for (i, name) in entries.into_iter().filter(|(_, n)| wanted.contains(n)) {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let mut out = File::create(dir.join(&name)).map_err(|e| format!("{name}: {e}"))?;
        io::copy(&mut entry, &mut out).map_err(|e| format!("{name}: {e}"))?;
    }
    write_map(&dir, &map)?;
    Ok(ImageMapSummary::of(&map, ImageMapSource::User))
}

// ---------------------------------------------------------------------------
// Tauri commands
//
// Crate-visible: `generate_handler!` in lib.rs is the only caller, and
// `set_imagemap_choice` takes the crate-private `AppData` state anyway.
// ---------------------------------------------------------------------------

fn user_root(app: &AppHandle) -> Result<PathBuf, String> {
    app.path().app_data_dir().map(|d| d.join("imagemaps")).map_err(|e| e.to_string())
}

/// Bundled image-maps dir; falls back to the source tree in development,
/// where the bundled resources are absent (same as `read_resource` in
/// lib.rs).
fn bundled_root(app: &AppHandle) -> PathBuf {
    app.path()
        .resolve("resources/imagemaps", BaseDirectory::Resource)
        .ok()
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources").join("imagemaps"))
}

#[tauri::command]
pub(crate) fn list_imagemaps(app: AppHandle) -> Vec<ImageMapSummary> {
    match user_root(&app) {
        Ok(user) => list(&bundled_root(&app), &user),
        Err(e) => {
            error!("no app data dir: {e}");
            list(&bundled_root(&app), Path::new(""))
        }
    }
}

#[tauri::command]
pub(crate) fn get_imagemap(id: String, app: AppHandle) -> Result<ImageMap, String> {
    get(&bundled_root(&app), &user_root(&app)?, &id)
}

#[tauri::command]
pub(crate) fn create_imagemap(
    name: String,
    hardware_id: String,
    hardware_name: Option<String>,
    image_path: String,
    app: AppHandle,
) -> Result<ImageMap, String> {
    create(&user_root(&app)?, &name, &hardware_id, hardware_name.as_deref().unwrap_or(""), Path::new(&image_path))
}

#[tauri::command]
pub(crate) fn save_imagemap(map: ImageMap, app: AppHandle) -> Result<ImageMap, String> {
    save(&bundled_root(&app), &user_root(&app)?, map)
}

#[tauri::command]
pub(crate) fn delete_imagemap(id: String, app: AppHandle) -> Result<(), String> {
    delete(&bundled_root(&app), &user_root(&app)?, &id)
}

#[tauri::command]
pub(crate) fn clone_imagemap(id: String, name: String, app: AppHandle) -> Result<ImageMap, String> {
    clone_map(&bundled_root(&app), &user_root(&app)?, &id, &name)
}

#[tauri::command]
pub(crate) fn add_imagemap_image(id: String, source_path: String, app: AppHandle) -> Result<ImageFile, String> {
    add_image(&bundled_root(&app), &user_root(&app)?, &id, Path::new(&source_path))
}

#[tauri::command]
pub(crate) fn remove_imagemap_image(id: String, file: String, app: AppHandle) -> Result<(), String> {
    remove_image(&bundled_root(&app), &user_root(&app)?, &id, &file)
}

#[tauri::command]
pub(crate) fn read_imagemap_image(id: String, file: String, app: AppHandle) -> Result<String, String> {
    read_image(&bundled_root(&app), &user_root(&app)?, &id, &file)
}

#[tauri::command]
pub(crate) fn export_imagemap(id: String, dest_path: String, app: AppHandle) -> Result<(), String> {
    export(&bundled_root(&app), &user_root(&app)?, &id, Path::new(&dest_path))
}

#[tauri::command]
pub(crate) fn import_imagemap(source_path: String, app: AppHandle) -> Result<ImageMapSummary, String> {
    import(&user_root(&app)?, Path::new(&source_path))
}

/// Persist which image-map to show for a device (keyed by lowercase hardware
/// id); `None` clears the choice. Returns the stored config.
#[tauri::command]
pub(crate) fn set_imagemap_choice(
    hardware_id: String,
    imagemap_id: Option<String>,
    app: AppHandle,
    data: State<Mutex<AppData>>,
) -> config::Config {
    let mut data = data.lock().unwrap();
    let key = hardware_id.to_lowercase();
    match imagemap_id {
        Some(p) => {
            data.config.imagemap_choices.insert(key, p);
        }
        None => {
            data.config.imagemap_choices.remove(&key);
        }
    }
    if let Err(e) = config::save(&app, &data.config) {
        error!("failed to save config: {e}");
    }
    data.config.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::names::NAME_MAX;

    /// Fresh temp dir per test, removed on drop.
    struct Tmp(PathBuf);

    impl Tmp {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("bindsight-imagemap-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&dir).unwrap();
            Tmp(dir)
        }
        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    // Smallest valid-looking PNG header; the content never gets decoded.
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";

    fn sample(id: &str, name: &str) -> ImageMap {
        ImageMap {
            format: FORMAT,
            id: id.into(),
            name: name.into(),
            hardware_id: "{0200231D-0000-0000-0000-504944564944}".into(),
            hardware_name: "Test Stick".into(),
            image: ImageFile { file: "top.png".into(), label: "Top".into() },
            shapes: vec![rect_shape("a1", "button:5")],
        }
    }

    fn rect_shape(id: &str, input: &str) -> Shape {
        Shape {
            id: id.into(),
            input: input.into(),
            stroke: None,
            fill: None,
            geometry: Geometry::Rect { x: 0.1, y: 0.2, w: 0.05, h: 0.04, rotation: 0.0, radius: 0.0 },
        }
    }

    fn image_shape(id: &str, file: &str) -> Shape {
        Shape {
            id: id.into(),
            input: "button:1".into(),
            stroke: None,
            fill: None,
            geometry: Geometry::Image { file: file.into(), x: 0.5, y: 0.5, w: 0.1, h: 0.1, rotation: 0.0 },
        }
    }

    /// Write an image-map folder with a dummy image under `root`.
    fn put(root: &Path, p: &ImageMap) {
        let dir = root.join(&p.id);
        write_map(&dir, p).unwrap();
        for f in p.files() {
            fs::write(dir.join(f), PNG).unwrap();
        }
    }

    #[test]
    fn geometry_json_uses_kind_tag() {
        let json = r#"{"kind":"symbol","symbol":"arrow","x":0.3,"y":0.3,"w":0.05,"h":0.02,"rotation":90}"#;
        let g: Geometry = serde_json::from_str(json).unwrap();
        assert!(matches!(g, Geometry::Symbol { rotation, h, .. } if rotation == 90.0 && h == 0.02));
        let back = serde_json::to_value(&g).unwrap();
        assert_eq!(back["kind"], "symbol");
        // Polygon has no rotation; rect rotation and radius default.
        let r: Geometry = serde_json::from_str(r#"{"kind":"rect","x":0,"y":0,"w":1,"h":1}"#).unwrap();
        assert!(matches!(r, Geometry::Rect { rotation, radius, .. } if rotation == 0.0 && radius == 0.0));
        let a: Geometry =
            serde_json::from_str(r#"{"kind":"arc","cx":0.5,"cy":0.5,"r":0.1,"inner":0.6,"angle":270}"#).unwrap();
        assert!(matches!(a, Geometry::Arc { inner, angle, .. } if inner == 0.6 && angle == 270.0));
        // Unset colours are left out of the JSON.
        let sh = serde_json::to_value(rect_shape("a", "button:1")).unwrap();
        assert!(sh.get("stroke").is_none() && sh.get("fill").is_none());
        assert_eq!(sh["geometry"]["kind"], "rect");
    }

    #[test]
    fn validate_checks_shapes() {
        let mut p = sample("p1", "ok");
        p.shapes[0].stroke = Some("#396cd8".into());
        p.shapes[0].fill = Some("#396CD880".into());
        assert!(validate(&p).is_ok());
        for bad in ["396cd8", "#396cd", "#zzzzzz", "#396cd8800", "blue"] {
            p.shapes[0].fill = Some(bad.into());
            assert!(validate(&p).is_err(), "{bad:?} should be rejected");
        }
        let mut p = sample("p1", "ok");
        p.shapes.push(image_shape("i", "../x.png"));
        assert!(validate(&p).is_err());
        let mut p = sample("p1", "ok");
        p.shapes[0].input = " ".into();
        assert!(validate(&p).is_err());

        // Shape ids are unique.
        let mut p = sample("p1", "ok");
        p.shapes.push(rect_shape("a1", "button:6"));
        assert!(validate(&p).unwrap_err().contains("duplicate shape id"));

        // Image files carry an image extension, whatever the case.
        for ok in ["top.png", "TOP.PNG", "a.jpeg", "b.webp", "c.svg", "d.gif"] {
            let mut p = sample("p1", "ok");
            p.image.file = ok.into();
            assert!(validate(&p).is_ok(), "{ok:?} should pass");
        }
        for bad in ["top", "top.", ".png", "top.exe", "top.html", "top.png.txt"] {
            let mut p = sample("p1", "ok");
            p.image.file = bad.into();
            assert!(validate(&p).is_err(), "{bad:?} should be rejected");
        }
        let mut p = sample("p1", "ok");
        p.shapes.push(image_shape("i", "glyph.js"));
        assert!(validate(&p).is_err());
    }

    #[test]
    fn validate_rejects_bad_maps() {
        assert!(validate(&sample("p1", "ok")).is_ok());

        let mut p = sample("p1", "ok");
        p.format = 3;
        assert!(validate(&p).unwrap_err().contains("format"));

        let mut p = sample("p1", "ok");
        p.name = "  ".into();
        assert!(validate(&p).unwrap_err().contains("name"));
        for bad in ["My/Map", "a\\b", " lead", "trail ", "quote\"", "naïve", &"x".repeat(NAME_MAX + 1)] {
            let mut p = sample("p1", "ok");
            p.name = bad.to_string();
            assert!(validate(&p).is_err(), "{bad:?} should be rejected");
        }
        let mut p = sample("p1", "ok");
        p.name = "VKB Gladiator_NXT-EVO 2 (left) [v1] {x}".into();
        assert!(validate(&p).is_ok());

        let mut p = sample("p1", "ok");
        p.hardware_id.clear();
        assert!(validate(&p).unwrap_err().contains("hardware id"));

        let mut p = sample("../p1", "ok");
        assert!(validate(&p).unwrap_err().contains("image-map id"));
        p.id = "a/b".into();
        assert!(validate(&p).is_err());

        for bad in ["../x.png", "sub/x.png", "sub\\x.png", "..", ""] {
            let mut p = sample("p1", "ok");
            p.image.file = bad.into();
            assert!(validate(&p).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn user_shadows_bundled_in_listing() {
        let t = Tmp::new();
        let (bundled, user) = (t.path("bundled"), t.path("user"));
        put(&bundled, &sample("shared", "Bundled name"));
        put(&bundled, &sample("only-bundled", "Zeta"));
        let mut u = sample("shared", "User name");
        u.shapes.clear();
        put(&user, &u);

        let list = list(&bundled, &user);
        assert_eq!(list.len(), 2);
        let shared = list.iter().find(|s| s.id == "shared").unwrap();
        assert_eq!(shared.source, ImageMapSource::User);
        assert_eq!(shared.name, "User name");
        assert_eq!(shared.shape_count, 0);
        assert_eq!(list.iter().find(|s| s.id == "only-bundled").unwrap().source, ImageMapSource::Bundled);
        // Bundled first ("Zeta"), then user ("User name") — source beats name.
        assert_eq!(list[0].id, "only-bundled");
        assert_eq!(list[1].id, "shared");

        // get() prefers the user copy; missing user root is fine.
        assert_eq!(get(&bundled, &user, "shared").unwrap().name, "User name");
        assert_eq!(get(&bundled, &t.path("nope"), "shared").unwrap().name, "Bundled name");
        assert!(get(&bundled, &user, "missing").is_err());
    }

    #[test]
    fn bundled_maps_are_read_only() {
        let t = Tmp::new();
        let (bundled, user) = (t.path("bundled"), t.path("user"));
        put(&bundled, &sample("b1", "Bundled"));

        let mut edited = sample("b1", "Edited");
        edited.shapes[0].input = "button:7".into();
        assert!(save(&bundled, &user, edited).unwrap_err().contains("read-only"));
        assert!(!user.join("b1").exists());

        assert!(add_image(&bundled, &user, "b1", &bundled.join("b1").join("top.png")).is_err());

        assert!(remove_image(&bundled, &user, "b1", "top.png").is_err());
        assert!(bundled.join("b1").join("top.png").is_file());

        assert!(delete(&bundled, &user, "b1").is_err());
        assert!(bundled.join("b1").join(MAP_FILE).is_file());
    }

    #[test]
    fn save_of_unknown_id_fails() {
        let t = Tmp::new();
        let (bundled, user) = (t.path("bundled"), t.path("user"));
        assert!(save(&bundled, &user, sample("nope", "X")).unwrap_err().contains("unknown"));
        assert!(!user.join("nope").exists());

        // A user map whose image file is missing does not save either.
        put(&user, &sample("u1", "User"));
        let mut p = sample("u1", "User");
        p.image = ImageFile { file: "side.png".into(), label: String::new() };
        assert!(save(&bundled, &user, p).unwrap_err().contains("side.png"));
    }

    #[test]
    fn clone_makes_an_editable_user_copy() {
        let t = Tmp::new();
        let (bundled, user) = (t.path("bundled"), t.path("user"));
        put(&bundled, &sample("p1", "Bundled"));

        let cloned = clone_map(&bundled, &user, "p1", "  My copy ").unwrap();
        assert_ne!(cloned.id, "p1");
        assert_eq!(cloned.name, "My copy");
        assert_eq!(cloned.hardware_id, sample("p1", "Bundled").hardware_id);
        assert_eq!(cloned.shapes.len(), 1);
        assert!(user.join(&cloned.id).join(MAP_FILE).is_file());
        assert!(user.join(&cloned.id).join(&cloned.image.file).is_file());

        let list = list(&bundled, &user);
        assert_eq!(list.len(), 2);
        assert_eq!(list.iter().find(|s| s.id == cloned.id).unwrap().source, ImageMapSource::User);
        assert_eq!(list.iter().find(|s| s.id == "p1").unwrap().source, ImageMapSource::Bundled);

        // The clone is a normal, writable user map.
        let mut edited = cloned.clone();
        edited.name = "Renamed".into();
        save(&bundled, &user, edited).unwrap();
        assert_eq!(read_map(&user.join(&cloned.id)).unwrap().name, "Renamed");

        // Bundled folder untouched throughout.
        assert_eq!(read_map(&bundled.join("p1")).unwrap().name, "Bundled");

        // Cloning a user map works the same way.
        put(&user, &sample("u1", "User"));
        let cloned2 = clone_map(&bundled, &user, "u1", "Copy of user").unwrap();
        assert_eq!(cloned2.name, "Copy of user");
        assert!(user.join(&cloned2.id).join(MAP_FILE).is_file());

        // Cloning an unknown id fails.
        assert!(clone_map(&bundled, &user, "missing", "X").is_err());
    }

    #[test]
    fn create_and_add_image() {
        let t = Tmp::new();
        let (bundled, user) = (t.path("bundled"), t.path("user"));
        let src = t.path("My Stick Top.PNG");
        fs::write(&src, PNG).unwrap();
        let p = create(&user, "New", "{GUID}", "Stick", &src).unwrap();
        assert_eq!(p.id.len(), 36);
        assert_eq!(p.image.file, "My Stick Top.PNG");
        assert_eq!(p.image.label, "My Stick Top");
        assert!(user.join(&p.id).join(MAP_FILE).is_file());
        assert!(user.join(&p.id).join("My Stick Top.PNG").is_file());
        // No image, no image-map.
        assert!(create(&user, "No", "{GUID}", "Stick", &t.path("missing.png")).is_err());

        // Same source again: file name de-duplicated.
        let img2 = add_image(&bundled, &user, &p.id, &src).unwrap();
        assert_eq!(img2.file, "My Stick Top-2.PNG");
        assert!(user.join(&p.id).join(&img2.file).is_file());

        let bad = t.path("readme.txt");
        fs::write(&bad, b"x").unwrap();
        assert!(add_image(&bundled, &user, &p.id, &bad).is_err());

        remove_image(&bundled, &user, &p.id, &img2.file).unwrap();
        assert!(!user.join(&p.id).join(&img2.file).exists());
        remove_image(&bundled, &user, &p.id, "never-there.png").unwrap();
        assert!(remove_image(&bundled, &user, &p.id, "../x").is_err());
    }

    #[test]
    fn read_image_maps_mime_by_extension() {
        let t = Tmp::new();
        let (bundled, user) = (t.path("bundled"), t.path("user"));
        let mut p = sample("p1", "P");
        p.image = ImageFile { file: "a.png".into(), label: String::new() };
        p.shapes.clear();
        put(&user, &p);
        // read_image serves any bare file in the folder, referenced or not.
        for f in ["b.JPG", "c.jpeg", "d.webp"] {
            fs::write(user.join("p1").join(f), PNG).unwrap();
        }

        let expect = |file: &str, mime: &str| {
            let url = read_image(&bundled, &user, "p1", file).unwrap();
            assert!(url.starts_with(&format!("data:{mime};base64,")), "{file} -> {url}");
        };
        expect("a.png", "image/png");
        expect("b.JPG", "image/jpeg");
        expect("c.jpeg", "image/jpeg");
        expect("d.webp", "image/webp");
        let url = read_image(&bundled, &user, "p1", "a.png").unwrap();
        assert_eq!(url, format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(PNG)));

        assert!(read_image(&bundled, &user, "p1", "x.gif").is_err());
        assert!(read_image(&bundled, &user, "p1", "../a.png").is_err());
        assert!(read_image(&bundled, &user, "p1", "missing.png").is_err());
    }

    #[test]
    fn export_import_round_trip() {
        let t = Tmp::new();
        let (bundled, user, user2) = (t.path("bundled"), t.path("user"), t.path("user2"));
        put(&user, &sample("rt", "Round trip"));
        let zip_path = t.path("rt.zip");
        export(&bundled, &user, "rt", &zip_path).unwrap();

        let summary = import(&user2, &zip_path).unwrap();
        assert_ne!(summary.id, "rt", "import assigns a fresh id");
        assert_eq!(summary.source, ImageMapSource::User);
        let imported = get(&bundled, &user2, &summary.id).unwrap();
        assert_eq!(imported.id, summary.id);
        assert_eq!(imported.name, "Round trip");
        assert_eq!(imported.shapes[0].input, "button:5");
        assert_eq!(fs::read(user2.join(&summary.id).join("top.png")).unwrap(), PNG);

        // Importing again makes a second, independent map.
        let again = import(&user2, &zip_path).unwrap();
        assert_ne!(again.id, summary.id);
        assert_eq!(list(&bundled, &user2).len(), 2);

        // Exporting a bundled image-map works too.
        put(&bundled, &sample("b", "B"));
        export(&bundled, &t.path("none"), "b", &t.path("b.zip")).unwrap();
    }

    #[test]
    fn import_refuses_unsafe_or_incomplete_zips() {
        let t = Tmp::new();
        let user = t.path("user");
        let write_zip = |name: &str, entries: &[(&str, &[u8])]| -> PathBuf {
            let path = t.path(name);
            let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
            for (n, bytes) in entries {
                zip.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
                zip.write_all(bytes).unwrap();
            }
            zip.finish().unwrap();
            path
        };
        let json = serde_json::to_vec(&sample("z", "Z")).unwrap();

        let z = write_zip("traversal.zip", &[("imagemap.json", &json), ("../evil.png", PNG)]);
        assert!(import(&user, &z).unwrap_err().contains("refusing"));

        let z = write_zip("nested.zip", &[("sub/imagemap.json", &json)]);
        assert!(import(&user, &z).is_err());

        let z = write_zip("noimage.zip", &[("imagemap.json", &json)]);
        assert!(import(&user, &z).unwrap_err().contains("top.png"));

        let z = write_zip("nojson.zip", &[("top.png", PNG)]);
        assert!(import(&user, &z).unwrap_err().contains("imagemap.json"));

        let bad = serde_json::to_vec(&sample("../z", "Z")).unwrap();
        let z = write_zip("badid.zip", &[("imagemap.json", &bad), ("top.png", PNG)]);
        assert!(import(&user, &z).is_err());
        assert!(!user.exists() || fs::read_dir(&user).unwrap().next().is_none());

        // Only the files the map references land on disk.
        let z = write_zip("extra.zip", &[("imagemap.json", &json), ("top.png", PNG), ("stray.png", PNG), ("notes.txt", b"x")]);
        let imported = import(&user, &z).unwrap();
        let dir = user.join(&imported.id);
        assert!(dir.join("top.png").is_file());
        assert!(!dir.join("stray.png").exists());
        assert!(!dir.join("notes.txt").exists());
    }

    #[test]
    fn image_shape_files_travel_with_the_map() {
        let t = Tmp::new();
        let (bundled, user, user2) = (t.path("bundled"), t.path("user"), t.path("user2"));
        let mut p = sample("img", "With image shape");
        p.shapes.push(image_shape("i1", "glyph.png"));
        put(&user, &p);

        // Clone copies the extra file.
        let cloned = clone_map(&bundled, &user, "img", "Copy").unwrap();
        assert!(user.join(&cloned.id).join("glyph.png").is_file());

        // Export / import carry it too, and import refuses a zip without it.
        let zip_path = t.path("img.zip");
        export(&bundled, &user, "img", &zip_path).unwrap();
        let imported = import(&user2, &zip_path).unwrap();
        assert!(user2.join(&imported.id).join("glyph.png").is_file());
        fs::remove_file(user.join("img").join("glyph.png")).unwrap();
        assert!(export(&bundled, &user, "img", &t.path("broken.zip")).is_err());
        assert!(save(&bundled, &user, p.clone()).unwrap_err().contains("glyph.png"));

        // Save prunes image files nothing references any more, and only those.
        fs::write(user.join("img").join("glyph.png"), PNG).unwrap();
        fs::write(user.join("img").join("orphan.png"), PNG).unwrap();
        fs::write(user.join("img").join("notes.txt"), b"x").unwrap();
        let mut without = p.clone();
        without.shapes.pop();
        save(&bundled, &user, without).unwrap();
        assert!(!user.join("img").join("glyph.png").exists());
        assert!(!user.join("img").join("orphan.png").exists());
        assert!(user.join("img").join("notes.txt").is_file());
        assert!(user.join("img").join("top.png").is_file());
    }

    #[test]
    fn unique_counts_up() {
        assert_eq!(unique("top", |_| false), "top");
        assert_eq!(unique("top", |s| s == "top" || s == "top-2"), "top-3");
    }
}
