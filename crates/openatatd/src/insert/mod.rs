//! Clipboard-first insert. Verify focus. AT-SPI when a text field is focused.

use openatat_ipc::FocusSnapshot;

use crate::a11y;
use crate::clipboard;
use crate::error::Result;
use crate::focus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertOutcome {
    Inserted,
    CopiedOnly,
    AbortedFocusChanged,
}

/// Order is mandatory: clipboard, then focus check, then a11y insert.
pub fn tab_insert(text: &str, expected: &FocusSnapshot) -> Result<InsertOutcome> {
    tab_insert_replace(text, expected, None)
}

/// Shelf Return: write the original offer first, then the same abort-on-focus
/// change as Tab. AT-SPI insert is plain; formatting stays on the clipboard.
pub fn paste_formatted(
    plain: &str,
    html: Option<&str>,
    rtf: Option<&str>,
    image: Option<&[u8]>,
    expected: &FocusSnapshot,
) -> Result<InsertOutcome> {
    clipboard::write_offer(plain, html, rtf, image)?;
    if focus::address_changed(expected) {
        return Ok(InsertOutcome::AbortedFocusChanged);
    }
    if plain.is_empty() {
        return Ok(InsertOutcome::CopiedOnly);
    }
    match platform_write(plain, None) {
        Ok(true) => Ok(InsertOutcome::Inserted),
        Ok(false) => Ok(InsertOutcome::CopiedOnly),
        Err(e) => {
            eprintln!(
                "openatatd: shelf insert failed after clipboard write ({e}); item is still copied"
            );
            Ok(InsertOutcome::CopiedOnly)
        }
    }
}

/// Replace `[start, end)` after confirm. Still clipboard-first. No backspaces.
pub fn tab_insert_replace(
    text: &str,
    expected: &FocusSnapshot,
    range: Option<(i32, i32)>,
) -> Result<InsertOutcome> {
    tab_insert_with(
        text,
        expected,
        range,
        clipboard::copy_text,
        focus::address_changed,
        platform_write,
    )
}

pub(crate) fn tab_insert_with(
    text: &str,
    expected: &FocusSnapshot,
    range: Option<(i32, i32)>,
    copy_text: fn(&str) -> Result<()>,
    address_changed: fn(&FocusSnapshot) -> bool,
    write: fn(&str, Option<(i32, i32)>) -> Result<bool>,
) -> Result<InsertOutcome> {
    copy_text(text)?;
    if address_changed(expected) {
        return Ok(InsertOutcome::AbortedFocusChanged);
    }
    match write(text, range) {
        Ok(true) => Ok(InsertOutcome::Inserted),
        Ok(false) => Ok(InsertOutcome::CopiedOnly),
        Err(e) => {
            eprintln!(
                "openatatd: insert failed after clipboard write ({e}); result is still copied"
            );
            Ok(InsertOutcome::CopiedOnly)
        }
    }
}

fn platform_write(text: &str, range: Option<(i32, i32)>) -> Result<bool> {
    if let Some((start, end)) = range {
        if a11y::probe_field_kind() == crate::trigger::FieldKind::Secure {
            return Ok(false);
        }
        return a11y::replace_range(text, start, end);
    }
    a11y::insert_into_focused_field(text)
}

