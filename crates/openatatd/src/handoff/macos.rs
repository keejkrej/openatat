//! macOS handoff stub. P2 — do not block Linux.
//!
//! Later: `NSWorkspace.shared.openApplication(at:configuration:)`
//! (`NSWorkspace.OpenConfiguration`: `arguments`, `currentDirectoryURL`,
//! `activates`). Prefer that over `open -a Terminal`, which is a shell-ish
//! string. iTerm2 can take an argv via the same NSWorkspace path.

use super::HandoffPlan;
use crate::error::{Error, Result};

pub fn spawn(_plan: &HandoffPlan) -> Result<()> {
    Err(Error::msg(
        "macOS handoff is a stub: NSWorkspace openApplication (not gpui)",
    ))
}
