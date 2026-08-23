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
}
