//! Platform accessibility. Linux talks AT-SPI; macOS talks AXUIElement.
//!
//! Password / secure roles are re-probed on every call. Never cache a field
//! kind on a window. Selection reads use GetText + caret/selection offsets
//! when the focused accessible is a text role. If AT-SPI exposes no
//! selection, we skip — no clipboard save/restore dance (browsers/Electron
//! often fail here). Nautilus has no selection D-Bus API; this module is
//! text, not the file manager.

use crate::error::Result;
use crate::trigger::FieldKind;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
pub(crate) mod macos;
#[cfg(target_os = "windows")]
pub(crate) mod windows;
mod macos_policy;
mod windows_policy;

pub use macos_policy::{
    encode_element_id, interpret_ax_field, interpret_ax_selection, is_browser_bundle,
    mac_identity_changed, parse_element_id, role_is_secure,
};
pub use windows_policy::{
    encode_win_identity, interpret_uia_field, interpret_uia_selection, is_browser_process,
    parse_win_identity, win_identity_changed, UiaControl, UiaFieldSnap,
};

/// Screen-coordinate box for placing the selection bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Focused-field text selection. `Debug` redacts the characters.
#[derive(Clone, PartialEq, Eq)]
pub struct TextSelection {
    text: String,
    pub start: i32,
    pub end: i32,
    pub bounds: Option<Rect>,
    /// Tiny HTML when AT-SPI text attributes look styled; otherwise none.
    html: Option<String>,
}

