//! C7 video recording. Capture stays in `openatatd`.
//!
//! Linux: `wf-recorder` (preferred) or `gpu-screen-recorder`. Geometry is
//! argv data, never a shell string. macOS: ScreenCaptureKit stream + local
//! file (cfg). Windows: WGC + Media Foundation local file (cfg).
//! Overlay / picker / record chrome are never gpui.

use std::path::PathBuf;

use super::picker::{self, PickedRegion};
use crate::error::{Error, Result};
use crate::session::SessionEnd;

pub mod policy;

mod bar;

#[cfg(target_os = "linux")]
mod bar_linux;
#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "macos")]
mod bar_macos;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "windows")]
mod bar_windows;
#[cfg(target_os = "windows")]
mod windows;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarEnd {
    Stop,
    Cancel,
}

enum LiveRecording {
    #[cfg(target_os = "linux")]
    Child(linux::ChildRecording),
    #[cfg(target_os = "macos")]
    Mac(macos::MacRecording),
    #[cfg(target_os = "windows")]
    Win(windows::WinRecording),
}

impl LiveRecording {
    fn stop(self) -> Result<PathBuf> {
        match self {
            #[cfg(target_os = "linux")]
            Self::Child(c) => c.stop(),
            #[cfg(target_os = "macos")]
            Self::Mac(m) => m.stop(),
            #[cfg(target_os = "windows")]
            Self::Win(w) => w.stop(),
        }
    }

    fn cancel(self) -> Result<()> {
        match self {
            #[cfg(target_os = "linux")]
            Self::Child(c) => c.cancel(),
            #[cfg(target_os = "macos")]
            Self::Mac(m) => m.cancel(),
            #[cfg(target_os = "windows")]
            Self::Win(w) => w.cancel(),
        }
    }
}

/// C7 entry. Native picker → record → stop bar → file tile on `@@`.
/// Esc / empty drag on the picker cancels without recording. No second C1.
pub fn run_record() -> Result<SessionEnd> {
    match run_record_inner() {
        Ok(end) => Ok(end),
        Err(e) => {
            let msg = policy::useful_encoder_error(&e.to_string());
            let _ = crate::clipboard::copy_text(&msg);
            Err(Error::msg(msg))
        }
    }
}

fn run_record_inner() -> Result<SessionEnd> {
    let region = match picker::pick_region() {
        Ok(r) => r,
        Err(e) if e.is_wayland_connect() => {
            eprintln!("openatatd: native area picker unavailable ({e}); slurp fallback");
            picker::slurp_fallback()?
        }
        Err(e) => return Err(e),
    };
    let Some(region) = region else {
        return Ok(SessionEnd::Cancelled);
    };
    let id = uuid::Uuid::new_v4().to_string();
    let path = policy::record_output_path(&id);
    let rec = start_os_recording(&region, &path)?;
    match run_stop_bar() {
        Ok(BarEnd::Stop) => {
            let path = rec.stop()?;
            crate::session::run_explicit_file(path)
        }
        Ok(BarEnd::Cancel) => {
            rec.cancel()?;
            Ok(SessionEnd::Cancelled)
        }
        Err(e) if e.is_wayland_connect() => {
            eprintln!("openatatd: record bar unavailable ({e}); stopping recorder");
            let path = rec.stop()?;
            crate::session::run_explicit_file(path)
        }
        Err(e) => {
            let _ = rec.cancel();
            Err(e)
        }
    }
}

fn start_os_recording(region: &PickedRegion, path: &std::path::Path) -> Result<LiveRecording> {
    #[cfg(target_os = "linux")]
    {
        return linux::start_recording(region, path).map(LiveRecording::Child);
    }
    #[cfg(target_os = "macos")]
    {
        return macos::start_recording(region, path).map(LiveRecording::Mac);
    }
    #[cfg(target_os = "windows")]
    {
        return windows::start_recording(region, path).map(LiveRecording::Win);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = (region, path);
        Err(Error::msg("C7 recording unsupported on this OS"))
    }
}

