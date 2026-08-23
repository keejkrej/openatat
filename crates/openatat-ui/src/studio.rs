//! C17 annotation document. Local PNG/JPEG only. Nothing is uploaded.
//!
//! Business logic is display-free so `cargo test --workspace` stays GPU-free.
//! The gpui window lives in `studio_ui` behind `--features gpui`.

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use font8x8::legacy::BASIC_LEGACY;
use image::{imageops, DynamicImage, ImageFormat, Rgba, RgbaImage};

use crate::paths;

/// Markup tools that stay individually undoable until export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Arrow,
    Rect,
    Ellipse,
    Freehand,
    Highlighter,
    Text,
    Step,
    Blur,
    Pixelate,
    Spotlight,
    Crop,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pt {
    pub x: f32,
    pub y: f32,
}

impl Pt {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn min(self, other: Self) -> Self {
        Self {
            x: self.x.min(other.x),
            y: self.y.min(other.y),
        }
    }

    pub fn max(self, other: Self) -> Self {
        Self {
            x: self.x.max(other.x),
            y: self.y.max(other.y),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LayerKind {
    Arrow { from: Pt, to: Pt },
    Rect { min: Pt, max: Pt },
    Ellipse { min: Pt, max: Pt },
    Freehand { points: Vec<Pt> },
    Highlighter { points: Vec<Pt> },
    Text { at: Pt, content: String },
    Step { at: Pt, n: u32 },
    Blur { min: Pt, max: Pt },
    Pixelate { min: Pt, max: Pt },
    Spotlight { min: Pt, max: Pt },
    Crop { min: Pt, max: Pt },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub id: u64,
    pub kind: LayerKind,
    pub color: [u8; 4],
    pub stroke: f32,
}

#[derive(Debug, Clone)]
pub struct Document {
    pub source: PathBuf,
    pub width: u32,
    pub height: u32,
    base: RgbaImage,
    layers: Vec<Layer>,
    redo: Vec<Layer>,
    next_id: u64,
    pub color: [u8; 4],
    pub stroke: f32,
    pub text: String,
}

const ACCENT: [u8; 4] = [232, 93, 76, 255];
const HIGHLIGHT: [u8; 4] = [245, 230, 66, 96];
const TEXT: [u8; 4] = [255, 255, 255, 255];
const STEP: [u8; 4] = [122, 162, 247, 255];

/// True when `s` names a remote resource. Annotate never fetches those.
pub fn is_remote_ref(s: &str) -> bool {
    let s = s.trim();
    if let Some(i) = s.find("://") {
        return i > 0 && s[..i].bytes().all(|c| c.is_ascii_alphabetic());
    }
    false
}

pub fn is_local_raster(path: &Path) -> bool {
    if is_remote_ref(&path.to_string_lossy()) {
        return false;
    }
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => {
            let e = ext.to_ascii_lowercase();
            e == "png" || e == "jpg" || e == "jpeg"
        }
        None => false,
    }
}

/// `<stem>-annotated.png` next to the source. Always a local path.
pub fn export_path_for(source: &Path) -> PathBuf {
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image");
    match source.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => {
            parent.join(format!("{stem}-annotated.png"))
        }
        _ => PathBuf::from(format!("{stem}-annotated.png")),
    }
}

pub fn cache_export_path(source: &Path) -> PathBuf {
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image");
    paths::cache_dir()
        .join("studio")
        .join(format!("{stem}-annotated.png"))
}

impl Document {
    pub fn open(path: &Path) -> Result<Self, String> {
        let raw = path.to_string_lossy();
        if raw.trim().is_empty() {
            return Err("open a local PNG or JPEG path".into());
        }
        if is_remote_ref(&raw) {
            return Err(
                "annotate never fetches URLs — pass a local PNG/JPEG path (nothing is uploaded)"
                    .into(),
            );
        }
        if !is_local_raster(path) {
            return Err("local PNG/JPEG only".into());
        }
        let bytes = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let img = image::load_from_memory(&bytes)
            .map_err(|e| format!("decode {}: {e}", path.display()))?
            .to_rgba8();
        let width = img.width();
        let height = img.height();
        if width == 0 || height == 0 {
            return Err("image has no pixels".into());
        }
        Ok(Self {
            source: path.to_path_buf(),
            width,
            height,
            base: img,
            layers: Vec::new(),
            redo: Vec::new(),
            next_id: 1,
            color: ACCENT,
            stroke: 3.0,
            text: "Text".into(),
        })
    }

    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }

