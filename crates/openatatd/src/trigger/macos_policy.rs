//! macOS `@@` trigger policy. Display-free so Linux CI can lock the contract.
//!
//! Listen-only event tap feeds [`crate::trigger::ImeFilter`]. Secure input
//! (`IsSecureEventInputEnabled` / `AXSecureTextField`) is re-probed every key
//! and never cached. IME composing / marked text does not touch the buffer.
//! Swallowing `@@` is AX replace — not a password field, not a synthetic
//! backspace stream.

use crate::trigger::{FieldKind, ImeAction, ImeFilter};

/// One key from a listen-only tap. Callers must fill this from a fresh probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacKeyInput {
    /// `IsSecureEventInputEnabled()` this event. Never reuse yesterday's value.
    pub secure_event_input: bool,
    /// Focused AX role/subrole is a secure field this event.
    pub ax_secure: bool,
    /// IME preedit / marked text / dead-key compose.
    pub composing: bool,
    /// Committed unicode (empty for modifiers / function keys).
    pub committed: Option<String>,
}

impl MacKeyInput {
    pub fn field_kind(&self) -> FieldKind {
        field_kind(self.secure_event_input, self.ax_secure)
    }
}

pub fn field_kind(secure_event_input: bool, ax_secure: bool) -> FieldKind {
    if secure_event_input || ax_secure {
        FieldKind::Secure
    } else {
        FieldKind::AcceptsText
    }
}

/// IME / marked-text gate. Empty unicode during an input method is compose.
pub fn is_ime_composing(committed: Option<&str>, marked_text: bool, input_method_active: bool) -> bool {
    if marked_text {
        return true;
    }
    match committed {
        None | Some("") if input_method_active => true,
        _ => false,
    }
}

/// Feed the shared [`ImeFilter`]. Secure is applied first, every key.
pub fn feed_filter(filter: &mut ImeFilter, input: &MacKeyInput) -> ImeAction {
    let kind = input.field_kind();
    filter.on_key(kind, input.composing, input.committed.as_deref())
}

/// AX swallow is allowed only after a trigger in a non-secure field.
pub fn should_swallow_trigger(kind: FieldKind, action: ImeAction) -> bool {
    action == ImeAction::FireTrigger && kind != FieldKind::Secure
}

/// Strip a trailing `@@` / `＠＠` from an AX value. `None` if there is nothing
/// to swallow (caller still opens the overlay).
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

/// How to tell the user to grant Input Monitoring when the tap is missing.
pub fn input_monitoring_hint() -> &'static str {
    "openatatd: Input Monitoring is not granted — the listen-only @@ tap is idle. \
     System Settings → Privacy & Security → Input Monitoring → enable openatatd. \
     --demo and the unix trigger socket still work."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secure_event_input_skips_every_key_never_cached() {
        let mut ime = ImeFilter::new();
        let first = MacKeyInput {
            secure_event_input: false,
            ax_secure: false,
            composing: false,
            committed: Some("@".into()),
        };
        assert_eq!(feed_filter(&mut ime, &first), ImeAction::Continue);
        let password = MacKeyInput {
            secure_event_input: true,
            ax_secure: false,
            composing: false,
            committed: Some("@".into()),
        };
        assert_eq!(feed_filter(&mut ime, &password), ImeAction::Ignore);
        assert_eq!(ime.buffer().len(), 0);
        let again = MacKeyInput {
            secure_event_input: false,
            ax_secure: false,
            composing: false,
            committed: Some("@".into()),
        };
        assert_eq!(feed_filter(&mut ime, &again), ImeAction::Continue);
        assert!(!ime.buffer().is_trigger());
    }

    #[test]
    fn ax_secure_text_field_is_also_secure() {
        let input = MacKeyInput {
            secure_event_input: false,
            ax_secure: true,
            composing: false,
            committed: Some("@@".into()),
        };
        assert_eq!(input.field_kind(), FieldKind::Secure);
        let mut ime = ImeFilter::new();
        assert_eq!(feed_filter(&mut ime, &input), ImeAction::Ignore);
        assert!(!should_swallow_trigger(input.field_kind(), ImeAction::FireTrigger));
    }

    #[test]
    fn last_two_char_fires_and_clears() {
        let mut ime = ImeFilter::new();
        let at = MacKeyInput {
            secure_event_input: false,
            ax_secure: false,
            composing: false,
            committed: Some("@".into()),
        };
        assert_eq!(feed_filter(&mut ime, &at), ImeAction::Continue);
        assert_eq!(feed_filter(&mut ime, &at), ImeAction::FireTrigger);
        assert_eq!(ime.buffer().len(), 0);
        assert!(should_swallow_trigger(FieldKind::AcceptsText, ImeAction::FireTrigger));
    }

    #[test]
    fn composing_marked_text_does_not_enter_buffer() {
        let mut ime = ImeFilter::new();
        let compose = MacKeyInput {
            secure_event_input: false,
            ax_secure: false,
            composing: true,
            committed: Some("@@".into()),
        };
        assert_eq!(feed_filter(&mut ime, &compose), ImeAction::Ignore);
        assert_eq!(ime.buffer().len(), 0);
        assert!(is_ime_composing(Some(""), true, false));
        assert!(is_ime_composing(None, false, true));
        assert!(!is_ime_composing(Some("@"), false, true));
    }

    #[test]
    fn swallow_strips_trailing_ats_only() {
        assert_eq!(strip_trailing_trigger("hello@@").as_deref(), Some("hello"));
        assert_eq!(strip_trailing_trigger("hello＠＠").as_deref(), Some("hello"));
        assert_eq!(strip_trailing_trigger("hello@").as_deref(), None);
        assert_eq!(strip_trigger_before_caret("ab@@cd", 4).as_deref(), Some("abcd"));
        assert_eq!(strip_trigger_before_caret("ab@@cd", 2), None);
    }

    #[test]
    fn hint_names_input_monitoring() {
        let h = input_monitoring_hint();
        assert!(h.contains("Input Monitoring"));
        assert!(h.contains("--demo"));
    }
}
