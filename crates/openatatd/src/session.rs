use openatat_ipc::{EntryPoint, FocusSnapshot, TriggerSource};

use crate::agent::{self, Attachment, Launch};
use crate::capture::{self, Still};
use crate::error::Result;
use crate::focus;
use crate::insert::{self, InsertOutcome};
use crate::overlay::{self, OverlayEnd};

#[derive(Debug, Clone)]
pub struct Session {
    pub source: TriggerSource,
    pub entry: EntryPoint,
    pub focus: FocusSnapshot,
    pub still: Option<Still>,
    pub prompt: String,
    pub preview: Option<String>,
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
        Ok(OverlayEnd::Tab) => finish_tab(&session),
        Err(e) if e.is_wayland_connect() => {
            eprintln!("openatatd: overlay unavailable ({e}); headless fallback");
            run_headless_on(session)
        }
        Err(e) => Err(e),
    }
}

pub fn run_headless(source: TriggerSource, prompt: &str) -> Result<SessionEnd> {
    let mut session = Session::begin(source);
    session.prompt = prompt.to_string();
    run_headless_on(session)
}

fn run_headless_on(mut session: Session) -> Result<SessionEnd> {
    if session.prompt.is_empty() {
        session.prompt =
            std::env::var("OPENATAT_PROMPT").unwrap_or_else(|_| "hello from openatat".into());
    }
    crate::history::append_prompt(session.entry, &session.prompt)?;
    let attachments = session_attachments(&session);
    let preview = agent::run_launch(&Launch {
        prompt: &session.prompt,
        attachments: &attachments,
        resolve: None,
        copy_text: crate::clipboard::copy_text,
    })?;
    session.preview = Some(preview);
    eprintln!(
        "openatatd: preview (headless):\n{}",
        session.preview.as_deref().unwrap_or("")
    );
    if std::env::var_os("OPENATAT_INSERT").is_some() {
        finish_tab(&session)
    } else {
        Ok(SessionEnd::CopiedOnly)
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
    match insert::tab_insert(&text, &session.focus)? {
        InsertOutcome::Inserted => Ok(SessionEnd::Inserted),
        InsertOutcome::CopiedOnly => Ok(SessionEnd::CopiedOnly),
        InsertOutcome::AbortedFocusChanged => Ok(SessionEnd::AbortedFocusChanged),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_dummy_writes_history_and_preview() {
        let dir = std::env::temp_dir().join(format!("openatat-sess-{}", std::process::id()));
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
}