/// Secure-field probe for the IME filter. Never cached by the caller.
pub fn probe_field_kind() -> crate::trigger::FieldKind {
    a11y::probe_field_kind()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ORDER: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn mock_copy(_: &str) -> Result<()> {
        ORDER.lock().unwrap().push("copy");
        Ok(())
    }

    fn mock_insert(_: &str, _: Option<(i32, i32)>) -> Result<bool> {
        ORDER.lock().unwrap().push("insert");
        Ok(true)
    }

    fn mock_replace(text: &str, range: Option<(i32, i32)>) -> Result<bool> {
        assert_eq!(range, Some((2, 6)));
        assert_eq!(text, "new");
        ORDER.lock().unwrap().push("insert");
        Ok(true)
    }

    fn never_changed(_: &FocusSnapshot) -> bool {
        false
    }

    fn always_changed(_: &FocusSnapshot) -> bool {
        true
    }

    #[test]
    fn abort_outcome_is_distinct() {
        assert_ne!(
            InsertOutcome::AbortedFocusChanged,
            InsertOutcome::CopiedOnly
        );
        assert_ne!(InsertOutcome::Inserted, InsertOutcome::CopiedOnly);
    }

    #[test]
    fn insert_is_clipboard_first() {
        let _g = TEST_LOCK.lock().unwrap();
        ORDER.lock().unwrap().clear();
        let expected = FocusSnapshot {
            window_address: Some("0x1".into()),
            ..FocusSnapshot::default()
        };
        let out =
            tab_insert_with("hi", &expected, None, mock_copy, never_changed, mock_insert).unwrap();
        assert_eq!(out, InsertOutcome::Inserted);
        assert_eq!(*ORDER.lock().unwrap(), vec!["copy", "insert"]);
    }

    #[test]
    fn replace_is_clipboard_first() {
        let _g = TEST_LOCK.lock().unwrap();
        ORDER.lock().unwrap().clear();
        let expected = FocusSnapshot::default();
        let out = tab_insert_with(
            "new",
            &expected,
            Some((2, 6)),
            mock_copy,
            never_changed,
            mock_replace,
        )
        .unwrap();
        assert_eq!(out, InsertOutcome::Inserted);
        assert_eq!(*ORDER.lock().unwrap(), vec!["copy", "insert"]);
    }

    #[test]
    fn focus_change_aborts_after_clipboard() {
        let _g = TEST_LOCK.lock().unwrap();
        ORDER.lock().unwrap().clear();
        let expected = FocusSnapshot {
            window_address: Some("0x1".into()),
            ..FocusSnapshot::default()
        };
        let out = tab_insert_with(
            "hi",
            &expected,
            None,
            mock_copy,
            always_changed,
            mock_insert,
        )
        .unwrap();
        assert_eq!(out, InsertOutcome::AbortedFocusChanged);
        assert_eq!(*ORDER.lock().unwrap(), vec!["copy"]);
    }

    #[test]
    fn mac_frontmost_and_ax_mismatch_aborts_after_clipboard() {
        let _g = TEST_LOCK.lock().unwrap();
        ORDER.lock().unwrap().clear();
        let expected = FocusSnapshot {
            pid: Some(10),
            app_id: Some("com.apple.TextEdit".into()),
            element_id: Some("10:AXTextField:-:0,0".into()),
            ..FocusSnapshot::default()
        };
        fn pid_moved(expected: &FocusSnapshot) -> bool {
            crate::a11y::mac_identity_changed(
                expected.pid,
                expected.app_id.as_deref(),
                expected.element_id.as_deref(),
                Some(11),
                expected.app_id.as_deref(),
                Some("11:AXTextField:-:0,0"),
            )
        }
        let out = tab_insert_with("hi", &expected, None, mock_copy, pid_moved, mock_insert).unwrap();
        assert_eq!(out, InsertOutcome::AbortedFocusChanged);
        assert_eq!(*ORDER.lock().unwrap(), vec!["copy"]);
    }

    #[test]
    fn win_hwnd_and_runtime_id_mismatch_aborts_after_clipboard() {
        let _g = TEST_LOCK.lock().unwrap();
        ORDER.lock().unwrap().clear();
        let expected = FocusSnapshot {
            window_address: Some("hwnd:0x10/rid:1.2.3".into()),
            element_id: Some("hwnd:0x10/rid:1.2.3".into()),
            ..FocusSnapshot::default()
        };
        fn hwnd_moved(expected: &FocusSnapshot) -> bool {
            crate::a11y::win_identity_changed(
                Some(0x10),
                expected.element_id.as_deref(),
                Some(0x11),
                Some("hwnd:0x11/rid:9.9.9"),
            )
        }
        let out =
            tab_insert_with("hi", &expected, None, mock_copy, hwnd_moved, mock_insert).unwrap();
        assert_eq!(out, InsertOutcome::AbortedFocusChanged);
        assert_eq!(*ORDER.lock().unwrap(), vec!["copy"]);
    }
}
