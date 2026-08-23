//! Session / preview / Tab / R / handoff state machine.
//!
//! Windowing stays native (layer-shell on Linux, NSPanel on Mac). This module
//! is the product logic both hosts call.

use std::path::PathBuf;

use openatat_ipc::EntryPoint;

use super::draw::{self, Frame, Phase, BAR_H, BAR_W, POPOVER_H, POPOVER_W};
use super::{OverlayEnd, OverlayKind};
use crate::a11y::TextSelection;
use crate::agent::{self, Attachment, Launch, RefineSession};
use crate::handoff::{self, Handoff};
use crate::history;
use crate::selection::{self, PromptAction};
use crate::session::Session;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayKey {
    Escape,
    Return { meta: bool },
    Tab,
    Backspace,
    R,
    Digit(u8),
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayEffect {
    Redraw,
    Resize { w: u32, h: u32 },
}

pub struct OverlayController {
    pub prompt: String,
    pub preview: String,
    pub refine: String,
    pub refine_session: Option<RefineSession>,
    pub template_display: String,
    pub still_png: Option<Vec<u8>>,
    pub phase: Phase,
    pub has_tile: bool,
    pub thumb: Option<(u32, u32, Vec<u8>)>,
    pub end: Option<OverlayEnd>,
    pub dirty: bool,
    pub entry: EntryPoint,
    pub selection: Option<TextSelection>,
    pub prompt_action: Option<PromptAction>,
    pub finder_cwd: Option<PathBuf>,
    pub finder_files: Vec<PathBuf>,
    pub width: u32,
    pub height: u32,
}

impl OverlayController {
    pub fn from_session(session: &Session, kind: OverlayKind) -> Self {
        let thumb = session
            .still
            .as_ref()
            .and_then(|s| s.thumbnail_argb(120, 64).ok());
        let (width, height) = match kind {
            OverlayKind::SelectionBar => (BAR_W, BAR_H),
            OverlayKind::Prompt => (POPOVER_W, POPOVER_H),
        };
        Self {
            prompt: String::new(),
            preview: String::new(),
            refine: String::new(),
            refine_session: None,
            template_display: agent::resolve_selected().display(),
            still_png: session.still.as_ref().map(|s| s.png.clone()),
            phase: match kind {
                OverlayKind::SelectionBar => Phase::Bar,
                OverlayKind::Prompt => Phase::Prompt,
            },
            has_tile: session.still.is_some(),
            thumb,
            end: None,
            dirty: true,
            entry: session.entry,
            selection: session.selection.clone(),
            prompt_action: None,
            finder_cwd: session.finder_cwd.clone(),
            finder_files: session.finder_files.clone(),
            width,
            height,
        }
    }

    pub fn write_back(&self, session: &mut Session) {
        session.prompt = self.prompt.clone();
        if !self.preview.is_empty() {
            session.preview = Some(self.preview.clone());
        }
        if !self.has_tile {
            session.still = None;
        }
        session.finder_cwd = self.finder_cwd.clone();
        session.finder_files = self.finder_files.clone();
    }

    pub fn ui_frame(&self) -> Frame {
        let status = match self.phase {
            Phase::Bar => "mouse selection · Ask / Copy / Search / Summarize / Explain",
            Phase::Prompt => {
                if self.has_tile {
                    "C1 still attached · click remove to drop it"
                } else {
                    "no still · capture missing or tile removed"
                }
            }
            Phase::Running => "scratch cwd · prompt passed as data",
            Phase::Preview => "result is not in your document until Tab · Super+Return / ⌘Return handoff",
            Phase::Refine => "same attachments · new result replaces the old",
        };
        let prompt = if self.phase == Phase::Refine {
            self.refine.clone()
        } else {
            self.prompt.clone()
        };
        let mut status = if self.phase == Phase::Running || self.phase == Phase::Prompt {
            format!("{} · {}", self.template_display, status)
        } else {
            status.to_string()
        };
        if !self.finder_files.is_empty() || self.finder_cwd.is_some() {
            let n = self.finder_files.len();
            let cwd = if self.finder_cwd.is_some() { 1 } else { 0 };
            status.push_str(&format!(" · Finder {cwd} cwd + {n} files"));
        }
        Frame {
            phase: self.phase,
            prompt,
            preview: self.preview.clone(),
            has_tile: self.has_tile,
            status,
        }
    }

    pub fn handle_key(&mut self, key: OverlayKey, text: Option<&str>) -> Vec<OverlayEffect> {
        match key {
            OverlayKey::Escape => {
                self.end = Some(OverlayEnd::Cancelled);
                return Vec::new();
            }
            OverlayKey::Digit(n) if self.phase == Phase::Bar => {
                let hit = match n {
                    1 => draw::BarHit::Ask,
                    2 => draw::BarHit::Copy,
                    3 => draw::BarHit::Search,
                    4 => draw::BarHit::Summarize,
                    5 => draw::BarHit::Explain,
                    _ => return Vec::new(),
                };
                return self.on_bar(hit);
            }
            OverlayKey::Return { meta: true } => {
                if self.phase != Phase::Bar && self.phase != Phase::Running {
                    return self.run_handoff();
                }
                return Vec::new();
            }
            OverlayKey::Return { meta: false } => {
                if self.phase == Phase::Prompt {
                    return self.run_agent(false);
                } else if self.phase == Phase::Refine {
                    return self.run_agent(true);
                }
                return Vec::new();
            }
            OverlayKey::Tab => {
                if self.phase == Phase::Preview {
                    self.end = Some(OverlayEnd::Tab);
                }
                return Vec::new();
            }
            OverlayKey::R if self.phase == Phase::Preview => {
                self.refine.clear();
                self.phase = Phase::Refine;
                self.dirty = true;
                return vec![OverlayEffect::Redraw];
            }
            OverlayKey::Backspace => {
                if self.phase == Phase::Prompt {
                    self.prompt.pop();
                    self.dirty = true;
                    return vec![OverlayEffect::Redraw];
                } else if self.phase == Phase::Refine {
                    self.refine.pop();
                    self.dirty = true;
                    return vec![OverlayEffect::Redraw];
                }
                return Vec::new();
            }
            OverlayKey::Text => {
                if let Some(txt) = text {
                    if !txt.is_empty() && !txt.chars().any(|c| c.is_control()) {
                        if self.phase == Phase::Prompt {
                            self.prompt.push_str(txt);
                            self.dirty = true;
                            return vec![OverlayEffect::Redraw];
                        } else if self.phase == Phase::Refine {
                            self.refine.push_str(txt);
                            self.dirty = true;
                            return vec![OverlayEffect::Redraw];
                        }
                    }
                }
                Vec::new()
            }
            OverlayKey::R | OverlayKey::Digit(_) => {
                if let Some(txt) = text {
                    return self.handle_key(OverlayKey::Text, Some(txt));
                }
                Vec::new()
            }
        }
    }

    pub fn handle_click(&mut self, x: f64, y: f64) -> Vec<OverlayEffect> {
        if self.phase == Phase::Bar {
            if let Some(hit) = draw::hit_bar(x, y, self.width) {
                return self.on_bar(hit);
            }
            return Vec::new();
        }
        if draw::hit_close(x, y, self.width) {
            self.end = Some(OverlayEnd::Cancelled);
            return Vec::new();
        }
        if draw::hit_handoff(x, y, self.phase) {
            return self.run_handoff();
        }
        if draw::hit_remove(x, y, self.has_tile) {
            self.has_tile = false;
            self.thumb = None;
            self.still_png = None;
            self.dirty = true;
            return vec![OverlayEffect::Redraw];
        }
        Vec::new()
    }

    pub fn on_bar(&mut self, hit: draw::BarHit) -> Vec<OverlayEffect> {
        match hit {
            draw::BarHit::Close => {
                self.end = Some(OverlayEnd::Cancelled);
                Vec::new()
            }
            draw::BarHit::Copy => {
                if let Some(sel) = self.held_selection() {
                    if selection::copy_selection(sel).is_ok() {
                        self.end = Some(OverlayEnd::Copied);
                    }
                }
                Vec::new()
            }
            draw::BarHit::Search => {
                if let Some(sel) = self.held_selection() {
                    if selection::search_selection(sel).is_ok() {
                        self.end = Some(OverlayEnd::Copied);
                    }
                }
                Vec::new()
            }
            draw::BarHit::Ask => {
                self.prompt_action = Some(PromptAction::Ask);
                let mut fx = self.grow_to_popover();
                if self.still_png.is_none() {
                    if let Ok(still) = crate::capture::capture_active_output(None) {
                        self.has_tile = true;
                        self.thumb = still.thumbnail_argb(120, 64).ok();
                        self.still_png = Some(still.png);
                    }
                }
                self.phase = Phase::Prompt;
                self.dirty = true;
                fx.push(OverlayEffect::Redraw);
                fx
            }
            draw::BarHit::Summarize => {
                self.prompt_action = Some(PromptAction::Summarize);
                self.prompt = "Summarize".into();
                let mut fx = self.grow_to_popover();
                fx.extend(self.run_agent(false));
                fx
            }
            draw::BarHit::Explain => {
                self.prompt_action = Some(PromptAction::Explain);
                self.prompt = "Explain".into();
                let mut fx = self.grow_to_popover();
                fx.extend(self.run_agent(false));
                fx
            }
        }
    }

    fn grow_to_popover(&mut self) -> Vec<OverlayEffect> {
        self.width = POPOVER_W;
        self.height = POPOVER_H;
        vec![OverlayEffect::Resize {
            w: POPOVER_W,
            h: POPOVER_H,
        }]
    }

    fn held_selection(&self) -> Option<&TextSelection> {
        if !selection::allow_held_selection() {
            return None;
        }
        self.selection.as_ref()
    }

    pub fn run_agent(&mut self, is_refine: bool) -> Vec<OverlayEffect> {
        if is_refine && self.refine.is_empty() {
            return Vec::new();
        }
        self.phase = Phase::Running;
        self.dirty = true;
        if self.prompt.is_empty() {
            self.prompt = "hello from openatat".to_string();
        }
        if self.refine_session.is_none() {
            self.refine_session = Some(RefineSession::new(self.prompt.clone()));
        }
        let Some(sess) = self.refine_session.as_mut() else {
            return vec![OverlayEffect::Redraw];
        };
        if is_refine {
            let sentence = if self.refine.is_empty() {
                return Vec::new();
            } else {
                self.refine.clone()
            };
            sess.apply_refine(sentence);
        }
        let history_prompt = if let Some(action) = self.prompt_action {
            selection::history_prompt_for(action, sess.history_prompt())
        } else {
            sess.history_prompt().to_string()
        };
        let mut launch_prompt = sess.launch_prompt();
        if let Some(sel) = self.held_selection() {
            launch_prompt = format!("{launch_prompt}\n\nSelected text:\n{}", sel.text());
        }
        let _ = history::append_prompt(self.entry, &history_prompt);
        let attachments = self.attachments();
        self.launch_agent(&launch_prompt, &attachments);
        vec![OverlayEffect::Redraw]
    }

    pub fn run_handoff(&mut self) -> Vec<OverlayEffect> {
        if self.phase == Phase::Refine && self.refine.is_empty() && self.refine_session.is_none() {
            return Vec::new();
        }
        if self.prompt.is_empty() {
            self.prompt = "hello from openatat".to_string();
        }
        if self.refine_session.is_none() {
            self.refine_session = Some(RefineSession::new(self.prompt.clone()));
        }
        let Some(sess) = self.refine_session.as_mut() else {
            return Vec::new();
        };
        if self.phase == Phase::Refine && !self.refine.is_empty() {
            sess.apply_refine(self.refine.clone());
        }
        let history_prompt = if let Some(action) = self.prompt_action {
            selection::history_prompt_for(action, sess.history_prompt())
        } else {
            sess.history_prompt().to_string()
        };
        let mut launch_prompt = sess.launch_prompt();
        if let Some(sel) = self.held_selection() {
            launch_prompt = format!("{launch_prompt}\n\nSelected text:\n{}", sel.text());
        }
        let _ = history::append_prompt(EntryPoint::Handoff, &history_prompt);
        let attachments = self.attachments();
        match handoff::run(&Handoff {
            prompt: &launch_prompt,
            attachments: &attachments,
            resolve: None,
            copy_text: crate::clipboard::copy_text,
            spawn: None,
        }) {
            Ok(_) => {
                self.end = Some(OverlayEnd::Handoff);
                Vec::new()
            }
            Err(e) => {
                self.preview = format!("handoff error: {e}");
                self.phase = Phase::Preview;
                let mut fx = self.grow_to_popover();
                self.dirty = true;
                fx.push(OverlayEffect::Redraw);
                fx
            }
        }
    }

    fn launch_agent(&mut self, launch_prompt: &str, attachments: &[Attachment]) {
        match agent::run_launch(&Launch {
            prompt: launch_prompt,
            attachments,
            resolve: None,
            copy_text: crate::clipboard::copy_text,
        }) {
            Ok(out) => {
                if let Some(sess) = self.refine_session.as_mut() {
                    sess.replace_preview(out.clone());
                }
                self.preview = out;
            }
            Err(e) => self.preview = format!("agent error: {e}"),
        }
        self.phase = Phase::Preview;
        self.dirty = true;
    }

    fn attachments(&self) -> Vec<Attachment> {
        let mut out = Vec::new();
        if self.has_tile {
            if let Some(png) = self.still_png.as_ref() {
                out.push(Attachment::Still { png: png.clone() });
            }
        }
        if let Some(cwd) = self.finder_cwd.as_ref() {
            out.push(Attachment::WorkingDir { path: cwd.clone() });
        }
        for path in &self.finder_files {
            out.push(Attachment::File { path: path.clone() });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::draw::Phase;
    use openatat_ipc::{FocusSnapshot, TriggerSource};

    fn empty_session() -> Session {
        Session {
            source: TriggerSource::Demo,
            entry: EntryPoint::Demo,
            focus: FocusSnapshot::default(),
            still: None,
            prompt: String::new(),
            preview: None,
            selection: None,
            placement: None,
            finder_cwd: None,
            finder_files: Vec::new(),
        }
    }

    #[test]
    fn escape_cancels() {
        let mut c = OverlayController::from_session(&empty_session(), OverlayKind::Prompt);
        c.handle_key(OverlayKey::Escape, None);
        assert_eq!(c.end, Some(OverlayEnd::Cancelled));
    }

    #[test]
    fn tab_only_from_preview() {
        let mut c = OverlayController::from_session(&empty_session(), OverlayKind::Prompt);
        c.handle_key(OverlayKey::Tab, None);
        assert!(c.end.is_none());
        c.phase = Phase::Preview;
        c.handle_key(OverlayKey::Tab, None);
        assert_eq!(c.end, Some(OverlayEnd::Tab));
    }

    #[test]
    fn r_enters_refine_from_preview() {
        let mut c = OverlayController::from_session(&empty_session(), OverlayKind::Prompt);
        c.phase = Phase::Preview;
        c.handle_key(OverlayKey::R, None);
        assert_eq!(c.phase, Phase::Refine);
    }

    #[test]
    fn typing_goes_to_prompt() {
        let mut c = OverlayController::from_session(&empty_session(), OverlayKind::Prompt);
        c.handle_key(OverlayKey::Text, Some("hi"));
        assert_eq!(c.prompt, "hi");
        c.handle_key(OverlayKey::Backspace, None);
        assert_eq!(c.prompt, "h");
    }
}
