//! Native overlay. Not gpui. Mac/Win are cfg-gated nonactivating stubs.

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
}

pub fn run(session: &mut Session) -> Result<OverlayEnd> {
    #[cfg(target_os = "linux")]
    {
        return linux::run(session);
    }
    #[cfg(target_os = "macos")]
    {
        return macos::run(session);
    }
    #[cfg(target_os = "windows")]
    {
        return windows::run(session);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = session;
        Err(crate::error::Error::msg("overlay unsupported on this OS"))
    }
}

pub fn still_thumb(still: &Still) -> Result<(u32, u32, Vec<u8>)> {
    still.thumbnail_argb(120, 64)
}
