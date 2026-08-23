//! Software Orb. No GPU. `@—@` eyes, a busy bubble, a wider error pill.

use font8x8::legacy::BASIC_LEGACY;

use super::policy::OrbOpenKind;

pub const ORB_IDLE: u32 = 56;
pub const ORB_BUSY: u32 = 64;
pub const ORB_ERROR_W: u32 = 200;
pub const ORB_ERROR_H: u32 = 48;

const COL_FACE: u32 = 0xFF1A1B26;
const COL_RING: u32 = 0xFF7AA2F7;
const COL_EYE: u32 = 0xFFC0CAF5;
const COL_PUPIL: u32 = 0xFF1A1B26;
const COL_BUBBLE: u32 = 0xFF7AA2F7;
const COL_ERR: u32 = 0xFFF7768E;
const COL_ERR_BG: u32 = 0xFF24283B;
const COL_TEXT: u32 = 0xFFC0CAF5;
const COL_ESC: u32 = 0xFF1A1B26;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrbFace {
    Idle,
    Busy,
    Error,
}

#[derive(Debug, Clone)]
pub struct OrbFrame {
    pub face: OrbFace,
    /// Pointer relative to the Orb's top-left, surface coords. Eyes look here.
    pub look_x: f32,
    pub look_y: f32,
    pub error: String,
    pub pulse: f32,
}

impl OrbFrame {
    pub fn idle(look_x: f32, look_y: f32) -> Self {
        Self {
            face: OrbFace::Idle,
            look_x,
            look_y,
            error: String::new(),
            pulse: 0.0,
        }
    }
}

pub fn size_for(face: OrbFace) -> (u32, u32) {
    match face {
        OrbFace::Idle => (ORB_IDLE, ORB_IDLE),
        OrbFace::Busy => (ORB_BUSY, ORB_BUSY),
        OrbFace::Error => (ORB_ERROR_W, ORB_ERROR_H),
    }
}

pub fn face_from_presence(busy: bool, error: Option<&str>) -> OrbFace {
    if busy {
        OrbFace::Busy
    } else if error.is_some() {
        OrbFace::Error
    } else {
        OrbFace::Idle
    }
}

/// Hit-test the circular face (or the whole pill when in error).
pub fn hit_orb(x: f64, y: f64, w: u32, h: u32, face: OrbFace) -> bool {
    if face == OrbFace::Error {
        return x >= 0.0 && y >= 0.0 && x < f64::from(w) && y < f64::from(h);
    }
    let cx = f64::from(w) / 2.0;
    let cy = f64::from(h) / 2.0;
    let r = f64::from(w.min(h)) / 2.0 - 1.0;
    let dx = x - cx;
    let dy = y - cy;
    dx * dx + dy * dy <= r * r
}

pub fn hit_error_esc(x: f64, y: f64, w: u32, h: u32, face: OrbFace) -> bool {
    face == OrbFace::Error
        && x >= f64::from(w.saturating_sub(52))
        && x <= f64::from(w.saturating_sub(8))
        && y >= 10.0
        && y <= f64::from(h.saturating_sub(10))
}

pub fn render(width: u32, height: u32, frame: &OrbFrame) -> Vec<u8> {
    let mut buf = vec![0u8; (width * height * 4) as usize];
    match frame.face {
        OrbFace::Error => render_error(&mut buf, width, height, &frame.error),
        OrbFace::Busy => {
            render_face(&mut buf, width, height, frame, ORB_BUSY);
            render_bubble(&mut buf, width, frame.pulse);
        }
        OrbFace::Idle => render_face(&mut buf, width, height, frame, ORB_IDLE),
    }
    let _ = OrbOpenKind::Click;
    buf
}

fn render_face(buf: &mut [u8], w: u32, h: u32, frame: &OrbFrame, size: u32) {
    let cx = w as i32 / 2;
    let cy = h as i32 / 2;
    let r = (size as i32 / 2) - 1;
    fill_circle(buf, w, cx, cy, r, COL_FACE);
    ring_circle(buf, w, cx, cy, r, COL_RING);
    // `@—@` — two at-signs with a dash. Pupils drift toward the pointer.
    let look_dx = ((frame.look_x - w as f32 / 2.0) / (w as f32 / 2.0)).clamp(-1.0, 1.0);
    let look_dy = ((frame.look_y - h as f32 / 2.0) / (h as f32 / 2.0)).clamp(-1.0, 1.0);
    let pupil = 3;
    let eye_y = cy - 4 + (look_dy * 3.0) as i32;
    let left_x = cx - 14 + (look_dx * 3.0) as i32;
    let right_x = cx + 6 + (look_dx * 3.0) as i32;
    text(buf, w, (cx - 18) as u32, (cy - 10) as u32, "@-@", COL_EYE, 1);
    fill_circle(buf, w, left_x + 4, eye_y + 6, pupil, COL_PUPIL);
    fill_circle(buf, w, right_x + 4, eye_y + 6, pupil, COL_PUPIL);
}

fn render_bubble(buf: &mut [u8], w: u32, pulse: f32) {
    let r = 7 + (pulse.sin().abs() * 2.0) as i32;
    fill_circle(buf, w, w as i32 - 10, 10, r, COL_BUBBLE);
}

