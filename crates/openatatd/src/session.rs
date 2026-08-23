use openatat_ipc::{EntryPoint, FocusSnapshot, TriggerSource};

use crate::a11y::TextSelection;
use crate::agent::{self, Attachment, Launch};
use crate::capture::{self, Still};
use crate::error::Result;
use crate::focus;
use crate::insert::{self, InsertOutcome};
use crate::overlay::{self, OverlayEnd};
use crate::selection::{self, Placement, PromptAction};

#[derive(Debug, Clone)]
pub struct Session {
    pub source: TriggerSource,
    pub entry: EntryPoint,
    pub focus: FocusSnapshot,
    pub still: Option<Still>,
    pub prompt: String,
    pub preview: Option<String>,
    /// Ephemeral. Never written to history and never logged.
    pub selection: Option<TextSelection>,
    pub placement: Option<Placement>,
}

impl Session {
    pub fn begin(source: TriggerSource) -> Self {
        let focus = focus::snapshot();
        let still = capture::capture_active_output(focus.output.as_deref()).ok();
        let entry = match source {
            TriggerSource::Demo => EntryPoint::Demo,
            TriggerSource::Ime => EntryPoint::TextField,
        };
        Self {
            source,
            entry,
            focus,
            still,
            prompt: String::new(),
            preview: None,
            selection: None,
            placement: None,
        }
    }

