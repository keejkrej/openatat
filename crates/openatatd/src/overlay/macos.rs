//! macOS overlay stub. P2: NSPanel + NSWindowStyleMaskNonactivatingPanel.
//! CollectionBehavior: canJoinAllSpaces, fullScreenAuxiliary.
//! Do not use an NSWindow that can become key. gpui-ce PopUp is not this.

use super::OverlayEnd;
use crate::error::Result;
use crate::session::Session;

pub fn run(_session: &mut Session) -> Result<OverlayEnd> {
    Err(crate::error::Error::msg(
        "macOS overlay is a stub: NSPanel nonactivating (not gpui)",
    ))
}
