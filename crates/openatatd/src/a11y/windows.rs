//! Windows a11y stub. P2: `IUIAutomationTextPattern.GetSelection()` /
//! `TextPattern` ranges. Re-probe `IsPassword` / Win32 password edit every
//! time (never cache). Mouse-up only; keyboard (shift+arrow) selections
//! must not summon the bar.

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
    Err(Error::msg("Windows insert is a stub: UI Automation"))
}