    pub fn can_undo(&self) -> bool {
        !self.layers.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn next_step(&self) -> u32 {
        self.layers
            .iter()
            .filter_map(|l| match l.kind {
                LayerKind::Step { n, .. } => Some(n),
                _ => None,
            })
            .max()
            .unwrap_or(0)
            + 1
    }

    pub fn push_kind(&mut self, kind: LayerKind) {
        let (color, stroke) = match &kind {
            LayerKind::Highlighter { .. } => (HIGHLIGHT, 14.0),
            LayerKind::Text { .. } => (TEXT, self.stroke),
            LayerKind::Step { .. } => (STEP, self.stroke),
            LayerKind::Blur { .. } | LayerKind::Pixelate { .. } | LayerKind::Crop { .. } => {
                (self.color, self.stroke)
            }
            LayerKind::Spotlight { .. } => (self.color, self.stroke),
            _ => (self.color, self.stroke),
        };
        self.layers.push(Layer {
            id: self.next_id,
            kind,
            color,
            stroke,
        });
        self.next_id += 1;
        self.redo.clear();
    }

    pub fn commit_drag(&mut self, tool: Tool, from: Pt, to: Pt) {
        let min = from.min(to);
        let max = from.max(to);
        let kind = match tool {
            Tool::Arrow => LayerKind::Arrow { from, to },
            Tool::Rect => LayerKind::Rect { min, max },
            Tool::Ellipse => LayerKind::Ellipse { min, max },
            Tool::Blur => LayerKind::Blur { min, max },
            Tool::Pixelate => LayerKind::Pixelate { min, max },
            Tool::Spotlight => LayerKind::Spotlight { min, max },
            Tool::Crop => LayerKind::Crop { min, max },
            Tool::Freehand => LayerKind::Freehand {
                points: vec![from, to],
            },
            Tool::Highlighter => LayerKind::Highlighter {
                points: vec![from, to],
            },
            Tool::Text | Tool::Step => return,
        };
        self.push_kind(kind);
    }

    pub fn commit_click(&mut self, tool: Tool, at: Pt) {
        match tool {
            Tool::Text => self.push_kind(LayerKind::Text {
                at,
                content: self.text.clone(),
            }),
            Tool::Step => self.push_kind(LayerKind::Step {
                at,
                n: self.next_step(),
            }),
            _ => {}
        }
    }

    pub fn commit_stroke(&mut self, tool: Tool, points: Vec<Pt>) {
        if points.len() < 2 {
            return;
        }
        let kind = match tool {
            Tool::Freehand => LayerKind::Freehand { points },
            Tool::Highlighter => LayerKind::Highlighter { points },
            _ => return,
        };
        self.push_kind(kind);
    }

    pub fn undo(&mut self) -> bool {
        if let Some(layer) = self.layers.pop() {
            self.redo.push(layer);
            true
        } else {
            false
        }
    }

    pub fn redo(&mut self) -> bool {
        if let Some(layer) = self.redo.pop() {
            self.layers.push(layer);
            true
        } else {
            false
        }
    }

    pub fn flatten(&self) -> RgbaImage {
        let mut img = self.base.clone();
        let mut crop: Option<(u32, u32, u32, u32)> = None;
        for layer in &self.layers {
            match &layer.kind {
                LayerKind::Crop { min, max } => {
                    crop = Some(clamp_rect(&img, *min, *max));
                }
                LayerKind::Arrow { from, to } => {
                    draw_arrow(&mut img, *from, *to, layer.color, layer.stroke);
                }
                LayerKind::Rect { min, max } => {
                    draw_rect_outline(&mut img, *min, *max, layer.color, layer.stroke);
                }
                LayerKind::Ellipse { min, max } => {
                    draw_ellipse_outline(&mut img, *min, *max, layer.color, layer.stroke);
                }
                LayerKind::Freehand { points } => {
                    draw_poly(&mut img, points, layer.color, layer.stroke);
                }
                LayerKind::Highlighter { points } => {
                    draw_poly(&mut img, points, layer.color, layer.stroke);
                }
                LayerKind::Text { at, content } => {
                    draw_label(&mut img, *at, content, layer.color, 2);
                }
                LayerKind::Step { at, n } => {
                    draw_step(&mut img, *at, *n, layer.color);
                }
                LayerKind::Blur { min, max } => {
                    apply_blur(&mut img, *min, *max);
                }
                LayerKind::Pixelate { min, max } => {
                    apply_pixelate(&mut img, *min, *max, 12);
                }
                LayerKind::Spotlight { min, max } => {
                    apply_spotlight(&mut img, *min, *max);
                }
            }
        }
        if let Some((x, y, w, h)) = crop {
            if w > 0 && h > 0 {
                return imageops::crop_imm(&img, x, y, w, h).to_image();
            }
        }
        img
    }

    pub fn encode_png(img: &RgbaImage) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        DynamicImage::ImageRgba8(img.clone())
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        Ok(out)
    }

