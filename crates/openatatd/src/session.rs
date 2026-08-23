use std::path::PathBuf;

use openatat_ipc::{EntryPoint, FocusSnapshot, TriggerSource};

use crate::a11y::TextSelection;
use crate::agent::{self, Attachment, Launch};
use crate::capture::{self, Still};
use crate::error::Result;
use crate::finder::{self, FinderProbe};
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
    /// Finder insertion location. Only from Automation, never the title bar.
    pub finder_cwd: Option<PathBuf>,
    /// Finder selected files. Only from Automation, never the title bar.
    pub finder_files: Vec<PathBuf>,
    /// C16: files dropped on the Orb. Real paths from the drop protocol.
    pub dropped_files: Vec<PathBuf>,
    /// C16: text dropped on the Orb.
    pub dropped_text: Vec<String>,
}

impl Session {
    pub fn begin(source: TriggerSource) -> Self {
        let focus = focus::snapshot();
        let still = capture::capture_active_output(focus.output.as_deref()).ok();
        let (finder_cwd, finder_files, entry_fm) = match finder::probe(&focus) {
            FinderProbe::Ready { cwd, files } => (cwd, files, true),
            FinderProbe::Denied => {
                eprintln!(
                    "openatatd: file manager is frontmost but cwd/selection is unavailable — \
                     not guessing paths from the title bar."
                );
                (None, Vec::new(), false)
            }
            FinderProbe::NotFinder => (None, Vec::new(), false),
        };
        let entry = match source {
            TriggerSource::Demo => EntryPoint::Demo,
            TriggerSource::Ime if entry_fm => EntryPoint::FileManager,
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
            finder_cwd,
            finder_files,
            dropped_files: Vec::new(),
            dropped_text: Vec::new(),
        }
    }

    /// Orb click: empty prompt. No C1 still, no Finder tiles, nothing the
    /// user did not add. Typed `@@` still uses [`Self::begin`].
    pub fn begin_orb_click() -> Self {
        debug_assert!(!crate::orb::auto_attach_c1(EntryPoint::Orb));
        Self {
            source: TriggerSource::Demo,
            entry: EntryPoint::Orb,
            focus: focus::snapshot(),
            still: None,
            prompt: String::new(),
            preview: None,
            selection: None,
            placement: None,
            finder_cwd: None,
            finder_files: Vec::new(),
            dropped_files: Vec::new(),
            dropped_text: Vec::new(),
        }
    }

    /// Orb drop (C16). Still is only raw image bytes from the drag, never grim.
    pub fn begin_orb_drop(drops: Vec<crate::orb::OrbDrop>) -> Self {
        let mut session = Self::begin_orb_click();
        crate::orb::apply_drops(&mut session, drops);
        session
    }

    /// C2 / C4: reuse the Orb-click constructor and attach the captured PNG.
    /// Never calls [`capture::capture_active_output`] — that would be a second C1.
    pub fn begin_explicit_still(still: Still) -> Self {
        debug_assert!(!crate::capture::policy::explicit_still_auto_attaches_c1());
        let mut session = Self::begin_orb_click();
        session.still = Some(still);
        session
    }

