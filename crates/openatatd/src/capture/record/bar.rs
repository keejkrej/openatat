//! Software stop bar. No GPU. Elapsed + Stop. Esc cancels.
//!
//! Clicks on empty background start a drag. Stop is the only click that
//! ends the recording. A stray click elsewhere is not this surface.

use font8x8::legacy::BASIC_LEGACY;

use super::policy::format_elapsed;

pub const BAR_W: u32 = 280;
pub const BAR_H: u32 = 44;

const COL_BG: u32 = 0xFF1A1B26;
const COL_PANEL: u32 = 0xFF24283B;
const COL_TEXT: u32 = 0xFFC0CAF5;
const COL_MUTED: u32 = 0xFF565F89;
const COL_DANGER: u32 = 0xFFF7768E;
const COL_REC: u32 = 0xFFF7768E;

const STOP_X: u32 = 196;
const STOP_Y: u32 = 10;
const STOP_W: u32 = 72;
const STOP_H: u32 = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarHit {
    Stop,
    Drag,
}

pub fn render(width: u32, height: u32, elapsed_secs: u64) -> Vec<u8> {
    let mut buf = vec![0u8; (width * height * 4) as usize];
    fill(&mut buf, width, 0, 0, width, height, COL_BG);
    fill(
        &mut buf,
        width,
        2,
        2,
        width.saturating_sub(4),
        height.saturating_sub(4),
        COL_PANEL,
    );
    fill(&mut buf, width, 12, 16, 10, 10, COL_REC);
    text(
        &mut buf,
        width,
        28,
        14,
        &format_elapsed(elapsed_secs),
        COL_TEXT,
        2,
    );
    text(&mut buf, width, 100, 16, "Esc cancels", COL_MUTED, 1);
    fill(&mut buf, width, STOP_X, STOP_Y, STOP_W, STOP_H, COL_DANGER);
    text(&mut buf, width, STOP_X + 14, STOP_Y + 6, "Stop", COL_BG, 1);
    buf
}

pub fn hit(x: f64, y: f64) -> BarHit {
    if x >= f64::from(STOP_X)
        && x <= f64::from(STOP_X + STOP_W)
        && y >= f64::from(STOP_Y)
        && y <= f64::from(STOP_Y + STOP_H)
    {
        BarHit::Stop
    } else {
        BarHit::Drag
    }
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
    fn stop_hit_is_only_the_button() {
        assert_eq!(hit(200.0, 16.0), BarHit::Stop);
        assert_eq!(hit(20.0, 16.0), BarHit::Drag);
        assert_eq!(hit(400.0, 16.0), BarHit::Drag);
    }

    #[test]
    fn render_is_sized() {
        let buf = render(BAR_W, BAR_H, 12);
        assert_eq!(buf.len(), (BAR_W * BAR_H * 4) as usize);
        assert!(buf.iter().any(|&b| b != 0));
    }
}
