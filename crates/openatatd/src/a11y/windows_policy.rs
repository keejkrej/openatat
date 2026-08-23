//! Windows UIA selection / field-kind / insert-identity policy.
//! Runs on every OS so CI can lock it.
//!
//! `IsPassword` / `ES_PASSWORD` are re-probed every call. Keyboard
//! selections never summon (that gate lives in `selection::policy`). If
//! UIA TextPattern exposes no selected text, skip — no clipboard dance.
//! Browsers paste once; they are not typed per-key.

use super::{Rect, SelectionProbe, TextSelection};
use crate::trigger::FieldKind;

/// UIA / Win32 snapshot used by the live probe and by tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiaFieldSnap {
    pub is_password: bool,
    pub es_password: bool,
    pub control_type: UiaControl,
    pub class_name: String,
    pub has_value_pattern: bool,
    pub has_text_pattern: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiaControl {
    Edit,
    Document,
    ComboBox,
    Text,
    Pane,
    Other,
}

pub fn role_is_secure(is_password: bool, es_password: bool) -> bool {
    is_password || es_password
}

pub fn control_accepts_text(control: UiaControl, class_name: &str) -> bool {
    match control {
        UiaControl::Edit | UiaControl::Document | UiaControl::ComboBox | UiaControl::Text => true,
        UiaControl::Pane | UiaControl::Other => class_looks_like_edit(class_name),
    }
}

pub fn class_looks_like_edit(class_name: &str) -> bool {
    let c = class_name.trim();
    c.eq_ignore_ascii_case("Edit")
        || c.eq_ignore_ascii_case("RichEdit")
        || c.eq_ignore_ascii_case("RichEdit20A")
        || c.eq_ignore_ascii_case("RichEdit20W")
        || c.eq_ignore_ascii_case("RichEdit50W")
        || c.eq_ignore_ascii_case("RICHEDIT50W")
        || c.eq_ignore_ascii_case("Scintilla")
        || c.contains("Edit")
        || c.contains("Chrome_WidgetWin")
        || c.contains("MozillaWindowClass")
}

pub fn interpret_uia_field(snap: &UiaFieldSnap) -> FieldKind {
    if role_is_secure(snap.is_password, snap.es_password) {
        FieldKind::Secure
    } else if control_accepts_text(snap.control_type, &snap.class_name)
        || snap.has_value_pattern
        || snap.has_text_pattern
    {
        FieldKind::AcceptsText
    } else {
        FieldKind::Other
    }
}

/// Interpret one UIA TextPattern snapshot. Missing selection is skip
/// (not a clipboard dance).
pub fn interpret_uia_selection(
    snap: &UiaFieldSnap,
    selected: Option<&str>,
    range: Option<(i32, i32)>,
    bounds: Option<Rect>,
) -> SelectionProbe {
    if role_is_secure(snap.is_password, snap.es_password) {
        return SelectionProbe::Secure;
    }
    if !control_accepts_text(snap.control_type, &snap.class_name) && !snap.has_text_pattern {
        return SelectionProbe::Unavailable;
    }
    match selected {
        None => SelectionProbe::Unavailable,
        Some(t) if t.is_empty() => SelectionProbe::NoSelection,
        Some(t) => {
            let (start, end) = range.unwrap_or((0, t.chars().count() as i32));
            if start == end {
                return SelectionProbe::NoSelection;
            }
            SelectionProbe::Ready(TextSelection::from_parts(
                t.to_string(),
                start,
                end,
                bounds,
                None,
            ))
        }
    }
}

/// Processes that should receive a single Ctrl+V paste instead of
/// ValuePattern / per-key typing.
pub fn is_browser_process(exe_or_class: &str) -> bool {
    let s = exe_or_class.trim().to_ascii_lowercase();
    let name = s.rsplit(['\\', '/']).next().unwrap_or(&s);
    matches!(
        name,
        "chrome.exe"
            | "msedge.exe"
            | "msedgewebview2.exe"
            | "firefox.exe"
            | "brave.exe"
            | "opera.exe"
            | "vivaldi.exe"
            | "chromium.exe"
            | "iexplore.exe"
            | "waterfox.exe"
            | "librewolf.exe"
    ) || s.contains("chrome")
        || s.contains("msedge")
        || s.contains("firefox")
        || s.contains("mozilla")
        || class_is_browser(&s)
}

fn class_is_browser(class: &str) -> bool {
    class.contains("chrome_widgetwin")
        || class.contains("mozilla")
        || class.contains("edge")
}

/// Encode foreground HWND + UIA RuntimeId for insert abort.
pub fn encode_win_identity(hwnd: isize, runtime_id: &[i32]) -> String {
    let rid = runtime_id
        .iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(".");
    format!("hwnd:{hwnd:#x}/rid:{rid}")
}

