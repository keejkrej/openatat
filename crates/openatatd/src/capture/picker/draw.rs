//! Software dim + rubber-band. No GPU. Same Argb8888 byte order as the overlay.

use font8x8::legacy::BASIC_LEGACY;

const DIM: u32 = 0x99000000;
const HOLE: u32 = 0x22000000;
const BORDER: u32 = 0xFFE0E6FF;
const HINT: u32 = 0xFFC0CAF5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

pub fn render(width: u32, height: u32, sel: Option<Rect>) -> Vec<u8> {
    let mut buf = vec![0u8; (width * height * 4) as usize];
    fill(&mut buf, width, 0, 0, width, height, DIM);
    if let Some(r) = sel {
        let x = r.x.max(0) as u32;
        let y = r.y.max(0) as u32;
        let w = r.w.min(width.saturating_sub(x));
        let h = r.h.min(height.saturating_sub(y));
        if w > 0 && h > 0 {
            fill(&mut buf, width, x, y, w, h, HOLE);
            frame(&mut buf, width, x, y, w, h, BORDER);
        }
    }
    text(
        &mut buf,
        width,
        24,
        24,
        "drag a rectangle  Esc cancels",
        HINT,
        2,
    );
    buf
}

fn fill(buf: &mut [u8], stride: u32, x: u32, y: u32, w: u32, h: u32, color: u32) {
    let bytes = color.to_le_bytes();
    for yy in y..(y + h) {
        for xx in x..(x + w) {
            if xx >= stride {
                continue;
            }
            let i = ((yy * stride + xx) * 4) as usize;
            if i + 3 < buf.len() {
                buf[i..i + 4].copy_from_slice(&bytes);
            }
        }
    }
}

fn frame(buf: &mut [u8], stride: u32, x: u32, y: u32, w: u32, h: u32, color: u32) {
    let t = 2u32;
    fill(buf, stride, x, y, w, t, color);
    fill(buf, stride, x, y.saturating_add(h.saturating_sub(t)), w, t, color);
    fill(buf, stride, x, y, t, h, color);
    fill(buf, stride, x.saturating_add(w.saturating_sub(t)), y, t, h, color);
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
                    fill(
                        buf,
                        stride,
                        x + col as u32 * scale,
                        y + row as u32 * scale,
                        scale,
                        scale,
                        color,
                    );
                }
            }
        }
        x += 8 * scale + scale;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_dims_and_cuts_a_hole() {
        let buf = render(
            64,
            48,
            Some(Rect {
                x: 8,
                y: 8,
                w: 20,
                h: 16,
            }),
        );
        assert_eq!(buf.len(), 64 * 48 * 4);
        assert!(buf.iter().any(|&b| b != 0));
    }
}
