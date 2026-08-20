//! Windows overlay stub. P2: WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST.
//! Do not call SetForegroundWindow. Do not use a normal WS_OVERLAPPEDWINDOW.

use super::OverlayEnd;
use crate::error::Result;
use crate::session::Session;

pub fn run(_session: &mut Session) -> Result<OverlayEnd> {
    Err(crate::error::Error::msg(
        "Windows overlay is a stub: WS_EX_NOACTIVATE (not gpui)",
    ))
}