pub fn parse_win_identity(s: &str) -> Option<(isize, String)> {
    let rest = s.strip_prefix("hwnd:")?;
    let (hwnd, rid) = rest.split_once("/rid:")?;
    let hwnd = isize::from_str_radix(hwnd.trim_start_matches("0x"), 16).ok()?;
    Some((hwnd, rid.to_string()))
}

/// Abort insert when the foreground HWND or the UIA runtime id moved.
pub fn win_identity_changed(
    expected_hwnd: Option<isize>,
    expected_runtime: Option<&str>,
    now_hwnd: Option<isize>,
    now_runtime: Option<&str>,
) -> bool {
    if let (Some(a), Some(b)) = (expected_hwnd, now_hwnd) {
        if a != b {
            return true;
        }
    }
    if let (Some(a), Some(b)) = (expected_runtime, now_runtime) {
        if !a.is_empty() && !b.is_empty() && a != b {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit_snap() -> UiaFieldSnap {
        UiaFieldSnap {
            is_password: false,
            es_password: false,
            control_type: UiaControl::Edit,
            class_name: "Edit".into(),
            has_value_pattern: true,
            has_text_pattern: true,
        }
    }

    #[test]
    fn password_skips_even_if_selected_text_is_present() {
        let mut snap = edit_snap();
        snap.is_password = true;
        let probe = interpret_uia_selection(&snap, Some("hunter2"), Some((0, 7)), None);
        assert_eq!(probe, SelectionProbe::Secure);
        assert_eq!(interpret_uia_field(&snap), FieldKind::Secure);
    }

    #[test]
    fn es_password_is_secure_without_uia_flag() {
        let mut snap = edit_snap();
        snap.es_password = true;
        assert_eq!(interpret_uia_field(&snap), FieldKind::Secure);
        assert_eq!(
            interpret_uia_selection(&snap, Some("x"), Some((0, 1)), None),
            SelectionProbe::Secure
        );
    }

    #[test]
    fn missing_uia_selection_is_unavailable_not_clipboard() {
        let probe = interpret_uia_selection(&edit_snap(), None, None, None);
        assert_eq!(probe, SelectionProbe::Unavailable);
    }

    #[test]
    fn empty_selection_skips() {
        let probe = interpret_uia_selection(&edit_snap(), Some(""), Some((3, 3)), None);
        assert_eq!(probe, SelectionProbe::NoSelection);
    }

    #[test]
    fn ready_keeps_text_and_range() {
        match interpret_uia_selection(&edit_snap(), Some("hello"), Some((0, 5)), None) {
            SelectionProbe::Ready(sel) => {
                assert_eq!(sel.text(), "hello");
                assert_eq!(sel.range(), (0, 5));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn browsers_paste_not_type() {
        assert!(is_browser_process("C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe"));
        assert!(is_browser_process("msedge.exe"));
        assert!(is_browser_process("firefox.exe"));
        assert!(is_browser_process("Chrome_WidgetWin_1"));
        assert!(!is_browser_process("notepad.exe"));
        assert!(!is_browser_process("explorer.exe"));
    }

    #[test]
    fn identity_abort_when_hwnd_or_runtime_moves() {
        assert!(win_identity_changed(
            Some(0x10),
            Some("1.2.3"),
            Some(0x11),
            Some("1.2.3"),
        ));
        assert!(win_identity_changed(
            Some(0x10),
            Some("1.2.3"),
            Some(0x10),
            Some("9.9.9"),
        ));
        assert!(!win_identity_changed(
            Some(0x10),
            Some("1.2.3"),
            Some(0x10),
            Some("1.2.3"),
        ));
    }

    #[test]
    fn windows_sources_never_activate_or_pick() {
        let overlay = include_str!("../overlay/windows.rs");
        let trigger = include_str!("../trigger/windows.rs");
        let capture = include_str!("../capture/windows.rs");
        let runtime = include_str!("../windows_runtime.rs");
        for (name, src) in [
            ("overlay", overlay),
            ("trigger", trigger),
            ("capture", capture),
            ("runtime", runtime),
        ] {
            assert!(
                !src.contains("SetForegroundWindow("),
                "{name} must never call SetForegroundWindow"
            );
            assert!(
                !src.contains("GraphicsCapturePicker::") && !src.contains("GraphicsCapturePicker {"),
                "{name} must not construct a system capture picker"
            );
        }
        assert!(overlay.contains("WS_EX_NOACTIVATE"));
        assert!(overlay.contains("WDA_EXCLUDEFROMCAPTURE"));
        assert!(capture.contains("CreateForMonitor"));
    }

    #[test]
    fn encode_identity_roundtrip() {
        let id = encode_win_identity(0xabc, &[42, 7, 1]);
        let (hwnd, rid) = parse_win_identity(&id).unwrap();
        assert_eq!(hwnd, 0xabc);
        assert_eq!(rid, "42.7.1");
    }
}
