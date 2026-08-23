//! Windows `@@` trigger policy. Display-free so Linux CI can lock the contract.
//!
//! A process-local listener (low-level keyboard hook or Raw Input) feeds
//! [`crate::trigger::ImeFilter`]. UIA `IsPassword` and Win32 `ES_PASSWORD`
//! are re-probed every key and never cached. IME composition does not touch
//! the buffer. Swallowing `@@` is UIA replace or a single paste — never into
//! a password field, never a backspace stream.

use crate::trigger::{FieldKind, ImeAction, ImeFilter};

/// One key from the process-local listener. Callers must fill this from a
/// fresh probe — never reuse yesterday's password / IME state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WinKeyInput {
    /// UIA `IsPassword` this event.
    pub uia_is_password: bool,
    /// Focused Win32 edit has `ES_PASSWORD` this event.
    pub es_password: bool,
    /// IME composition / preedit (ImmGetCompositionString / VK_PROCESSKEY).
    pub composing: bool,
    /// Committed unicode (empty for modifiers / function keys).
    pub committed: Option<String>,
}

impl WinKeyInput {
    pub fn field_kind(&self) -> FieldKind {
        field_kind(self.uia_is_password, self.es_password)
    }
}

pub fn field_kind(uia_is_password: bool, es_password: bool) -> FieldKind {
    if uia_is_password || es_password {
        FieldKind::Secure
    } else {
        FieldKind::AcceptsText
    }
}

/// IME / composition gate. `VK_PROCESSKEY` or a non-empty composition string
/// is compose. Empty committed unicode during an open IME is also compose.
pub fn is_ime_composing(
    committed: Option<&str>,
    composition_string: bool,
    vk_process_key: bool,
    ime_open: bool,
) -> bool {
    if composition_string || vk_process_key {
        return true;
    }
    match committed {
        None | Some("") if ime_open => true,
        _ => false,
    }
}

/// Feed the shared [`ImeFilter`]. Secure is applied first, every key.
pub fn feed_filter(filter: &mut ImeFilter, input: &WinKeyInput) -> ImeAction {
    let kind = input.field_kind();
    filter.on_key(kind, input.composing, input.committed.as_deref())
}

/// Swallow is allowed only after a trigger in a non-secure field.
pub fn should_swallow_trigger(kind: FieldKind, action: ImeAction) -> bool {
    action == ImeAction::FireTrigger && kind != FieldKind::Secure
}

/// Strip a trailing `@@` / `＠＠` from a UIA Value. `None` if there is
/// nothing to swallow (caller still opens the overlay).
pub fn strip_trailing_trigger(value: &str) -> Option<String> {
    strip_trigger_before_caret(value, value.chars().count())
}

/// Remove the two trigger characters immediately before `caret` (char index).
pub fn strip_trigger_before_caret(value: &str, caret: usize) -> Option<String> {
    let chars: Vec<char> = value.chars().collect();
    if caret < 2 || caret > chars.len() {
        return None;
    }
    let a = chars[caret - 2];
    let b = chars[caret - 1];
    if !is_at(a) || !is_at(b) {
        return None;
    }
    let mut out: String = chars[..caret - 2].iter().collect();
    out.extend(chars[caret..].iter());
    Some(out)
}

fn is_at(ch: char) -> bool {
    ch == '@' || ch == '＠'
}

/// How to tell the user the hook / Raw Input listener is idle.
pub fn hook_install_hint() -> &'static str {
    "openatatd: process-local keyboard listener failed to install — the @@ \
     hook is idle. --demo and the trigger socket still work."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_skipped_every_key_never_cached() {
        let mut ime = ImeFilter::new();
        let first = WinKeyInput {
            uia_is_password: false,
            es_password: false,
            composing: false,
            committed: Some("@".into()),
        };
        assert_eq!(feed_filter(&mut ime, &first), ImeAction::Continue);
        let password = WinKeyInput {
            uia_is_password: true,
            es_password: false,
            composing: false,
            committed: Some("@".into()),
        };
        assert_eq!(feed_filter(&mut ime, &password), ImeAction::Ignore);
        assert_eq!(ime.buffer().len(), 0);
        let again = WinKeyInput {
            uia_is_password: false,
            es_password: false,
            composing: false,
            committed: Some("@".into()),
        };
        assert_eq!(feed_filter(&mut ime, &again), ImeAction::Continue);
        assert!(!ime.buffer().is_trigger());
    }

    #[test]
    fn es_password_is_also_secure() {
        let input = WinKeyInput {
            uia_is_password: false,
            es_password: true,
            composing: false,
            committed: Some("@@".into()),
        };
        assert_eq!(input.field_kind(), FieldKind::Secure);
        let mut ime = ImeFilter::new();
        assert_eq!(feed_filter(&mut ime, &input), ImeAction::Ignore);
        assert!(!should_swallow_trigger(
            input.field_kind(),
            ImeAction::FireTrigger
        ));
    }

    #[test]
    fn last_two_char_fires_and_clears() {
        let mut ime = ImeFilter::new();
        let at = WinKeyInput {
            uia_is_password: false,
            es_password: false,
            composing: false,
            committed: Some("@".into()),
        };
        assert_eq!(feed_filter(&mut ime, &at), ImeAction::Continue);
        assert_eq!(feed_filter(&mut ime, &at), ImeAction::FireTrigger);
        assert_eq!(ime.buffer().len(), 0);
        assert!(should_swallow_trigger(
            FieldKind::AcceptsText,
            ImeAction::FireTrigger
        ));
    }

    #[test]
    fn composing_does_not_enter_buffer() {
        let mut ime = ImeFilter::new();
        let compose = WinKeyInput {
            uia_is_password: false,
            es_password: false,
            composing: true,
            committed: Some("@@".into()),
        };
        assert_eq!(feed_filter(&mut ime, &compose), ImeAction::Ignore);
        assert_eq!(ime.buffer().len(), 0);
        assert!(is_ime_composing(Some(""), true, false, false));
        assert!(is_ime_composing(None, false, true, false));
        assert!(is_ime_composing(None, false, false, true));
        assert!(!is_ime_composing(Some("@"), false, false, true));
    }

    #[test]
    fn swallow_strips_trailing_ats_only() {
        assert_eq!(strip_trailing_trigger("hello@@").as_deref(), Some("hello"));
        assert_eq!(strip_trailing_trigger("hello＠＠").as_deref(), Some("hello"));
        assert_eq!(strip_trailing_trigger("hello@").as_deref(), None);
        assert_eq!(
            strip_trigger_before_caret("ab@@cd", 4).as_deref(),
            Some("abcd")
        );
        assert_eq!(strip_trigger_before_caret("ab@@cd", 2), None);
    }

    #[test]
    fn hint_keeps_demo_path() {
        let h = hook_install_hint();
        assert!(h.contains("--demo"));
        assert!(h.contains("socket"));
    }
}