    fn write_png(&self, dest: &Path) -> Result<(), String> {
        if is_remote_ref(&dest.to_string_lossy()) {
            return Err("export is a local file — refusing a remote path".into());
        }
        if let Some(parent) = dest.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
        }
        let img = self.flatten();
        let bytes = Self::encode_png(&img)?;
        fs::write(dest, bytes).map_err(|e| format!("write {}: {e}", dest.display()))
    }

    /// Flattened PNG next to the source, or a working copy under the cache.
    pub fn export(&self) -> Result<PathBuf, String> {
        let dest = export_path_for(&self.source);
        match self.write_png(&dest) {
            Ok(()) => Ok(dest),
            Err(e) => {
                let fallback = cache_export_path(&self.source);
                self.write_png(&fallback)
                    .map_err(|e2| format!("{e}; cache fallback: {e2}"))?;
                Ok(fallback)
            }
        }
    }
}

fn clamp_rect(img: &RgbaImage, min: Pt, max: Pt) -> (u32, u32, u32, u32) {
    let x0 = min.x.min(max.x).max(0.0) as u32;
    let y0 = min.y.min(max.y).max(0.0) as u32;
    let x1 = min.x.max(max.x).max(0.0) as u32;
    let y1 = min.y.max(max.y).max(0.0) as u32;
    let x1 = x1.min(img.width());
    let y1 = y1.min(img.height());
    let x0 = x0.min(x1);
    let y0 = y0.min(y1);
    (x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0))
}

fn blend(dst: &mut Rgba<u8>, src: [u8; 4]) {
    let a = src[3] as f32 / 255.0;
    if a <= 0.0 {
        return;
    }
    if a >= 1.0 {
        *dst = Rgba(src);
        return;
    }
    for i in 0..3 {
        dst.0[i] = (src[i] as f32 * a + dst.0[i] as f32 * (1.0 - a)).round() as u8;
    }
    dst.0[3] = 255;
}

fn plot(img: &mut RgbaImage, x: i32, y: i32, color: [u8; 4]) {
    if x < 0 || y < 0 {
        return;
    }
    let (x, y) = (x as u32, y as u32);
    if x < img.width() && y < img.height() {
        blend(img.get_pixel_mut(x, y), color);
    }
}

fn disk(img: &mut RgbaImage, cx: i32, cy: i32, r: i32, color: [u8; 4]) {
    let r = r.max(1);
    let r2 = r * r;
    for dy in -r..=r {
        for dx in -r..=r {
            if dx * dx + dy * dy <= r2 {
                plot(img, cx + dx, cy + dy, color);
            }
        }
    }
}

