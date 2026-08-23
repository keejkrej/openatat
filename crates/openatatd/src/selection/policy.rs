//! Display-free C10 rules: mouse-up vs keyboard, secure skip, ephemeral prompts.

use crate::trigger::FieldKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionOrigin {
    /// Left-button release. The only origin that may summon the bar.
    MouseUp,
    /// Shift+arrow / keyboard caret selection. Never summons.
    Keyboard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummonDecision {
    ShowBar,
    SkipSecure,
    SkipKeyboard,
    SkipEmpty,
    /// AT-SPI did not expose a selection (browsers/Electron often). No clipboard dance.
    SkipNoAtspi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarAction {
    Ask,
    Copy,
    Search,
    Summarize,
    Explain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptAction {
    Ask,
    Summarize,
    Explain,
}

impl BarAction {
    pub fn prompt_action(self) -> Option<PromptAction> {
        match self {
            Self::Ask => Some(PromptAction::Ask),
            Self::Summarize => Some(PromptAction::Summarize),
            Self::Explain => Some(PromptAction::Explain),
            Self::Copy | Self::Search => None,
        }
    }

    pub fn runs_immediately(self) -> bool {
        matches!(self, Self::Copy | Self::Search)
    }
}

pub fn decide_summon(
    origin: SelectionOrigin,
    field: FieldKind,
    text: Option<&str>,
) -> SummonDecision {
    if field == FieldKind::Secure {
        return SummonDecision::SkipSecure;
    }
    if origin != SelectionOrigin::MouseUp {
        return SummonDecision::SkipKeyboard;
    }
    match text {
        None => SummonDecision::SkipNoAtspi,
        Some(t) if t.is_empty() => SummonDecision::SkipEmpty,
        Some(_) => SummonDecision::ShowBar,
    }
}

/// History row. Never the selected text — only the user prompt or the canned name.
pub fn history_prompt_for(action: PromptAction, user_prompt: &str) -> String {
    match action {
        PromptAction::Ask => user_prompt.to_string(),
        PromptAction::Summarize => "Summarize".into(),
        PromptAction::Explain => "Explain".into(),
    }
}

/// What we send to the BYO CLI. Includes the ephemeral selection.
pub fn launch_prompt_for(action: PromptAction, user_prompt: &str, selection: &str) -> String {
    let instruction = match action {
        PromptAction::Ask if !user_prompt.is_empty() => user_prompt,
        PromptAction::Ask => "Answer using the selected text.",
        PromptAction::Summarize => "Summarize the selected text.",
        PromptAction::Explain => "Explain the selected text.",
    };
    format!("{instruction}\n\nSelected text:\n{selection}")
}

pub fn search_url(query: &str) -> String {
    format!("https://duckduckgo.com/?q={}", encode_www_form(query))
}

fn encode_www_form(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_selection_never_summons() {
        assert_eq!(
            decide_summon(
                SelectionOrigin::Keyboard,
                FieldKind::AcceptsText,
                Some("hello")
            ),
            SummonDecision::SkipKeyboard
        );
    }

    #[test]
    fn mouse_up_with_text_summons() {
        assert_eq!(
            decide_summon(
                SelectionOrigin::MouseUp,
                FieldKind::AcceptsText,
                Some("hello")
            ),
            SummonDecision::ShowBar
        );
    }

    #[test]
    fn secure_skipped_on_every_probe() {
        assert_eq!(
            decide_summon(SelectionOrigin::MouseUp, FieldKind::Secure, Some("hunter2")),
            SummonDecision::SkipSecure
        );
        // A later probe in a normal field is independent — no cache.
        assert_eq!(
            decide_summon(SelectionOrigin::MouseUp, FieldKind::AcceptsText, Some("ok")),
            SummonDecision::ShowBar
        );
    }

    #[test]
    fn empty_and_missing_atspi_skip() {
        assert_eq!(
            decide_summon(SelectionOrigin::MouseUp, FieldKind::AcceptsText, Some("")),
            SummonDecision::SkipEmpty
        );
        assert_eq!(
            decide_summon(SelectionOrigin::MouseUp, FieldKind::AcceptsText, None),
            SummonDecision::SkipNoAtspi
        );
    }

    #[test]
    fn history_never_contains_selected_text() {
        let selected = "SECRET_SELECTION_XYZ";
        assert_eq!(
            history_prompt_for(PromptAction::Summarize, selected),
            "Summarize"
        );
        assert_eq!(
            history_prompt_for(PromptAction::Explain, selected),
            "Explain"
        );
        assert_eq!(
            history_prompt_for(PromptAction::Ask, "make this nicer"),
            "make this nicer"
        );
        assert!(!history_prompt_for(PromptAction::Ask, "make this nicer").contains(selected));
        let launch = launch_prompt_for(PromptAction::Summarize, "", selected);
        assert!(launch.contains(selected), "{launch}");
        assert!(launch.contains("Summarize"), "{launch}");
    }

    #[test]
    fn copy_and_search_are_immediate() {
        assert!(BarAction::Copy.runs_immediately());
        assert!(BarAction::Search.runs_immediately());
        assert!(!BarAction::Ask.runs_immediately());
        assert!(!BarAction::Summarize.runs_immediately());
    }

    #[test]
    fn search_url_encodes_without_logging() {
        let url = search_url("hello world & you");
        assert!(url.starts_with("https://duckduckgo.com/?q="));
        assert!(url.contains("hello+world"));
        assert!(url.contains("%26"));
    }
}
