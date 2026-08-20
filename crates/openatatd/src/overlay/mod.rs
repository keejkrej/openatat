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

#[cfg(target_os = "macos")]
mod macos {
    use super::*;

    pub fn run(_session: &mut Session) -> Result<OverlayEnd> {
        // NSPanel + NSWindowStyleMaskNonactivatingPanel.
        // CollectionBehavior: canJoinAllSpaces, fullScreenAuxiliary.
        // Do not use an NSWindow that can become key in the normal app sense.
        // gpui-ce PopUp / focus:false is not this.
        Err(crate::error::Error::msg(
            "macOS overlay is a stub: NSPanel nonactivating (not gpui)",
        ))
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use super::*;

    pub fn run(_session: &mut Session) -> Result<OverlayEnd> {
        // WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST layered popup.
        // Do not call SetForegroundWindow. Do not use a normal WS_OVERLAPPEDWINDOW.
        Err(crate::error::Error::msg(
            "Windows overlay is a stub: WS_EX_NOACTIVATE (not gpui)",
        ))
    }
}
