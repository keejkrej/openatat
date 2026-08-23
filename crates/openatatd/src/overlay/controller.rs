//! Session / preview / Tab / R / handoff state machine.
//!
//! Windowing stays native (layer-shell on Linux, NSPanel on Mac). This module
//! is the product logic both hosts call.

use std::path::PathBuf;

use openatat_ipc::EntryPoint;

use super::draw::{self, Frame, Phase, BAR_H, BAR_W, POPOVER_H, POPOVER_W, SHELF_H, SHELF_W};
use super::{OverlayEnd, OverlayKind};
use crate::a11y::TextSelection;
use crate::agent::{self, Attachment, Launch, RefineSession};
use crate::handoff::{self, Handoff};
use crate::history;
use crate::insert::InsertOutcome;
use crate::selection::{self, PromptAction};
use crate::session::Session;
use crate::shelf::{self, Clip, ShelfView};
use crate::studio_attach::StudioAttach;
use openatat_ipc::FocusSnapshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayKey {
    Escape,
    Return { meta: bool },
    Tab,
    Backspace,
    R,
    Digit(u8),
    Up,
    Down,
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
    pub dropped_files: Vec<PathBuf>,
    pub dropped_text: Vec<String>,
    pub studio: StudioAttach,
    pub width: u32,
    pub height: u32,
    pub focus: FocusSnapshot,
    pub shelf: Option<ShelfView>,
    paste_clip: fn(&Clip, &FocusSnapshot) -> crate::error::Result<InsertOutcome>,
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
            OverlayKind::Shelf => (SHELF_W, SHELF_H),
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
                OverlayKind::Shelf => Phase::Shelf,
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
            dropped_files: session.dropped_files.clone(),
            dropped_text: session.dropped_text.clone(),
            studio: StudioAttach::default(),
            width,
            height,
            focus: session.focus.clone(),
            shelf: matches!(kind, OverlayKind::Shelf).then(ShelfView::load),
            paste_clip: shelf::paste_clip,
        }
    }

    pub fn write_back(&self, session: &mut Session) {
        session.prompt = self.prompt.clone();
        if !self.preview.is_empty() {
            session.preview = Some(self.preview.clone());
        }
        crate::studio_attach::apply_still_bytes(self.has_tile, self.still_png.clone(), session);
        session.finder_cwd = self.finder_cwd.clone();
        session.finder_files = self.finder_files.clone();
        session.dropped_files = self.dropped_files.clone();
        session.dropped_text = self.dropped_text.clone();
    }

    pub fn ui_frame(&self) -> Frame {
        let status = match self.phase {
            Phase::Bar => "mouse selection · Ask / Copy / Search / Summarize / Explain",
            Phase::Prompt => {
                if self.has_tile {
                    "still attached · remove drops it · Edit opens studio"
                } else {
                    "no still · capture missing or tile removed"
                }
            }
            Phase::Running => "scratch cwd · prompt passed as data",
            Phase::Preview => {
                "result is not in your document until Tab · Super+Return / ⌘Return handoff"
            }
            Phase::Refine => "same attachments · new result replaces the old",
            Phase::Shelf => {
                if self.shelf.as_ref().is_some_and(|s| s.recording) {
                    "clipboard shelf · recording on · contents stay on this machine"
                } else {
                    "clipboard shelf · recording off · existing items stay until cleared"
                }
            }
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
        if !self.dropped_files.is_empty() || !self.dropped_text.is_empty() {
            status.push_str(&format!(
                " · drop {} files + {} text",
                self.dropped_files.len(),
                self.dropped_text.len()
            ));
        }
        let (shelf_query, shelf_lines, shelf_sel) = match &self.shelf {
            Some(s) if self.phase == Phase::Shelf => (s.query.clone(), s.preview_rows(), s.selected),
            _ => (String::new(), Vec::new(), 0),
        };
        Frame {
            phase: self.phase,
            prompt,
            preview: self.preview.clone(),
            has_tile: self.has_tile,
            status,
            shelf_query,
            shelf_lines,
            shelf_sel,
        }
    }

    fn poll_studio(&mut self) -> Vec<OverlayEffect> {
        if let Some(png) = self.studio.take_export() {
            self.still_png = Some(png.clone());
            if let Some(still) = crate::studio_attach::still_from_png(png) {
                self.thumb = still.thumbnail_argb(120, 64).ok();
            }
            self.has_tile = true;
            self.dirty = true;
            return vec![OverlayEffect::Redraw];
        }
        Vec::new()
    }

    pub fn handle_key(&mut self, key: OverlayKey, text: Option<&str>) -> Vec<OverlayEffect> {
        let _ = self.poll_studio();
        if self.phase == Phase::Shelf {
            return self.handle_shelf_key(key, text);
        }
        match key {
            OverlayKey::Escape => {
                self.end = Some(OverlayEnd::Cancelled);
                return Vec::new();
            }
            OverlayKey::Up | OverlayKey::Down => return Vec::new(),
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
        let mut fx = self.poll_studio();
        if self.phase == Phase::Shelf {
            if draw::hit_close(x, y, self.width) {
                self.end = Some(OverlayEnd::Cancelled);
                return Vec::new();
            }
            let n = self.shelf.as_ref().map(|s| s.visible().len()).unwrap_or(0);
            if let Some(i) = draw::hit_shelf_row(x, y, n) {
                if let Some(s) = self.shelf.as_mut() {
                    s.selected = i;
                }
                return self.shelf_paste();
            }
            return fx;
        }
        if self.phase == Phase::Bar {
            if let Some(hit) = draw::hit_bar(x, y, self.width) {
                return self.on_bar(hit);
            }
            return fx;
        }
        if draw::hit_close(x, y, self.width) {
            self.end = Some(OverlayEnd::Cancelled);
            return Vec::new();
        }
        if draw::hit_handoff(x, y, self.phase) {
            return self.run_handoff();
        }
        if draw::hit_edit(x, y, self.has_tile) {
            if let Some(png) = self.still_png.clone() {
                self.studio.edit(&png);
            }
            self.dirty = true;
            fx.push(OverlayEffect::Redraw);
            return fx;
        }
        if draw::hit_remove(x, y, self.has_tile) {
            self.has_tile = false;
            self.thumb = None;
            self.still_png = None;
            self.dirty = true;
            return vec![OverlayEffect::Redraw];
        }
        fx
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

    fn handle_shelf_key(&mut self, key: OverlayKey, text: Option<&str>) -> Vec<OverlayEffect> {
        match key {
            OverlayKey::Escape => {
                self.end = Some(OverlayEnd::Cancelled);
                Vec::new()
            }
            OverlayKey::Return { meta: true } => self.shelf_ask(),
            OverlayKey::Return { meta: false } => self.shelf_paste(),
            OverlayKey::Up => {
                if let Some(s) = self.shelf.as_mut() {
                    s.move_sel(-1);
                }
                self.dirty = true;
                vec![OverlayEffect::Redraw]
            }
            OverlayKey::Down => {
                if let Some(s) = self.shelf.as_mut() {
                    s.move_sel(1);
                }
                self.dirty = true;
                vec![OverlayEffect::Redraw]
            }
            OverlayKey::Backspace => {
                if let Some(s) = self.shelf.as_mut() {
                    s.query.pop();
                    s.clamp_selected();
                }
                self.dirty = true;
                vec![OverlayEffect::Redraw]
            }
            OverlayKey::Text | OverlayKey::R | OverlayKey::Digit(_) => {
                if let Some(txt) = text {
                    if !txt.is_empty() && !txt.chars().any(|c| c.is_control()) {
                        if let Some(s) = self.shelf.as_mut() {
                            s.query.push_str(txt);
                            s.selected = 0;
                        }
                        self.dirty = true;
                        return vec![OverlayEffect::Redraw];
                    }
                }
                Vec::new()
            }
            OverlayKey::Tab => Vec::new(),
        }
    }

    fn shelf_paste(&mut self) -> Vec<OverlayEffect> {
        let Some(clip) = self.shelf.as_ref().and_then(|s| s.selected_clip()).cloned() else {
            return Vec::new();
        };
        match (self.paste_clip)(&clip, &self.focus) {
            Ok(_) => {
                self.end = Some(OverlayEnd::Pasted);
            }
            Err(_) => {
                eprintln!("openatatd: shelf paste failed (contents not logged)");
            }
        }
        Vec::new()
    }

    fn shelf_ask(&mut self) -> Vec<OverlayEffect> {
        let Some(clip) = self.shelf.as_ref().and_then(|s| s.selected_clip()).cloned() else {
            return Vec::new();
        };
        if !clip.plain.is_empty() {
            self.dropped_text.push(clip.plain);
        }
        self.end = Some(OverlayEnd::ShelfAsk);
        Vec::new()
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
        for path in &self.dropped_files {
            out.push(Attachment::File { path: path.clone() });
        }
        for text in &self.dropped_text {
            out.push(Attachment::Snippet { text: text.clone() });
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
            dropped_files: Vec::new(),
            dropped_text: Vec::new(),
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

    #[test]
    fn edit_click_does_not_drop_the_tile() {
        let mut c = OverlayController::from_session(&empty_session(), OverlayKind::Prompt);
        c.has_tile = true;
        c.still_png = Some(b"not-a-png".to_vec());
        c.handle_click(160.0, 214.0);
        assert!(c.has_tile);
        assert!(c.still_png.is_some());
        c.handle_click(160.0, 190.0);
        assert!(!c.has_tile);
    }

    fn dummy_paste(_clip: &Clip, _: &FocusSnapshot) -> crate::error::Result<InsertOutcome> {
        Ok(InsertOutcome::CopiedOnly)
    }

    fn shelf_ctl() -> OverlayController {
        let mut c = OverlayController::from_session(&empty_session(), OverlayKind::Shelf);
        c.paste_clip = dummy_paste;
        c.shelf = Some(crate::shelf::ShelfView {
            query: String::new(),
            selected: 0,
            recording: true,
            items: vec![Clip {
                id: "1".into(),
                timestamp: "0".into(),
                plain: "shelf-item".into(),
                html: None,
                rtf: None,
                image_path: None,
            }],
        });
        c
    }

    #[test]
    fn shelf_return_pastes_and_esc_closes() {
        let mut c = shelf_ctl();
        assert_eq!(c.phase, Phase::Shelf);
        c.handle_key(OverlayKey::Return { meta: false }, None);
        assert_eq!(c.end, Some(OverlayEnd::Pasted));
        let mut c = shelf_ctl();
        c.handle_key(OverlayKey::Escape, None);
        assert_eq!(c.end, Some(OverlayEnd::Cancelled));
    }

    #[test]
    fn shelf_meta_return_hands_clip_as_tile() {
        let mut c = shelf_ctl();
        c.handle_key(OverlayKey::Return { meta: true }, None);
        assert_eq!(c.end, Some(OverlayEnd::ShelfAsk));
        assert_eq!(c.dropped_text, ["shelf-item"]);
    }

    #[test]
    fn shelf_search_filters_without_logging() {
        let mut c = shelf_ctl();
        c.handle_key(OverlayKey::Text, Some("nope"));
        assert!(c.shelf.as_ref().unwrap().visible().is_empty());
        c.handle_key(OverlayKey::Backspace, None);
        c.handle_key(OverlayKey::Backspace, None);
        c.handle_key(OverlayKey::Backspace, None);
        c.handle_key(OverlayKey::Backspace, None);
        assert_eq!(c.shelf.as_ref().unwrap().visible().len(), 1);
    }
}
