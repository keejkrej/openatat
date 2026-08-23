//! Native overlay. Not gpui.
//!
//! Linux: one compositor client hosts both the `@@` popover and the C10
//! selection bar (`zwlr_layer_shell_v1` + `wl_shm`). Idle maps no overlay
//! surface; the Orb is a separate layer-shell surface. macOS: NSPanel
//! nonactivating. Windows: `WS_EX_NOACTIVATE` popover.
//! Session / Tab / R / handoff live in [`controller`].

use crate::capture::Still;
use crate::error::Result;
use crate::session::Session;

#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
pub(crate) mod controller;
mod draw;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayEnd {
    Cancelled,
    Tab,
    /// Copy or Search completed; field was not modified.
    Copied,
    /// Super+Return / Handoff opened a terminal session.
    Handoff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayKind {
    Prompt,
    SelectionBar,
}

pub fn run(session: &mut Session) -> Result<OverlayEnd> {
    run_kind(session, OverlayKind::Prompt)
}

pub fn run_bar(session: &mut Session) -> Result<OverlayEnd> {
    run_kind(session, OverlayKind::SelectionBar)
}

fn run_kind(session: &mut Session, kind: OverlayKind) -> Result<OverlayEnd> {
    #[cfg(target_os = "linux")]
    {
        return linux::run(session, kind);
    }
    #[cfg(target_os = "macos")]
    {
        return macos::run(session, kind);
    }
    #[cfg(target_os = "windows")]
    {
        return windows::run(session, kind);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = (session, kind);
        Err(crate::error::Error::msg("overlay unsupported on this OS"))
    }
}

pub fn still_thumb(still: &Still) -> Result<(u32, u32, Vec<u8>)> {
    still.thumbnail_argb(120, 64)
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_overlay_hwnd() -> Option<isize> {
    windows::overlay_hwnd()
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_overlay_wants_keys() -> bool {
    windows::overlay_wants_keys()
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_feed_vk(vk: u32, scan: u32) {
    windows::feed_vk(vk, scan)
}
