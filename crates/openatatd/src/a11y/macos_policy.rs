//! macOS AX selection / field-kind policy. Runs on every OS so CI can lock it.
//!
//! Secure roles are re-probed every call. Keyboard selections never summon
//! (that gate lives in `selection::policy`). If AX exposes no selected text,
//! skip — no clipboard save/restore.

use super::{Rect, SelectionProbe, TextSelection};
use crate::trigger::FieldKind;

/// AX role / subrole strings we treat as a password field.
pub fn role_is_secure(role: &str, subrole: Option<&str>) -> bool {
    let role = role.trim();
    if role.eq_ignore_ascii_case("AXSecureTextField") {
        return true;
    }
    if subrole.is_some_and(|s| s.eq_ignore_ascii_case("AXSecureTextField") || s.eq_ignore_ascii_case("AXSecureTextFieldSubrole"))
    {
        return true;
    }
    false
}

pub fn role_accepts_text(role: &str) -> bool {
    matches!(
        role.trim(),
        "AXTextField"
            | "AXTextArea"
            | "AXComboBox"
            | "AXSearchField"
            | "AXStaticText"
            | "AXText"
            | "AXWebArea"
            | "AXGroup"
            | "AXDocument"
    ) || role.contains("Text")
}

pub fn interpret_ax_field(role: &str, subrole: Option<&str>) -> FieldKind {
    if role_is_secure(role, subrole) {
        FieldKind::Secure
    } else if role_accepts_text(role) {
        FieldKind::AcceptsText
    } else {
        FieldKind::Other
    }
}

/// Interpret one AX snapshot. `selected` is `AXSelectedText`. Empty / missing
/// is skip (not a clipboard dance).
pub fn interpret_ax_selection(
    role: &str,
    subrole: Option<&str>,
    selected: Option<&str>,
    range: Option<(i32, i32)>,
    bounds: Option<Rect>,
) -> SelectionProbe {
    if role_is_secure(role, subrole) {
        return SelectionProbe::Secure;
    }
    if !role_accepts_text(role) {
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

/// Bundles that should receive a single Cmd+V paste instead of AX set-value.
pub fn is_browser_bundle(bundle: &str) -> bool {
    matches!(
        bundle.trim(),
        "com.apple.Safari"
            | "com.apple.SafariTechnologyPreview"
            | "com.google.Chrome"
            | "com.google.Chrome.canary"
            | "org.mozilla.firefox"
            | "org.mozilla.firefoxdeveloperedition"
            | "com.brave.Browser"
            | "com.microsoft.edgemac"
            | "com.operasoftware.Opera"
            | "company.thebrowser.Browser"
            | "com.kagi.orion"
            | "org.chromium.Chromium"
    ) || bundle.contains("Chrome")
        || bundle.contains("Chromium")
        || bundle.contains("Firefox")
        || bundle.contains("Safari")
}

/// Encode frontmost pid + focused AX identity for insert abort.
pub fn encode_element_id(pid: i32, role: &str, identifier: Option<&str>, pos: Option<(i32, i32)>) -> String {
    let ident = identifier.unwrap_or("-");
    match pos {
        Some((x, y)) => format!("{pid}:{role}:{ident}:{x},{y}"),
        None => format!("{pid}:{role}:{ident}"),
    }
}

pub fn parse_element_id(s: &str) -> Option<(i32, String)> {
    let (pid, rest) = s.split_once(':')?;
    Some((pid.parse().ok()?, rest.to_string()))
}

/// Abort insert when the frontmost app or the focused AX element moved.
pub fn mac_identity_changed(
    expected_pid: Option<i32>,
    expected_bundle: Option<&str>,
    expected_element: Option<&str>,
    now_pid: Option<i32>,
    now_bundle: Option<&str>,
    now_element: Option<&str>,
) -> bool {
    if let (Some(a), Some(b)) = (expected_pid, now_pid) {
        if a != b {
            return true;
        }
    }
    if let (Some(a), Some(b)) = (expected_bundle, now_bundle) {
        if !a.is_empty() && !b.is_empty() && a != b {
            return true;
        }
    }
    if let (Some(a), Some(b)) = (expected_element, now_element) {
        if !a.is_empty() && !b.is_empty() && a != b {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secure_role_skips_even_if_selected_text_is_present() {
        let probe = interpret_ax_selection(
            "AXSecureTextField",
            None,
            Some("hunter2"),
            Some((0, 7)),
            None,
        );
        assert_eq!(probe, SelectionProbe::Secure);
        assert_eq!(
            interpret_ax_field("AXTextField", Some("AXSecureTextField")),
            FieldKind::Secure
        );
    }

    #[test]
    fn missing_ax_selected_text_is_unavailable_not_clipboard() {
        let probe = interpret_ax_selection("AXTextField", None, None, None, None);
        assert_eq!(probe, SelectionProbe::Unavailable);
    }

    #[test]
    fn empty_selection_skips() {
        let probe = interpret_ax_selection("AXTextArea", None, Some(""), Some((3, 3)), None);
        assert_eq!(probe, SelectionProbe::NoSelection);
    }

    #[test]
    fn ready_keeps_text_and_range() {
        match interpret_ax_selection("AXTextField", None, Some("hello"), Some((0, 5)), None) {
            SelectionProbe::Ready(sel) => {
                assert_eq!(sel.text(), "hello");
                assert_eq!(sel.range(), (0, 5));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn browser_bundles_paste_not_type() {
        assert!(is_browser_bundle("com.apple.Safari"));
        assert!(is_browser_bundle("com.google.Chrome"));
        assert!(is_browser_bundle("org.mozilla.firefox"));
        assert!(!is_browser_bundle("com.apple.TextEdit"));
        assert!(!is_browser_bundle("com.apple.finder"));
    }

    #[test]
    fn identity_abort_when_pid_or_element_moves() {
        assert!(mac_identity_changed(
            Some(10),
            Some("com.apple.TextEdit"),
            Some("10:AXTextField:-:0,0"),
            Some(11),
            Some("com.apple.TextEdit"),
            Some("11:AXTextField:-:0,0"),
        ));
        assert!(mac_identity_changed(
            Some(10),
            Some("com.apple.TextEdit"),
            Some("10:AXTextField:-:0,0"),
            Some(10),
            Some("com.apple.Safari"),
            Some("10:AXTextField:-:0,0"),
        ));
        assert!(mac_identity_changed(
            Some(10),
            Some("com.apple.TextEdit"),
            Some("10:AXTextField:box:0,0"),
            Some(10),
            Some("com.apple.TextEdit"),
            Some("10:AXTextField:other:8,8"),
        ));
        assert!(!mac_identity_changed(
            Some(10),
            Some("com.apple.TextEdit"),
            Some("10:AXTextField:-:0,0"),
            Some(10),
            Some("com.apple.TextEdit"),
            Some("10:AXTextField:-:0,0"),
        ));
    }

    #[test]
    fn encode_element_id_roundtrip_pid() {
        let id = encode_element_id(42, "AXTextField", Some("email"), Some((10, 20)));
        let (pid, rest) = parse_element_id(&id).unwrap();
        assert_eq!(pid, 42);
        assert!(rest.contains("AXTextField"));
        assert!(rest.contains("email"));
    }
}