fn run_stop_bar() -> Result<BarEnd> {
    #[cfg(target_os = "linux")]
    {
        return bar_linux::run();
    }
    #[cfg(target_os = "macos")]
    {
        return bar_macos::run();
    }
    #[cfg(target_os = "windows")]
    {
        return bar_windows::run();
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Ok(BarEnd::Stop)
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_bar_wants_keys() -> bool {
    bar_windows::bar_wants_keys()
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_feed_bar_vk(vk: u32) {
    bar_windows::feed_vk(vk)
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_bar_hwnd() -> Option<isize> {
    bar_windows::bar_hwnd()
}

#[cfg(not(target_os = "windows"))]
#[allow(dead_code)]
pub(crate) fn windows_bar_wants_keys() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_and_bar_sources_forbid_gpui_iced_and_activation() {
        let files = [
            include_str!("mod.rs"),
            include_str!("policy.rs"),
            include_str!("bar.rs"),
            include_str!("linux.rs"),
            include_str!("bar_linux.rs"),
            include_str!("macos.rs"),
            include_str!("windows.rs"),
            include_str!("bar_macos.rs"),
            include_str!("bar_windows.rs"),
        ];
        for src in files {
            let gpui = ["gpui", "::"].concat();
            let iced = ["iced", "::"].concat();
            let set_fg = ["SetForegroundWindow", "("].concat();
            assert!(!src.contains(&gpui), "C7 chrome must not use gpui");
            assert!(!src.contains(&iced), "C7 chrome must not use iced");
            assert!(!src.contains(&set_fg), "C7 must never activate");
            let stub = ["ScreenCapture", "Frame"].concat();
            assert!(!src.contains(&stub), "C7 must not use the gpui capture stub");
        }
    }

    #[test]
    fn recording_session_does_not_call_capture_active_output() {
        let src = include_str!("mod.rs");
        let start = src.find("fn run_record_inner").expect("run_record_inner");
        let rest = &src[start..];
        let end = rest.find("\nfn start_os_recording").unwrap_or(900);
        let body = &rest[..end];
        assert!(
            !body.contains("capture_active_output"),
            "C7 must not call C1 a second time: {body}"
        );
        assert!(body.contains("run_explicit_file"));
        assert!(!body.contains("Session::begin("));
        assert!(!body.contains("begin_explicit_still"));
    }

    #[test]
    fn c1_grim_still_has_no_portal_screenshot() {
        assert!(!policy::c1_may_use_portal_screenshot());
        assert_eq!(
            policy::record_portal_interface(),
            "org.freedesktop.portal.ScreenCast"
        );
        let grim = include_str!("../mod.rs");
        let start = grim.find("fn grim_to_file").expect("grim_to_file");
        let grim = &grim[start..start + 900];
        assert!(grim.contains("Command::new(\"grim\")"));
        assert!(!grim.contains("Screenshot"));
        assert!(!grim.contains("portal"));
    }

    #[test]
    fn windows_record_source_excludes_hwnds_and_skips_picker() {
        let src = include_str!("windows.rs");
        assert!(src.contains("CreateForMonitor"));
        assert!(src.contains("WDA_EXCLUDEFROMCAPTURE"));
        assert!(src.contains("Media Foundation") || src.contains("IMFSinkWriter"));
        let picker = ["GraphicsCapturePicker", "::"].concat();
        assert!(!src.contains(&picker) && !src.contains("GraphicsCapturePicker::"));
        let set_fg = ["SetForegroundWindow", "("].concat();
        assert!(!src.contains(&set_fg));
    }

    #[test]
    fn macos_record_source_is_sck_stream_not_window_list() {
        let src = include_str!("macos.rs");
        assert!(src.contains("SCStream"));
        assert!(src.contains("AVAssetWriter") || src.contains("AVFoundation"));
        let deprecated = ["CGWindow", "ListCreateImage"].concat();
        assert!(!src.contains(&deprecated));
    }
}
