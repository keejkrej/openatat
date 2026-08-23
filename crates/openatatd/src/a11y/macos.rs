//! macOS a11y stub. P2: `AXSelectedText` / `AXSelectedTextRange` on the
//! focused `AXUIElement`. Re-probe secure input / `AXSecureTextField` every
//! time (never cache). Mouse-up only (`NSEventTypeLeftMouseUp`); shift+arrow
//! keyboard selections must not summon the bar.

use super::SelectionProbe;
use crate::error::{Error, Result};
use crate::trigger::FieldKind;

pub fn probe_field_kind() -> FieldKind {
    FieldKind::Other
}

pub fn probe_selection() -> SelectionProbe {
    SelectionProbe::Unavailable
}

pub fn insert_into_focused_field() -> Result<bool> {
    Err(Error::msg(
        "macOS insert is a stub: AXUIElement (Accessibility)",
    ))
}
