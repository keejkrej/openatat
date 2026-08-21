//! Software popover. No GPU, no fullscreen blur.

use font8x8::legacy::BASIC_LEGACY;

pub const POPOVER_W: u32 = 460;
pub const POPOVER_H: u32 = 300;

pub const COL_BG: u32 = 0xFF1A1B26;
pub const COL_PANEL: u32 = 0xFF24283B;
pub const COL_TEXT: u32 = 0xFFC0CAF5;
pub const COL_MUTED: u32 = 0xFF565F89;
pub const COL_ACCENT: u32 = 0xFF7AA2F7;
pub const COL_TILE: u32 = 0xFF414868;
pub const COL_DANGER: u32 = 0xFFF7768E;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Prompt,
    Running,
    Preview,
    Refine,
}

#[derive(Debug, Clone)]
pub struct Frame {
    pub phase: Phase,
    pub prompt: String,
    pub preview: String,
    pub has_tile: bool,
    pub status: String,
}

pub fn render(width: u32, height: u32, frame: &Frame, thumb: Option<(u32, u32, &[u8])>) -> Vec<u8> {
    let mut buf = vec![0u8; (width * height * 4) as usize];
    fill_rect(&mut buf, width, 0, 0, width, height, COL_BG);
    fill_rect(
        &mut buf,
        width,
        8,
        8,
        width.saturating_sub(16),
        height.saturating_sub(16),
        COL_PANEL,
    );

    text(&mut buf, width, 20, 18, "OpenAtat", COL_ACCENT, 2);
    text(
        &mut buf,
        width,
        width.saturating_sub(36),
        16,
        "x",
        COL_MUTED,
        2,
    );

    match frame.phase {
        Phase::Prompt => {
            text(
                &mut buf,
                width,
                20,
                52,
                "Type a prompt  Return runs  Esc cancels",
                COL_MUTED,
                1,
            );
            fill_rect(
                &mut buf,
                width,
                20,
                72,
                width.saturating_sub(40),
                36,
                COL_BG,
            );
            let shown = truncate(&frame.prompt, 48);
            let caret = format!("{shown}_");
            text(&mut buf, width, 26, 82, &caret, COL_TEXT, 1);
        }
        Phase::Running => {
            text(
                &mut buf,
                width,
                20,
                72,
                "Running BYO CLI…",
                COL_ACCENT,
                1,
            );
        }
        Phase::Refine => {
            text(
                &mut buf,
                width,
                20,
                52,
                "Refine  one more sentence  Return re-runs",
                COL_MUTED,
                1,
            );
            fill_rect(
                &mut buf,
                width,
                20,
                72,
                width.saturating_sub(40),
                36,
                COL_BG,
            );
            let shown = truncate(&frame.prompt, 48);
            let caret = format!("{shown}_");
            text(&mut buf, width, 26, 82, &caret, COL_TEXT, 1);
        }
        Phase::Preview => {
            text(
                &mut buf,
                width,
                20,
                52,
                "Preview  Tab inserts  R refine  Esc cancels",
                COL_MUTED,
                1,
            );
            fill_rect(
                &mut buf,
                width,
                20,
                72,
                width.saturating_sub(40),
                90,
                COL_BG,
            );
            for (i, line) in wrap(&frame.preview, 52).into_iter().take(6).enumerate() {
                text(
                    &mut buf,
                    width,
                    26,
                    80 + (i as u32) * 12,
                    &line,
                    COL_TEXT,
                    1,
                );
            }
        }
    }

    if frame.has_tile {
        fill_rect(&mut buf, width, 20, 180, 128, 72, COL_TILE);
        if let Some((tw, th, pixels)) = thumb {
            blit(&mut buf, width, 24, 184, tw, th, pixels);
        } else {
            text(&mut buf, width, 28, 208, "C1 still", COL_TEXT, 1);
        }
        fill_rect(&mut buf, width, 154, 180, 70, 22, COL_DANGER);
        text(&mut buf, width, 160, 186, "remove", COL_BG, 1);
        text(&mut buf, width, 154, 210, "auto-still", COL_MUTED, 1);
    }

    text(
        &mut buf,
        width,
        20,
        height.saturating_sub(28),
        &frame.status,
        COL_MUTED,
        1,
    );
    buf
}

pub fn hit_remove(x: f64, y: f64, has_tile: bool) -> bool {
    has_tile && x >= 154.0 && x <= 224.0 && y >= 180.0 && y <= 202.0
}

pub fn hit_close(x: f64, y: f64, width: u32) -> bool {
    x >= f64::from(width.saturating_sub(40)) && y <= 36.0
}

fn fill_rect(buf: &mut [u8], stride_px: u32, x: u32, y: u32, w: u32, h: u32, color: u32) {
    let bytes = color.to_le_bytes(); // B,G,R,A for 0xAARRGGBB?
                                     // color is 0xAARRGGBB; to_le_bytes on LE is B,G,R,A. Matches simple_layer.
    for yy in y..(y + h) {
        for xx in x..(x + w) {
            if xx >= stride_px {
                continue;
            }
            let i = ((yy * stride_px + xx) * 4) as usize;
            if i + 3 < buf.len() {
                buf[i..i + 4].copy_from_slice(&bytes);
            }
        }
    }
}

fn blit(buf: &mut [u8], stride_px: u32, x: u32, y: u32, tw: u32, th: u32, pixels: &[u8]) {
    for yy in 0..th {
        for xx in 0..tw {
            let si = ((yy * tw + xx) * 4) as usize;
            if si + 3 >= pixels.len() {
                continue;
            }
            let dx = x + xx;
            let dy = y + yy;
            if dx >= stride_px {
                continue;
            }
            let di = ((dy * stride_px + dx) * 4) as usize;
            if di + 3 < buf.len() {
                buf[di..di + 4].copy_from_slice(&pixels[si..si + 4]);
            }
        }
    }
}

fn text(buf: &mut [u8], stride_px: u32, mut x: u32, y: u32, s: &str, color: u32, scale: u32) {
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
                    fill_rect(
                        buf,
                        stride_px,
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

fn truncate(s: &str, max: usize) -> String {
    let c: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        format!("{c}…")
    } else {
        c
    }
}

fn wrap(s: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in s.split_whitespace() {
        if cur.len() + word.len() + 1 > width {
            if !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
            }
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_is_opaque_and_sized() {
        let frame = Frame {
            phase: Phase::Prompt,
            prompt: "hi".into(),
            preview: String::new(),
            has_tile: false,
            status: "idle".into(),
        };
        let buf = render(64, 32, &frame, None);
        assert_eq!(buf.len(), 64 * 32 * 4);
        assert!(buf.iter().any(|&b| b != 0));
    }

    #[test]
    fn remove_hit_only_with_tile() {
        assert!(!hit_remove(160.0, 190.0, false));
        assert!(hit_remove(160.0, 190.0, true));
    }
}
