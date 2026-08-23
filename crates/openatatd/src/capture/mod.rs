//! Capture inventory C1–C19 lives in SPEC.md. P0 implements C1 only.
//!
//! Not in P0 (stubs / comments only):
//!   C2 area, C3 window, C4 explicit display, C5 all-in-one,
//!   C6 scrolling, C7 video, C8 GIF, C9 OCR,
//!   C10 selection bar lives in `crate::selection` (native layer-shell, not here),
//!   C11–C12 file manager (Nautilus has no selection D-Bus API),
//!   C13 current clipboard tile, C14 clipboard shelf, C15 app-layout,
//!   C16 Orb drop lives in `crate::orb` (Wayland / Cocoa / Win32 drop target),
//!   C17 annotation, C18 video trim, C19 record bezel.
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
    use std::sync::mpsc;
    use std::time::Duration;

    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2_foundation::{NSArray, NSError};
    use objc2_screen_capture_kit::{
        SCContentFilter, SCDisplay, SCRunningApplication, SCScreenshotManager, SCShareableContent,
        SCStreamConfiguration, SCWindow,
    };

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
        fn CGImageGetWidth(image: *mut std::ffi::c_void) -> usize;
        fn CGImageGetHeight(image: *mut std::ffi::c_void) -> usize;
        fn CGImageGetBytesPerRow(image: *mut std::ffi::c_void) -> usize;
        fn CGImageGetBitsPerPixel(image: *mut std::ffi::c_void) -> usize;
        fn CGImageGetDataProvider(image: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
        fn CGDataProviderCopyData(provider: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
        fn CFDataGetLength(data: *mut std::ffi::c_void) -> isize;
        fn CFDataGetBytePtr(data: *mut std::ffi::c_void) -> *const u8;
        fn CFRelease(cf: *mut std::ffi::c_void);
    }

    pub fn capture_active_output() -> Result<Still> {
        // Screen Recording is optional. Skip the tile when TCC denies.
        // Do not use deprecated CGWindowListCreateImage as the primary path.
        unsafe {
            if !CGPreflightScreenCaptureAccess() {
                return Err(crate::error::Error::msg(
                    "Screen Recording is not granted — C1 still skipped. \
                     System Settings → Privacy & Security → Screen Recording → openatatd.",
                ));
            }
        }
        capture_sck()
    }

    fn capture_sck() -> Result<Still> {
        let content = shareable_content()?;
        let displays = unsafe { content.displays() };
        if displays.is_empty() {
            return Err(crate::error::Error::msg("ScreenCaptureKit: no displays"));
        }
        let display: Retained<SCDisplay> = displays
            .iter()
            .next()
            .ok_or_else(|| crate::error::Error::msg("ScreenCaptureKit: empty display list"))?
            .retain();

        let excluded_apps = our_apps(&content);
        let excluded_windows = our_windows(&content);
        let filter = unsafe {
            if !excluded_apps.is_empty() {
                SCContentFilter::initWithDisplay_excludingApplications_exceptingWindows(
                    SCContentFilter::alloc(),
                    &display,
                    &excluded_apps,
                    &NSArray::from_retained_slice(&[]),
                )
            } else {
                SCContentFilter::initWithDisplay_excludingWindows(
                    SCContentFilter::alloc(),
                    &display,
                    &excluded_windows,
                )
            }
        };

        let width = unsafe { display.width() } as u32;
        let height = unsafe { display.height() } as u32;
        let config = SCStreamConfiguration::new();
        unsafe {
            config.setWidth(width as usize);
            config.setHeight(height as usize);
            config.setShowsCursor(false);
        }

        let (tx, rx) = mpsc::channel();
        let block = RcBlock::new(move |image: *mut objc2_core_graphics::CGImage, err: *mut NSError| {
            if image.is_null() {
                let msg = unsafe { err.as_ref() }
                    .map(|e| e.localizedDescription().to_string())
                    .unwrap_or_else(|| "SCScreenshotManager returned no image".into());
                let _ = tx.send(Err(msg));
            } else {
                let _ = tx.send(cgimage_png(image as *mut std::ffi::c_void));
            }
        });
        unsafe {
            SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(
                &filter,
                &config,
                Some(&block),
            );
        }
        match rx.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(png)) => downscale_long_edge(&png, LONG_EDGE),
            Ok(Err(e)) => Err(crate::error::Error::msg(e)),
            Err(_) => Err(crate::error::Error::msg(
                "ScreenCaptureKit screenshot timed out",
            )),
        }
    }

    fn shareable_content() -> Result<Retained<SCShareableContent>> {
        let (tx, rx) = mpsc::channel();
        let block = RcBlock::new(move |content: *mut SCShareableContent, err: *mut NSError| {
            if content.is_null() {
                let msg = unsafe { err.as_ref() }
                    .map(|e| e.localizedDescription().to_string())
                    .unwrap_or_else(|| "SCShareableContent unavailable".into());
                let _ = tx.send(Err(msg));
            } else {
                let retained = unsafe { Retained::retain(content) }
                    .ok_or_else(|| "SCShareableContent retain failed".to_string());
                let _ = tx.send(retained);
            }
        });
        unsafe {
            SCShareableContent::getShareableContentWithCompletionHandler(&block);
        }
        match rx.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(c)) => Ok(c),
            Ok(Err(e)) => Err(crate::error::Error::msg(e)),
            Err(_) => Err(crate::error::Error::msg("SCShareableContent timed out")),
        }
    }

    fn our_apps(content: &SCShareableContent) -> Retained<NSArray<SCRunningApplication>> {
        let apps = unsafe { content.applications() };
        let mut ours = Vec::new();
        for app in apps.iter() {
            let name = unsafe { app.applicationName().to_string() };
            let bid = unsafe { app.bundleIdentifier().to_string() };
            if bid.contains("openatat") || name.to_ascii_lowercase().contains("openatat") {
                ours.push(app.retain());
            }
        }
        NSArray::from_retained_slice(&ours)
    }

    fn our_windows(content: &SCShareableContent) -> Retained<NSArray<SCWindow>> {
        let windows = unsafe { content.windows() };
        let mut ours = Vec::new();
        for w in windows.iter() {
            let title = unsafe { w.title().to_string() };
            if title.contains("OpenAtat") {
                ours.push(w.retain());
            }
        }
        NSArray::from_retained_slice(&ours)
    }

    fn cgimage_png(image: *mut std::ffi::c_void) -> std::result::Result<Vec<u8>, String> {
        unsafe {
            let w = CGImageGetWidth(image) as u32;
            let h = CGImageGetHeight(image) as u32;
            let bpr = CGImageGetBytesPerRow(image);
            let bpp = CGImageGetBitsPerPixel(image);
            if w == 0 || h == 0 || bpp < 24 {
                return Err("CGImage has no pixels".into());
            }
            let provider = CGImageGetDataProvider(image);
            if provider.is_null() {
                return Err("CGImage has no data provider".into());
            }
            let data = CGDataProviderCopyData(provider);
            if data.is_null() {
                return Err("CGDataProviderCopyData failed".into());
            }
            let len = CFDataGetLength(data) as usize;
            let ptr = CFDataGetBytePtr(data);
            let bytes = std::slice::from_raw_parts(ptr, len);
            let spp = (bpp / 8).max(3) as usize;
            let mut rgba = vec![0u8; (w * h * 4) as usize];
            for y in 0..h as usize {
                let row = &bytes[y * bpr..];
                for x in 0..w as usize {
                    let s = x * spp;
                    let d = (y * w as usize + x) * 4;
                    if s + 2 < row.len() && d + 3 < rgba.len() {
                        // SCK typically delivers BGRA.
                        rgba[d] = row[s + 2];
                        rgba[d + 1] = row[s + 1];
                        rgba[d + 2] = row[s];
                        rgba[d + 3] = if spp > 3 { row[s + 3] } else { 255 };
                    }
                }
            }
            CFRelease(data);
            let img = image::RgbaImage::from_raw(w, h, rgba)
                .ok_or_else(|| "RGBA size mismatch".to_string())?;
            let mut png = Vec::new();
            image::DynamicImage::ImageRgba8(img)
                .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                .map_err(|e| e.to_string())?;
            Ok(png)
        }
    }
}

#[cfg(target_os = "windows")]
mod windows;

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
    fn windows_c1_source_never_uses_picker_or_activates() {
        let src = include_str!("windows.rs");
        assert!(
            !src.contains("GraphicsCapturePicker::") && !src.contains("GraphicsCapturePicker {"),
            "auto-attach must not construct a system capture picker"
        );
        assert!(
            !src.contains("SetForegroundWindow("),
            "C1 must not steal the foreground"
        );
        assert!(src.contains("CreateForMonitor"));
        assert!(src.contains("WDA_EXCLUDEFROMCAPTURE") || src.contains("exclude"));
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
