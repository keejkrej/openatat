//! Capture inventory C1–C19 lives in SPEC.md. P0 implements C1 only.
//!
//! Not in P0 (stubs / comments only):
//!   C2 area, C3 window, C4 explicit display, C5 all-in-one,
//!   C6 scrolling, C7 video, C8 GIF, C9 OCR,
//!   C10 selection bar lives in `crate::selection` (native layer-shell, not here),
//!   C11–C12 file manager (Nautilus has no selection D-Bus API),
//!   C13 current clipboard tile, C14 clipboard shelf, C15 app-layout,
//!   C16 Orb drop, C17 annotation, C18 video trim, C19 record bezel.
//!
//! gpui `ScreenCaptureFrame` is a stub — capture stays in this daemon.

use crate::error::Result;

/// Long-edge target after C1 downscale (SPEC: ~1600–1920).
pub const LONG_EDGE: u32 = 1760;

#[derive(Debug, Clone)]
pub struct Still {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl Still {
    pub fn thumbnail_argb(&self, max_w: u32, max_h: u32) -> Result<(u32, u32, Vec<u8>)> {
        let img = image::load_from_memory(&self.png)?;
        let thumb = img.thumbnail(max_w, max_h);
        let rgba = thumb.to_rgba8();
        let w = rgba.width();
        let h = rgba.height();
        let mut argb = Vec::with_capacity((w * h * 4) as usize);
        for px in rgba.pixels() {
            // wl_shm Argb8888 little-endian byte order is B,G,R,A.
            argb.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
        Ok((w, h, argb))
    }
}

pub fn downscale_long_edge(png: &[u8], long_edge: u32) -> Result<Still> {
    let img = image::load_from_memory(png)?;
    let (w, h) = img.dimensions_u32();
    let current = w.max(h);
    let img = if current > long_edge {
        let scale = long_edge as f32 / current as f32;
        let nw = ((w as f32) * scale).round().max(1.0) as u32;
        let nh = ((h as f32) * scale).round().max(1.0) as u32;
        img.resize(nw, nh, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let (width, height) = img.dimensions_u32();
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)?;
    Ok(Still {
        png: out,
        width,
        height,
    })
}

trait Dims {
    fn dimensions_u32(&self) -> (u32, u32);
}

impl Dims for image::DynamicImage {
    fn dimensions_u32(&self) -> (u32, u32) {
        (self.width(), self.height())
    }
}

/// C1: one-shot still of the active output.
pub fn capture_active_output(output: Option<&str>) -> Result<Still> {
    #[cfg(target_os = "linux")]
    {
        return linux::capture_active_output(output);
    }
    #[cfg(target_os = "macos")]
    {
        let _ = output;
        return macos::capture_active_output();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = output;
        return windows::capture_active_output();
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = output;
        Err(crate::error::Error::msg("capture unsupported on this OS"))
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::process::Command;

    pub fn capture_active_output(output: Option<&str>) -> Result<Still> {
        // grim is silent on Hyprland. Do not use xdg-desktop-portal Screenshot
        // on this path — that presents a picker and breaks the Atat moment.
        let tmp = std::env::temp_dir().join(format!("openatat-c1-{}.png", std::process::id()));
        let mut cmd = Command::new("grim");
        if let Some(name) = output {
            cmd.args(["-o", name]);
        }
        cmd.arg(&tmp);
        let status = cmd.status().map_err(|e| {
            crate::error::Error::msg(format!(
                "grim is required for C1 auto-still (`{e}`). On Omarchy/Hyprland install grim."
            ))
        })?;
        if !status.success() {
            let _ = std::fs::remove_file(&tmp);
            return Err(crate::error::Error::msg(
                "grim failed (is the output name valid?)",
            ));
        }
        let png = std::fs::read(&tmp)?;
        let _ = std::fs::remove_file(&tmp);
        downscale_long_edge(&png, LONG_EDGE)
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;

    pub fn capture_active_output() -> Result<Still> {
        // ScreenCaptureKit. SCShareableContent + SCScreenshotManager.
        // Do not use gpui ScreenCaptureFrame (stub).
        Err(crate::error::Error::msg(
            "macOS capture is a stub: ScreenCaptureKit (not gpui ScreenCaptureFrame)",
        ))
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use super::*;

    pub fn capture_active_output() -> Result<Still> {
        // Windows.Graphics.Capture (WGC) / DXGI desktop duplication.
        Err(crate::error::Error::msg(
            "Windows capture is a stub: WGC (Windows.Graphics.Capture)",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downscale_caps_long_edge() {
        let mut img = image::RgbaImage::new(4000, 2000);
        for px in img.pixels_mut() {
            *px = image::Rgba([10, 20, 30, 255]);
        }
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let still = downscale_long_edge(&png, 1760).unwrap();
        assert_eq!(still.width.max(still.height), 1760);
        assert!(still.width > 0 && still.height > 0);
    }

    #[test]
    fn small_image_is_unchanged() {
        let img = image::RgbaImage::new(64, 32);
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let still = downscale_long_edge(&png, 1760).unwrap();
        assert_eq!((still.width, still.height), (64, 32));
    }
}
