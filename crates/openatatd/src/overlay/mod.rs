//! Native overlay. Not gpui. Mac/Win are cfg-gated nonactivating stubs.
//!
//! One compositor client hosts both the `@@` popover and the C10 selection
//! bar (`zwlr_layer_shell_v1` + `wl_shm`). Idle maps no surface.

use crate::capture::Still;
use crate::error::Result;
use crate::session::Session;

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
        let _ = kind;
        return macos::run(session);
    }
    #[cfg(target_os = "windows")]
    {
        let _ = kind;
        return windows::run(session);
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