fn line(img: &mut RgbaImage, a: Pt, b: Pt, color: [u8; 4], stroke: f32) {
    let r = (stroke * 0.5).ceil().max(1.0) as i32;
    let x0 = a.x as i32;
    let y0 = a.y as i32;
    let x1 = b.x as i32;
    let y1 = b.y as i32;
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let mut x = x0;
    let mut y = y0;
    loop {
        disk(img, x, y, r, color);
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

fn draw_arrow(img: &mut RgbaImage, from: Pt, to: Pt, color: [u8; 4], stroke: f32) {
    line(img, from, to, color, stroke);
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    let ux = dx / len;
    let uy = dy / len;
    let head = 14.0_f32.max(stroke * 3.0);
    let left = Pt::new(
        to.x - ux * head + uy * head * 0.45,
        to.y - uy * head - ux * head * 0.45,
    );
    let right = Pt::new(
        to.x - ux * head - uy * head * 0.45,
        to.y - uy * head + ux * head * 0.45,
    );
    line(img, to, left, color, stroke);
    line(img, to, right, color, stroke);
}

fn draw_rect_outline(img: &mut RgbaImage, min: Pt, max: Pt, color: [u8; 4], stroke: f32) {
    let a = Pt::new(min.x, min.y);
    let b = Pt::new(max.x, min.y);
    let c = Pt::new(max.x, max.y);
    let d = Pt::new(min.x, max.y);
    line(img, a, b, color, stroke);
    line(img, b, c, color, stroke);
    line(img, c, d, color, stroke);
    line(img, d, a, color, stroke);
}

fn draw_ellipse_outline(img: &mut RgbaImage, min: Pt, max: Pt, color: [u8; 4], stroke: f32) {
    let cx = (min.x + max.x) * 0.5;
    let cy = (min.y + max.y) * 0.5;
    let rx = ((max.x - min.x).abs() * 0.5).max(1.0);
    let ry = ((max.y - min.y).abs() * 0.5).max(1.0);
    let steps = ((rx + ry) * 3.0).clamp(24.0, 360.0) as i32;
    let mut prev = Pt::new(cx + rx, cy);
    for i in 1..=steps {
        let t = std::f32::consts::TAU * (i as f32) / (steps as f32);
        let p = Pt::new(cx + rx * t.cos(), cy + ry * t.sin());
        line(img, prev, p, color, stroke);
        prev = p;
    }
}

fn draw_poly(img: &mut RgbaImage, points: &[Pt], color: [u8; 4], stroke: f32) {
    for pair in points.windows(2) {
        line(img, pair[0], pair[1], color, stroke);
    }
}

fn draw_label(img: &mut RgbaImage, at: Pt, text: &str, color: [u8; 4], scale: u32) {
    let mut x = at.x as i32;
    let y = at.y as i32;
    for ch in text.chars() {
        let idx = if (ch as u32) < 128 {
            ch as usize
        } else {
            b'?' as usize
        };
        let glyph = BASIC_LEGACY[idx];
        for (row, bits) in glyph.iter().enumerate() {
            for col in 0..8 {
                if bits & (1 << col) != 0 {
                    let px = x + col as i32 * scale as i32;
                    let py = y + row as i32 * scale as i32;
                    for oy in 0..scale as i32 {
                        for ox in 0..scale as i32 {
                            plot(img, px + ox - 1, py + oy, [0, 0, 0, 220]);
                            plot(img, px + ox + 1, py + oy, [0, 0, 0, 220]);
                            plot(img, px + ox, py + oy - 1, [0, 0, 0, 220]);
                            plot(img, px + ox, py + oy + 1, [0, 0, 0, 220]);
                            plot(img, px + ox, py + oy, color);
                        }
                    }
                }
            }
        }
        x += (8 * scale + scale) as i32;
    }
}

fn draw_step(img: &mut RgbaImage, at: Pt, n: u32, color: [u8; 4]) {
    let cx = at.x as i32;
    let cy = at.y as i32;
    disk(img, cx, cy, 12, color);
    disk(img, cx, cy, 10, [20, 22, 30, 255]);
    let label = n.to_string();
    draw_label(
        img,
        Pt::new(at.x - 4.0 * label.len() as f32, at.y - 7.0),
        &label,
        TEXT,
        1,
    );
}

fn apply_blur(img: &mut RgbaImage, min: Pt, max: Pt) {
    let (x, y, w, h) = clamp_rect(img, min, max);
    if w < 2 || h < 2 {
        return;
    }
    let sub = imageops::crop_imm(img, x, y, w, h).to_image();
    let blurred = imageops::blur(&sub, 4.5);
    imageops::replace(img, &blurred, x as i64, y as i64);
}

fn apply_pixelate(img: &mut RgbaImage, min: Pt, max: Pt, block: u32) {
    let (x0, y0, w, h) = clamp_rect(img, min, max);
    if w == 0 || h == 0 {
        return;
    }
    let block = block.max(2);
    for y in (y0..y0 + h).step_by(block as usize) {
        for x in (x0..x0 + w).step_by(block as usize) {
            let px = *img.get_pixel(x, y);
            let y1 = (y + block).min(y0 + h);
            let x1 = (x + block).min(x0 + w);
            for yy in y..y1 {
                for xx in x..x1 {
                    img.put_pixel(xx, yy, px);
                }
            }
        }
    }
}

fn apply_spotlight(img: &mut RgbaImage, min: Pt, max: Pt) {
    let (x0, y0, w, h) = clamp_rect(img, min, max);
    let x1 = x0 + w;
    let y1 = y0 + h;
    for y in 0..img.height() {
        for x in 0..img.width() {
            if x >= x0 && x < x1 && y >= y0 && y < y1 {
                continue;
            }
            let p = img.get_pixel_mut(x, y);
            p.0[0] = (p.0[0] as f32 * 0.28) as u8;
            p.0[1] = (p.0[1] as f32 * 0.28) as u8;
            p.0[2] = (p.0[2] as f32 * 0.28) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_png(name: &str, w: u32, h: u32, color: [u8; 4]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "openatat-studio-{}-{}",
            name,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("shot.png");
        let img = RgbaImage::from_pixel(w, h, Rgba(color));
        DynamicImage::ImageRgba8(img).save(&path).unwrap();
        path
    }

    #[test]
    fn rejects_urls_and_never_mentions_an_http_client() {
        assert!(is_remote_ref("https://example.com/a.png"));
        assert!(is_remote_ref("http://127.0.0.1/x.jpg"));
        assert!(is_remote_ref("ftp://files.test/a.png"));
        assert!(!is_remote_ref("/tmp/shot.png"));
        assert!(!is_remote_ref("shot.png"));
        let err = Document::open(Path::new("https://example.com/secret.png")).unwrap_err();
        assert!(err.contains("never fetches"), "{err}");
        let src = include_str!("studio.rs");
        let code = src.split("mod tests").next().unwrap_or(src);
        assert!(!code.contains("reqwest"));
        assert!(!code.contains("ureq"));
        assert!(!code.contains("attohttpc"));
        assert!(!code.contains("hyper::"));
        assert!(!code.contains("ScreenCaptureFrame"));
    }

    #[test]
    fn open_local_png_undo_export_is_a_local_file() {
        let path = tmp_png("export", 80, 40, [10, 20, 30, 255]);
        let mut doc = Document::open(&path).unwrap();
        doc.commit_drag(Tool::Arrow, Pt::new(4.0, 4.0), Pt::new(60.0, 20.0));
        doc.commit_drag(Tool::Rect, Pt::new(8.0, 8.0), Pt::new(30.0, 24.0));
        doc.commit_click(Tool::Step, Pt::new(16.0, 16.0));
        assert_eq!(doc.layers().len(), 3);
        assert!(doc.undo());
        assert_eq!(doc.layers().len(), 2);
        assert!(doc.redo());
        assert_eq!(doc.layers().len(), 3);
        let out = doc.export().unwrap();
        assert!(out.is_file(), "{}", out.display());
        assert_eq!(out.extension().and_then(|e| e.to_str()), Some("png"));
        assert!(!is_remote_ref(&out.to_string_lossy()));
        assert_eq!(out, export_path_for(&path));
        assert!(out
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with("-annotated.png"));
        let loaded = image::open(&out).unwrap();
        assert_eq!(loaded.width(), 80);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn crop_changes_export_size_and_is_undoable() {
        let path = tmp_png("crop", 100, 60, [80, 80, 80, 255]);
        let mut doc = Document::open(&path).unwrap();
        doc.commit_drag(Tool::Crop, Pt::new(10.0, 10.0), Pt::new(50.0, 40.0));
        let out = doc.export().unwrap();
        let cropped = image::open(&out).unwrap();
        assert_eq!((cropped.width(), cropped.height()), (40, 30));
        assert!(doc.undo());
        let full = doc.export().unwrap();
        let restored = image::open(&full).unwrap();
        assert_eq!((restored.width(), restored.height()), (100, 60));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn jpeg_extension_is_accepted_remote_is_not() {
        assert!(is_local_raster(Path::new("/home/me/pic.JPEG")));
        assert!(is_local_raster(Path::new("a.jpg")));
        assert!(!is_local_raster(Path::new("a.webp")));
        assert!(!is_local_raster(Path::new("https://x/a.png")));
        let err = Document::open(Path::new("http://localhost/a.jpg")).unwrap_err();
        assert!(err.contains("never fetches"), "{err}");
    }

    #[test]
    fn wrong_extension_is_rejected() {
        let err = Document::open(Path::new("/tmp/notes.txt")).unwrap_err();
        assert!(err.contains("PNG/JPEG"), "{err}");
    }

    #[test]
    fn source_has_no_network_fetch() {
        let src = include_str!("studio.rs");
        let code = src.split("mod tests").next().unwrap_or(src);
        assert!(code.contains("never fetches"));
        assert!(code.contains("local PNG/JPEG"));
        assert!(!code.contains("ureq::"));
        assert!(!code.contains("reqwest::"));
    }

    #[test]
    fn studio_window_source_is_local_paths_only() {
        let src = include_str!("studio_ui.rs");
        assert!(!src.contains("SharedUri"));
        assert!(!src.contains("http://"));
        assert!(!src.contains("https://"));
        assert!(src.contains("local paths only") || src.contains("local PNG"));
        assert!(!src.contains("ScreenCaptureFrame"));
        assert!(!src.contains("reqwest"));
    }
}