    pub fn begin_selection(sel: TextSelection, pointer: Option<(i32, i32)>) -> Self {
        let focus = focus::snapshot();
        let placement = Some(selection::placement_for(
            sel.bounds,
            pointer.or_else(focus::cursor_pos),
            focus::focused_output(),
        ));
        Self {
            source: TriggerSource::Demo,
            entry: EntryPoint::TextField,
            focus,
            still: None,
            prompt: String::new(),
            preview: None,
            selection: Some(sel),
            placement,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEnd {
    Cancelled,
    Inserted,
    CopiedOnly,
    AbortedFocusChanged,
}

pub fn run_interactive(source: TriggerSource) -> Result<SessionEnd> {
    let mut session = Session::begin(source);
    match overlay::run(&mut session) {
        Ok(OverlayEnd::Cancelled) => Ok(SessionEnd::Cancelled),
        Ok(OverlayEnd::Copied) => Ok(SessionEnd::CopiedOnly),
        Ok(OverlayEnd::Tab) => finish_tab(&session),
        Err(e) if e.is_wayland_connect() => {
            eprintln!("openatatd: overlay unavailable ({e}); headless fallback");
            run_headless_on(session)
        }
        Err(e) => Err(e),
    }
}

pub fn run_selection_bar(sel: TextSelection, pointer: Option<(i32, i32)>) -> Result<SessionEnd> {
    let mut session = Session::begin_selection(sel, pointer);
    match overlay::run_bar(&mut session) {
        Ok(OverlayEnd::Cancelled) => Ok(SessionEnd::Cancelled),
        Ok(OverlayEnd::Copied) => Ok(SessionEnd::CopiedOnly),
        Ok(OverlayEnd::Tab) => finish_tab(&session),
        Err(e) if e.is_wayland_connect() => {
            eprintln!("openatatd: selection bar unavailable ({e})");
            Ok(SessionEnd::Cancelled)
        }
        Err(e) => Err(e),
    }
}

pub fn run_headless(source: TriggerSource, prompt: &str) -> Result<SessionEnd> {
    let mut session = Session::begin(source);
    session.prompt = prompt.to_string();
    run_headless_on(session)
}

/// Headless C10 action. Selected text is used for the CLI and never stored.
pub fn run_headless_selection(
    action: PromptAction,
    selected: &str,
    user_prompt: &str,
) -> Result<SessionEnd> {
    let sel = TextSelection::from_parts(selected.to_string(), 0, selected.len() as i32, None, None);
    let mut session = Session::begin_selection(sel, None);
    session.prompt = match action {
        PromptAction::Ask => user_prompt.to_string(),
        PromptAction::Summarize => "Summarize".into(),
        PromptAction::Explain => "Explain".into(),
    };
    run_headless_on(session)
}

fn run_headless_on(mut session: Session) -> Result<SessionEnd> {
    if session.prompt.is_empty() {
        session.prompt =
            std::env::var("OPENATAT_PROMPT").unwrap_or_else(|_| "hello from openatat".into());
    }
    let history_prompt = if session.selection.is_some() {
        let action = prompt_action_from_session_prompt(&session.prompt);
        selection::history_prompt_for(action, &session.prompt)
    } else {
        session.prompt.clone()
    };
    crate::history::append_prompt(session.entry, &history_prompt)?;
    let attachments = session_attachments(&session);
    let launch_prompt = if let Some(sel) = session.selection.as_ref() {
        let action = prompt_action_from_session_prompt(&session.prompt);
        selection::launch_prompt_for(action, &session.prompt, sel.text())
    } else {
        session.prompt.clone()
    };
    let preview = agent::run_launch(&Launch {
        prompt: &launch_prompt,
        attachments: &attachments,
        resolve: None,
        copy_text: crate::clipboard::copy_text,
    })?;
    session.preview = Some(preview);
    if session.selection.is_some() {
        let n = session.preview.as_ref().map(|s| s.len()).unwrap_or(0);
        eprintln!("openatatd: preview ready ({n} chars, selection not logged)");
    } else {
        eprintln!(
            "openatatd: preview (headless):\n{}",
            session.preview.as_deref().unwrap_or("")
        );
    }
    if std::env::var_os("OPENATAT_INSERT").is_some() {
        finish_tab(&session)
    } else {
        Ok(SessionEnd::CopiedOnly)
    }
}

fn prompt_action_from_session_prompt(prompt: &str) -> PromptAction {
    match prompt {
        "Summarize" => PromptAction::Summarize,
        "Explain" => PromptAction::Explain,
        _ => PromptAction::Ask,
    }
}

pub fn session_attachments(session: &Session) -> Vec<Attachment> {
    session
        .still
        .as_ref()
        .map(|s| Attachment::Still { png: s.png.clone() })
        .into_iter()
        .collect()
}

fn finish_tab(session: &Session) -> Result<SessionEnd> {
    let text = session
        .preview
        .clone()
        .ok_or_else(|| crate::error::Error::msg("preview card is empty"))?;
    let range = session.selection.as_ref().map(|s| s.range());
    match insert::tab_insert_replace(&text, &session.focus, range)? {
        InsertOutcome::Inserted => Ok(SessionEnd::Inserted),
        InsertOutcome::CopiedOnly => Ok(SessionEnd::CopiedOnly),
        InsertOutcome::AbortedFocusChanged => Ok(SessionEnd::AbortedFocusChanged),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static XDG_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn headless_dummy_writes_history_and_preview() {
        let _g = XDG_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("openatat-sess-{}", uuid::Uuid::new_v4()));
        std::env::set_var("XDG_DATA_HOME", &dir);
        std::env::remove_var("OPENATAT_AGENT");
        std::env::remove_var("OPENATAT_INSERT");
        let cfg = dir.join("config");
        std::fs::create_dir_all(cfg.join("openatat")).unwrap();
        std::fs::write(cfg.join("openatat/agent.toml"), "provider = \"dummy\"\n").unwrap();
        std::env::set_var("XDG_CONFIG_HOME", &cfg);
        std::env::set_var("XDG_CACHE_HOME", dir.join("cache"));
        let end = run_headless(TriggerSource::Demo, "friendlier").unwrap();
        assert_eq!(end, SessionEnd::CopiedOnly);
        let hist = std::fs::read_to_string(dir.join("openatat/history.jsonl")).unwrap();
        assert!(hist.contains("friendlier"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn selection_is_ephemeral_not_written_to_history() {
        let _g = XDG_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("openatat-selhist-{}", uuid::Uuid::new_v4()));
        std::env::set_var("XDG_DATA_HOME", &dir);
        std::env::remove_var("OPENATAT_AGENT");
        std::env::remove_var("OPENATAT_INSERT");
        let cfg = dir.join("config");
        std::fs::create_dir_all(cfg.join("openatat")).unwrap();
        std::fs::write(cfg.join("openatat/agent.toml"), "provider = \"dummy\"\n").unwrap();
        std::env::set_var("XDG_CONFIG_HOME", &cfg);
        std::env::set_var("XDG_CACHE_HOME", dir.join("cache"));
        let secret = "SECRET_SELECTION_XYZ";
        let end = run_headless_selection(PromptAction::Summarize, secret, "").unwrap();
        assert_eq!(end, SessionEnd::CopiedOnly);
        let hist = std::fs::read_to_string(dir.join("openatat/history.jsonl")).unwrap();
        assert!(
            !hist.contains(secret),
            "selected text must not be stored: {hist}"
        );
        assert!(hist.contains("Summarize"), "{hist}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