impl TextSelection {
    pub fn from_parts(
        text: String,
        start: i32,
        end: i32,
        bounds: Option<Rect>,
        html: Option<String>,
    ) -> Self {
        Self {
            text,
            start,
            end,
            bounds,
            html,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn html(&self) -> Option<&str> {
        self.html.as_deref()
    }

    pub fn range(&self) -> (i32, i32) {
        (self.start, self.end)
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty() || self.start == self.end
    }
}

impl std::fmt::Debug for TextSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextSelection")
            .field("chars", &self.text.chars().count())
            .field("start", &self.start)
            .field("end", &self.end)
            .field("has_html", &self.html.is_some())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionProbe {
    /// Password / secure role. Do not read text.
    Secure,
    /// Focused accessible is not a text role, or AT-SPI is missing.
    Unavailable,
    /// Text role but GetNSelections is 0 / offsets empty. Skip; no clipboard.
    NoSelection,
    Ready(TextSelection),
}

impl SelectionProbe {
    pub fn is_secure(&self) -> bool {
        matches!(self, Self::Secure)
    }
}

/// AT-SPI `Role` values used by the Linux probe (and unit tests).
pub const ROLE_PASSWORD_TEXT: u32 = 40;
pub const ROLE_TEXT: u32 = 61;
pub const ROLE_TERMINAL: u32 = 60;
pub const ROLE_ENTRY: u32 = 79;
pub const ROLE_EDITBAR: u32 = 77;
pub const ROLE_DOCUMENT_TEXT: u32 = 94;

/// Interpret a single AT-SPI snapshot. Pure: no bus, no display.
pub fn interpret_selection_snapshot(
    role: u32,
    interfaces: &[String],
    n_selections: Option<i32>,
    range: Option<(i32, i32)>,
    text: Option<String>,
    bounds: Option<Rect>,
    html: Option<String>,
) -> SelectionProbe {
    if role == ROLE_PASSWORD_TEXT {
        return SelectionProbe::Secure;
    }
    if !is_text_field(role, interfaces) {
        return SelectionProbe::Unavailable;
    }
    match n_selections {
        None => return SelectionProbe::Unavailable,
        Some(n) if n <= 0 => return SelectionProbe::NoSelection,
        Some(_) => {}
    }
    let Some((start, end)) = range else {
        return SelectionProbe::NoSelection;
    };
    if start == end {
        return SelectionProbe::NoSelection;
    }
    let Some(text) = text else {
        // Toolkit advertised a selection but GetText failed — treat as
        // "AT-SPI cannot expose" rather than a clipboard fallback.
        return SelectionProbe::Unavailable;
    };
    if text.is_empty() {
        return SelectionProbe::NoSelection;
    }
    SelectionProbe::Ready(TextSelection::from_parts(text, start, end, bounds, html))
}

pub fn is_text_field(role: u32, interfaces: &[String]) -> bool {
    interfaces
        .iter()
        .any(|i| i.contains("EditableText") || i.contains("Text"))
        || matches!(
            role,
            ROLE_TEXT | ROLE_ENTRY | ROLE_TERMINAL | ROLE_EDITBAR | ROLE_DOCUMENT_TEXT
        )
}

pub fn probe_field_kind() -> FieldKind {
    #[cfg(target_os = "linux")]
    {
        linux::probe_field_kind()
    }
    #[cfg(target_os = "macos")]
    {
        macos::probe_field_kind()
    }
    #[cfg(target_os = "windows")]
    {
        windows::probe_field_kind()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        FieldKind::Other
    }
}

/// Re-probes the focused accessible. Never uses a cached role.
pub fn probe_selection() -> SelectionProbe {
    #[cfg(target_os = "linux")]
    {
        linux::probe_selection()
    }
    #[cfg(target_os = "macos")]
    {
        macos::probe_selection()
    }
    #[cfg(target_os = "windows")]
    {
        windows::probe_selection()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        SelectionProbe::Unavailable
    }
}

pub fn insert_into_focused_field(text: &str) -> Result<bool> {
    #[cfg(target_os = "linux")]
    {
        linux::insert_into_focused_field(text)
    }
    #[cfg(target_os = "macos")]
    {
        macos::insert_into_focused_field(text)
    }
    #[cfg(target_os = "windows")]
    {
        windows::insert_into_focused_field(text)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = text;
        Ok(false)
    }
}

/// Delete `[start, end)` then insert at `start`. No synthetic backspaces.
pub fn replace_range(text: &str, start: i32, end: i32) -> Result<bool> {
    #[cfg(target_os = "linux")]
    {
        linux::replace_range(text, start, end)
    }
    #[cfg(target_os = "macos")]
    {
        macos::replace_range(text, start, end)
    }
    #[cfg(target_os = "windows")]
    {
        windows::replace_range(text, start, end)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = (text, start, end);
        insert_into_focused_field(text)
    }
}

/// Pointer position from the mouse-up that triggered a probe.
#[derive(Debug, Clone)]
pub struct MouseUpHit {
    pub probe: SelectionProbe,
    pub pointer: Option<(i32, i32)>,
}

#[cfg(target_os = "linux")]
pub use linux::spawn_mouse_up_watcher;

#[cfg(target_os = "macos")]
pub use macos::spawn_mouse_up_watcher;

#[cfg(target_os = "windows")]
pub use windows::spawn_mouse_up_watcher;

#[cfg(target_os = "macos")]
pub use macos::{macos_has_marked_text, macos_swallow_trigger};

#[cfg(target_os = "windows")]
pub use windows::{
    explorer_root_hwnd, foreground_class, foreground_exe, foreground_title, windows_es_password,
    windows_password_flags, windows_swallow_trigger,
};

/// AT-SPI `Event.Mouse::Button` detail for left-button release (`mouse:b1r`).
pub fn mouse_button_is_left_release(detail: &str) -> bool {
    let d = detail.trim().to_ascii_lowercase();
    d == "1r" || d == "b1r" || d.ends_with(":1r") || d.ends_with("b1r") || d == "1:r"
}

/// Left-button press. Used so tests can model drag vs mouse-up.
pub fn mouse_button_is_left_press(detail: &str) -> bool {
    let d = detail.trim().to_ascii_lowercase();
    d == "1p" || d == "b1p" || d.ends_with(":1p") || d.ends_with("b1p") || d == "1:p"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_role_is_secure_even_if_text_is_present() {
        let probe = interpret_selection_snapshot(
            ROLE_PASSWORD_TEXT,
            &["org.a11y.atspi.Text".into()],
            Some(1),
            Some((0, 8)),
            Some("hunter2".into()),
            None,
            None,
        );
        assert_eq!(probe, SelectionProbe::Secure);
    }

    #[test]
    fn no_selections_skips_without_inventing_clipboard() {
        let probe = interpret_selection_snapshot(
            ROLE_ENTRY,
            &["org.a11y.atspi.EditableText".into()],
            Some(0),
            None,
            None,
            None,
            None,
        );
        assert_eq!(probe, SelectionProbe::NoSelection);
    }

    #[test]
    fn get_text_failure_is_unavailable_not_clipboard() {
        let probe = interpret_selection_snapshot(
            ROLE_TEXT,
            &["org.a11y.atspi.Text".into()],
            Some(1),
            Some((0, 4)),
            None,
            None,
            None,
        );
        assert_eq!(probe, SelectionProbe::Unavailable);
    }

    #[test]
    fn ready_selection_keeps_offsets() {
        let probe = interpret_selection_snapshot(
            ROLE_ENTRY,
            &["org.a11y.atspi.EditableText".into()],
            Some(1),
            Some((2, 6)),
            Some("abcd".into()),
            Some(Rect {
                x: 10,
                y: 20,
                width: 40,
                height: 12,
            }),
            None,
        );
        match probe {
            SelectionProbe::Ready(sel) => {
                assert_eq!(sel.text(), "abcd");
                assert_eq!(sel.range(), (2, 6));
                assert_eq!(sel.bounds.unwrap().y, 20);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn debug_redacts_characters() {
        let sel = TextSelection::from_parts("secret-token".into(), 0, 12, None, None);
        let dbg = format!("{sel:?}");
        assert!(!dbg.contains("secret-token"), "{dbg}");
        assert!(dbg.contains("chars: 12"), "{dbg}");
    }

    #[test]
    fn mouse_up_details_from_atspi() {
        assert!(mouse_button_is_left_release("1r"));
        assert!(mouse_button_is_left_release("b1r"));
        assert!(mouse_button_is_left_press("1p"));
        assert!(!mouse_button_is_left_release("1p"));
        assert!(!mouse_button_is_left_release("2r"));
    }
}
