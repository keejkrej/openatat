//! Windows handoff stub. P2 — do not block Linux.
//!
//! Later: `CreateProcessW` with `lpCurrentDirectory` = scratch and an argv
//! array (not `cmd.exe /c`). Windows Terminal: `wt.exe -d <scratch> -- <cli>`.

use super::HandoffPlan;
use crate::error::{Error, Result};

pub fn spawn(_plan: &HandoffPlan) -> Result<()> {
    Err(Error::msg(
        "Windows handoff is a stub: CreateProcessW (not gpui)",
    ))
}