fn render_error(buf: &mut [u8], w: u32, h: u32, message: &str) {
    fill_roundish(buf, w, 0, 0, w, h, COL_ERR_BG);
    fill_roundish(buf, w, 2, 2, w.saturating_sub(4), h.saturating_sub(4), COL_FACE);
    let shown = truncate(message, 16);
    text(buf, w, 10, 18, &shown, COL_ERR, 1);
    fill_roundish(buf, w, w.saturating_sub(48), 12, 36, 24, COL_ERR);
    text(buf, w, w.saturating_sub(42), 18, "Esc", COL_ESC, 1);
    let _ = COL_TEXT;
}

fn fill_circle(buf: &mut [u8], stride: u32, cx: i32, cy: i32, r: i32, color: u32) {
    let r2 = r * r;
    for y in (cy - r)..(cy + r + 1) {
        for x in (cx - r)..(cx + r + 1) {
            if x < 0 || y < 0 {
                continue;
            }
            let dx = x - cx;
            let dy = y - cy;
            if dx * dx + dy * dy <= r2 {
                put(buf, stride, x as u32, y as u32, color);
            }
        }
    }
}

fn ring_circle(buf: &mut [u8], stride: u32, cx: i32, cy: i32, r: i32, color: u32) {
    let outer = r * r;
    let inner = (r - 2).max(0);
    let inner2 = inner * inner;
    for y in (cy - r)..(cy + r + 1) {
        for x in (cx - r)..(cx + r + 1) {
            if x < 0 || y < 0 {
                continue;
            }
            let dx = x - cx;
            let dy = y - cy;
            let d = dx * dx + dy * dy;
            if d <= outer && d >= inner2 {
                put(buf, stride, x as u32, y as u32, color);
            }
        }
    }
}

fn fill_roundish(buf: &mut [u8], stride: u32, x: u32, y: u32, w: u32, h: u32, color: u32) {
    for yy in y..(y + h) {
        for xx in x..(x + w) {
            put(buf, stride, xx, yy, color);
        }
    }
}

fn put(buf: &mut [u8], stride: u32, x: u32, y: u32, color: u32) {
    if x >= stride {
        return;
    }
    let i = ((y * stride + x) * 4) as usize;
    if i + 3 < buf.len() {
        buf[i..i + 4].copy_from_slice(&color.to_le_bytes());
    }
}

fn text(buf: &mut [u8], stride: u32, mut x: u32, y: u32, s: &str, color: u32, scale: u32) {
    for ch in s.chars() {
        let idx = if (ch as u32) < 128 {
            ch as usize
        } else {
            b'?' as usize
        };
        let glyph = BASIC_LEGACY[idx];
        for (row, bits) in glyph.iter().enumerate() {
            for col in 0..8 {
                if bits & (1 << col) != 0 {
                    for sy in 0..scale {
                        for sx in 0..scale {
                            put(
                                buf,
                                stride,
                                x + col as u32 * scale + sx,
                                y + row as u32 * scale + sy,
                                color,
                            );
                        }
                    }
                }
            }
        }
        x += 8 * scale + scale;
    }
}

fn truncate(s: &str, max: usize) -> String {
    let c: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        format!("{c}…")
    } else {
        c
    }
}

/// Approximate a circle as stacked rectangles for `wl_region` / `SetWindowRgn`.
pub fn circle_rects(diameter: u32) -> Vec<(i32, i32, i32, i32)> {
    let r = diameter as i32 / 2;
    let mut out = Vec::new();
    for y in 0..diameter as i32 {
        let dy = y - r;
        let dx2 = r * r - dy * dy;
        if dx2 < 0 {
            continue;
        }
        let dx = (dx2 as f64).sqrt() as i32;
        let w = dx * 2;
        if w > 0 {
            out.push((r - dx, y, w, 1));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_buffer_is_sized_and_not_empty() {
        let buf = render(ORB_IDLE, ORB_IDLE, &OrbFrame::idle(10.0, 10.0));
        assert_eq!(buf.len(), (ORB_IDLE * ORB_IDLE * 4) as usize);
        assert!(buf.iter().any(|&b| b != 0));
    }

    #[test]
    fn circle_hit_is_the_disk() {
        assert!(hit_orb(28.0, 28.0, 56, 56, OrbFace::Idle));
        assert!(!hit_orb(0.0, 0.0, 56, 56, OrbFace::Idle));
        assert!(hit_orb(4.0, 4.0, ORB_ERROR_W, ORB_ERROR_H, OrbFace::Error));
    }

    #[test]
    fn esc_hit_only_on_error_pill() {
        assert!(!hit_error_esc(170.0, 20.0, ORB_ERROR_W, ORB_ERROR_H, OrbFace::Idle));
        assert!(hit_error_esc(170.0, 20.0, ORB_ERROR_W, ORB_ERROR_H, OrbFace::Error));
        assert!(!hit_error_esc(10.0, 20.0, ORB_ERROR_W, ORB_ERROR_H, OrbFace::Error));
    }

    #[test]
    fn circle_rects_cover_a_disk() {
        let rects = circle_rects(56);
        assert!(!rects.is_empty());
        assert!(rects.iter().all(|r| r.2 > 0 && r.3 > 0));
    }
}