    /// C14 shelf. No C1 still. Focus is captured for paste abort.
    pub fn begin_shelf() -> Self {
        Self {
            source: TriggerSource::Demo,
            entry: EntryPoint::Orb,
            focus: focus::snapshot(),
            still: None,
            prompt: String::new(),
            preview: None,
            selection: None,
            placement: None,
            finder_cwd: None,
            finder_files: Vec::new(),
            dropped_files: Vec::new(),
            dropped_text: Vec::new(),
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
            finder_cwd: None,
            finder_files: Vec::new(),
            dropped_files: Vec::new(),
            dropped_text: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEnd {
    Cancelled,
    Inserted,
    CopiedOnly,
    AbortedFocusChanged,
    HandedOff,
}

pub fn run_orb_click() -> Result<SessionEnd> {
    let mut session = Session::begin_orb_click();
    run_overlay_session(&mut session)
}

pub fn run_explicit_still(still: Still) -> Result<SessionEnd> {
    let mut session = Session::begin_explicit_still(still);
    run_overlay_session(&mut session)
}

pub fn run_orb_drop(drops: Vec<crate::orb::OrbDrop>) -> Result<SessionEnd> {
    let mut session = Session::begin_orb_drop(drops);
    run_overlay_session(&mut session)
}

pub fn run_interactive(source: TriggerSource) -> Result<SessionEnd> {
    let mut session = Session::begin(source);
    run_overlay_session(&mut session)
}

pub(crate) fn run_overlay_from_shelf(mut session: Session) -> Result<SessionEnd> {
    run_overlay_session(&mut session)
}

fn run_overlay_session(session: &mut Session) -> Result<SessionEnd> {
    match overlay::run(session) {
        Ok(OverlayEnd::Cancelled) => Ok(SessionEnd::Cancelled),
        Ok(OverlayEnd::Copied) | Ok(OverlayEnd::Pasted) => Ok(SessionEnd::CopiedOnly),
        Ok(OverlayEnd::Handoff) => Ok(SessionEnd::HandedOff),
        Ok(OverlayEnd::Tab) => finish_tab(&session),
        Ok(OverlayEnd::ShelfAsk) => {
            let mut next = Session::begin_orb_click();
            next.dropped_text = session.dropped_text.clone();
            next.focus = session.focus.clone();
            run_overlay_session(&mut next)
        }
        Err(e) if e.is_wayland_connect() => {
            eprintln!("openatatd: overlay unavailable ({e}); headless fallback");
            run_headless_on(session.clone())
        }
        Err(e) => Err(e),
    }
}

pub fn run_selection_bar(sel: TextSelection, pointer: Option<(i32, i32)>) -> Result<SessionEnd> {
    let mut session = Session::begin_selection(sel, pointer);
    match overlay::run_bar(&mut session) {
        Ok(OverlayEnd::Cancelled) => Ok(SessionEnd::Cancelled),
        Ok(OverlayEnd::Copied) | Ok(OverlayEnd::Pasted) => Ok(SessionEnd::CopiedOnly),
        Ok(OverlayEnd::Handoff) => Ok(SessionEnd::HandedOff),
        Ok(OverlayEnd::Tab) => finish_tab(&session),
        Ok(OverlayEnd::ShelfAsk) => Ok(SessionEnd::Cancelled),
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

pub(crate) fn run_headless_session(session: Session) -> Result<SessionEnd> {
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
    let mut out = Vec::new();
    if let Some(s) = session.still.as_ref() {
        out.push(Attachment::Still { png: s.png.clone() });
    }
    if let Some(cwd) = session.finder_cwd.as_ref() {
        out.push(Attachment::WorkingDir { path: cwd.clone() });
    }
    for path in &session.finder_files {
        out.push(Attachment::File { path: path.clone() });
    }
    out.extend(crate::orb::policy::drop_attachments(session));
    out
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

    #[test]
    fn headless_dummy_writes_history_and_preview() {
        let _g = crate::paths::xdg_test_lock();
        let dir = std::env::temp_dir().join(format!("openatat-sess-{}", uuid::Uuid::new_v4()));
        std::env::set_var("XDG_DATA_HOME", &dir);
        std::env::remove_var("OPENATAT_AGENT");
        std::env::remove_var("OPENATAT_INSERT");
        let cfg = dir.join("config");
        std::fs::create_dir_all(cfg.join("openatat")).unwrap();
        std::fs::write(cfg.join("openatat/agent.toml"), "provider = \"dummy\"\n").unwrap();
        std::env::set_var("XDG_CONFIG_HOME", &cfg);
        std::env::set_var("XDG_CACHE_HOME", dir.join("cache"));
        let hist_path = dir.join("openatat/history.jsonl");
        let end = run_headless(TriggerSource::Demo, "friendlier").unwrap();
        assert_eq!(end, SessionEnd::CopiedOnly);
        let hist = std::fs::read_to_string(&hist_path).unwrap();
        assert!(hist.contains("friendlier"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn shelf_session_skips_c1_and_does_not_touch_history() {
        let s = Session::begin_shelf();
        assert!(s.still.is_none());
        assert!(s.dropped_text.is_empty());
        assert!(!crate::orb::auto_attach_c1(s.entry));
    }

    #[test]
    fn explicit_still_reuses_orb_constructor_without_c1() {
        let img = image::RgbaImage::new(8, 8);
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let still = Still::from_png(png).unwrap();
        let s = Session::begin_explicit_still(still);
        assert!(s.still.is_some());
        assert_eq!(s.entry, EntryPoint::Orb);
        assert!(!crate::orb::auto_attach_c1(s.entry));
        let src = include_str!("session.rs");
        let start = src.find("fn begin_explicit_still").expect("begin_explicit_still");
        let body = &src[start..start + 400];
        assert!(body.contains("begin_orb_click"));
        assert!(!body.contains("capture_active_output"));
    }

    #[test]
    fn orb_click_skips_still_and_typed_path_still_requests_c1() {
        let orb = Session::begin_orb_click();
        assert_eq!(orb.entry, EntryPoint::Orb);
        assert!(orb.still.is_none());
        assert!(orb.finder_files.is_empty());
        assert!(orb.dropped_files.is_empty());
        assert!(!crate::orb::auto_attach_c1(orb.entry));
        assert!(crate::orb::auto_attach_c1(EntryPoint::TextField));
    }

    #[test]
    fn selection_is_ephemeral_not_written_to_history() {
        let _g = crate::paths::xdg_test_lock();
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
        let hist_path = dir.join("openatat/history.jsonl");
        let end = run_headless_selection(PromptAction::Summarize, secret, "").unwrap();
        assert_eq!(end, SessionEnd::CopiedOnly);
        let hist = std::fs::read_to_string(&hist_path).unwrap();
        assert!(
            !hist.contains(secret),
            "selected text must not be stored: {hist}"
        );
        assert!(hist.contains("Summarize"), "{hist}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
