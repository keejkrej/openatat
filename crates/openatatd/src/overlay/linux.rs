//! Native `zwlr_layer_shell_v1` popover. Software `wl_shm`. No GPU.
//!
//! Keyboard is `OnDemand` only while this surface is mapped. Idle `openatatd`
//! destroys the surface. This is not an xdg-toplevel and not gpui.

use std::convert::TryInto;
use std::num::NonZeroU32;

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::keyboard::{
    KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers,
};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind, PointerHandler};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat, delegate_shm, registry_handlers,
};
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface};
use wayland_client::{Connection, QueueHandle};

use super::controller::{OverlayController, OverlayEffect, OverlayKey};
use super::draw::{self, Frame, Phase, BAR_H, BAR_W, POPOVER_H, POPOVER_W, SHELF_H, SHELF_W};
use super::{OverlayEnd, OverlayKind};
use crate::a11y::TextSelection;
use crate::agent::{self, Attachment, Launch, RefineSession};
use crate::error::{Error, Result};
use crate::handoff::{self, Handoff};
use crate::history;
use crate::selection::{self, PromptAction};
use crate::session::Session;
use crate::studio_attach::StudioAttach;
use openatat_ipc::EntryPoint;

pub fn run(session: &mut Session, kind: OverlayKind) -> Result<OverlayEnd> {
    let conn = Connection::connect_to_env().map_err(|e| Error::msg(format!("wayland: {e}")))?;
    let (globals, mut event_queue) =
        registry_queue_init(&conn).map_err(|e| Error::msg(format!("wayland registry: {e}")))?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)
        .map_err(|_| Error::msg("wl_compositor is not available"))?;
    let layer_shell = LayerShell::bind(&globals, &qh)
        .map_err(|_| Error::msg("zwlr_layer_shell_v1 is not available"))?;
    let shm = Shm::bind(&globals, &qh).map_err(|_| Error::msg("wl_shm is not available"))?;

    let surface = compositor.create_surface(&qh);
    let layer =
        layer_shell.create_layer_surface(&qh, surface, Layer::Overlay, Some("openatat"), None);
    // Small popover / compact bar, not a reserved Exclusive zone (that steals the seat).
    let (init_w, init_h, top, left) = match kind {
        OverlayKind::SelectionBar => {
            let p = session.placement.clone().unwrap_or(selection::Placement {
                margin_top: 80,
                margin_left: 0,
            });
            layer.set_anchor(Anchor::TOP | Anchor::LEFT);
            (BAR_W, BAR_H, p.margin_top, p.margin_left)
        }
        OverlayKind::Prompt => {
            layer.set_anchor(Anchor::TOP);
            (POPOVER_W, POPOVER_H, 80, 0)
        }
        OverlayKind::Shelf => {
            // Slides up from the bottom. Same nonactivating layer-shell class.
            layer.set_anchor(Anchor::BOTTOM);
            (SHELF_W, SHELF_H, 0, 0)
        }
    };
    let (margin_top, margin_right, margin_bottom, margin_left) = match kind {
        OverlayKind::Shelf => (0, 0, 24, 0),
        OverlayKind::SelectionBar => (top, 0, 0, left),
        OverlayKind::Prompt => (top, 0, 0, left),
    };
    layer.set_margin(margin_top, margin_right, margin_bottom, margin_left);
    layer.set_exclusive_zone(0);
    layer.set_keyboard_interactivity(KeyboardInteractivity::OnDemand);
    layer.set_size(init_w, init_h);
    layer.commit();

    let pool = SlotPool::new((SHELF_W * SHELF_H * 4) as usize, &shm)
        .map_err(|e| Error::msg(format!("wl_shm pool: {e}")))?;

    let thumb = session
        .still
        .as_ref()
        .and_then(|s| s.thumbnail_argb(120, 64).ok());

    let mut overlay = Overlay {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        pool,
        layer,
        width: init_w,
        height: init_h,
        first_configure: true,
        keyboard: None,
        pointer: None,
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
        selection: session.selection.clone(),
        prompt_action: None,
        has_tile: session.still.is_some(),
        thumb,
        end: None,
        dirty: true,
        entry: session.entry,
        mods: Modifiers::default(),
        dropped_files: session.dropped_files.clone(),
        dropped_text: session.dropped_text.clone(),
        finder_cwd: session.finder_cwd.clone(),
        finder_files: session.finder_files.clone(),
        studio: StudioAttach::default(),
        ctl: matches!(kind, OverlayKind::Shelf)
            .then(|| OverlayController::from_session(session, kind)),
    };

    while overlay.end.is_none() {
        event_queue
            .blocking_dispatch(&mut overlay)
            .map_err(|e| Error::msg(format!("wayland dispatch: {e}")))?;
    }

    session.prompt = overlay.prompt;
    if !overlay.preview.is_empty() {
        session.preview = Some(overlay.preview);
    }
    crate::studio_attach::apply_still_bytes(overlay.has_tile, overlay.still_png, session);
    session.dropped_files = overlay.dropped_files;
    session.dropped_text = overlay.dropped_text;
    if let Some(ctl) = overlay.ctl {
        ctl.write_back(session);
        return Ok(ctl.end.unwrap_or(OverlayEnd::Cancelled));
    }
    Ok(overlay.end.unwrap_or(OverlayEnd::Cancelled))
}

struct Overlay {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    layer: LayerSurface,
    width: u32,
    height: u32,
    first_configure: bool,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    prompt: String,
    preview: String,
    refine: String,
    refine_session: Option<RefineSession>,
    template_display: String,
    still_png: Option<Vec<u8>>,
    phase: Phase,
    has_tile: bool,
    thumb: Option<(u32, u32, Vec<u8>)>,
    end: Option<OverlayEnd>,
    dirty: bool,
    entry: EntryPoint,
    selection: Option<TextSelection>,
    prompt_action: Option<PromptAction>,
    mods: Modifiers,
    dropped_files: Vec<std::path::PathBuf>,
    dropped_text: Vec<String>,
    finder_cwd: Option<std::path::PathBuf>,
    finder_files: Vec<std::path::PathBuf>,
    studio: StudioAttach,
    ctl: Option<OverlayController>,
}

impl Overlay {
    fn poll_studio(&mut self, qh: &QueueHandle<Self>) {
        if let Some(png) = self.studio.take_export() {
            self.still_png = Some(png.clone());
            if let Some(still) = crate::studio_attach::still_from_png(png) {
                self.thumb = still.thumbnail_argb(120, 64).ok();
            }
            self.has_tile = true;
            self.dirty = true;
            self.draw(qh);
        }
    }

    fn ui_frame(&self) -> Frame {
        let status = match self.phase {
            Phase::Bar => "mouse selection · Ask / Copy / Search / Summarize / Explain",
            Phase::Prompt => {
                if self.has_tile {
                    "still attached · remove drops it · Edit opens studio"
                } else {
                    "no still · grim missing or tile removed"
                }
            }
            Phase::Running => "scratch cwd · prompt passed as data",
            Phase::Preview => "result is not in your document until Tab · Super+Return handoff",
            Phase::Refine => "same attachments · new result replaces the old",
            Phase::Shelf => "clipboard shelf · Return pastes · Super+Return tile",
        };
        let prompt = if self.phase == Phase::Refine {
            self.refine.clone()
        } else {
            self.prompt.clone()
        };
        let status = if self.phase == Phase::Running || self.phase == Phase::Prompt {
            format!("{} · {}", self.template_display, status)
        } else {
            status.to_string()
        };
        Frame {
            phase: self.phase,
            prompt,
            preview: self.preview.clone(),
            has_tile: self.has_tile,
            status,
            shelf_query: String::new(),
            shelf_lines: Vec::new(),
            shelf_sel: 0,
        }
    }

    fn apply_ctl(&mut self, fx: Vec<OverlayEffect>, qh: &QueueHandle<Self>) {
        let Some(ctl) = self.ctl.as_mut() else {
            return;
        };
        for e in fx {
            match e {
                OverlayEffect::Redraw => self.dirty = true,
                OverlayEffect::Resize { w, h } => {
                    self.width = w;
                    self.height = h;
                    ctl.width = w;
                    ctl.height = h;
                    self.layer.set_size(w, h);
                    self.layer.commit();
                }
            }
        }
        if ctl.end.is_some() {
            self.end = ctl.end;
        }
        if ctl.dirty || self.dirty {
            ctl.dirty = false;
            self.dirty = true;
            self.draw(qh);
        }
    }

    fn map_key(&self, event: &KeyEvent) -> OverlayKey {
        match event.keysym {
            Keysym::Escape => OverlayKey::Escape,
            Keysym::Return | Keysym::KP_Enter => OverlayKey::Return {
                meta: self.mods.logo,
            },
            Keysym::Tab => OverlayKey::Tab,
            Keysym::BackSpace => OverlayKey::Backspace,
            Keysym::Up | Keysym::KP_Up => OverlayKey::Up,
            Keysym::Down | Keysym::KP_Down => OverlayKey::Down,
            Keysym::r | Keysym::R => OverlayKey::R,
            Keysym::_1 => OverlayKey::Digit(1),
            Keysym::_2 => OverlayKey::Digit(2),
            Keysym::_3 => OverlayKey::Digit(3),
            Keysym::_4 => OverlayKey::Digit(4),
            Keysym::_5 => OverlayKey::Digit(5),
            _ => OverlayKey::Text,
        }
    }

    fn draw(&mut self, qh: &QueueHandle<Self>) {
        let width = self.width.max(1);
        let height = self.height.max(1);
        let stride = width as i32 * 4;
        let thumb = self.thumb.as_ref().map(|(w, h, p)| (*w, *h, p.as_slice()));
        let frame = if let Some(ctl) = self.ctl.as_ref() {
            ctl.ui_frame()
        } else {
            self.ui_frame()
        };
        let pixels = draw::render(width, height, &frame, thumb);

        let (buffer, canvas) = match self.pool.create_buffer(
            width as i32,
            height as i32,
            stride,
            wl_shm::Format::Argb8888,
        ) {
            Ok(v) => v,
            Err(_) => return,
        };
        let n = canvas.len().min(pixels.len());
        canvas[..n].copy_from_slice(&pixels[..n]);

        self.layer
            .wl_surface()
            .damage_buffer(0, 0, width as i32, height as i32);
        self.layer
            .wl_surface()
            .frame(qh, self.layer.wl_surface().clone());
        let _ = buffer.attach_to(self.layer.wl_surface());
        self.layer.commit();
        self.dirty = false;
    }

    fn handle_key(&mut self, event: KeyEvent, qh: &QueueHandle<Self>) {
        self.poll_studio(qh);
        if self.ctl.is_some() {
            let key = self.map_key(&event);
            let text = event.utf8.clone();
            let fx = self
                .ctl
                .as_mut()
                .unwrap()
                .handle_key(key, text.as_deref());
            self.apply_ctl(fx, qh);
            return;
        }
        match event.keysym {
            Keysym::Escape => {
                self.end = Some(OverlayEnd::Cancelled);
                return;
            }
            Keysym::_1 if self.phase == Phase::Bar => {
                self.on_bar(draw::BarHit::Ask, qh);
                return;
            }
            Keysym::_2 if self.phase == Phase::Bar => {
                self.on_bar(draw::BarHit::Copy, qh);
                return;
            }
            Keysym::_3 if self.phase == Phase::Bar => {
                self.on_bar(draw::BarHit::Search, qh);
                return;
            }
            Keysym::_4 if self.phase == Phase::Bar => {
                self.on_bar(draw::BarHit::Summarize, qh);
                return;
            }
            Keysym::_5 if self.phase == Phase::Bar => {
                self.on_bar(draw::BarHit::Explain, qh);
                return;
            }
            Keysym::Return | Keysym::KP_Enter => {
                if self.mods.logo {
                    // Super+Return / ⌘Return. Does not change OnDemand focus.
                    if self.phase != Phase::Bar && self.phase != Phase::Running {
                        self.run_handoff(qh);
                    }
                    return;
                }
                if self.phase == Phase::Prompt {
                    self.run_agent(qh, false);
                } else if self.phase == Phase::Refine {
                    self.run_agent(qh, true);
                }
                return;
            }
            Keysym::Tab => {
                if self.phase == Phase::Preview {
                    self.end = Some(OverlayEnd::Tab);
                }
                return;
            }
            Keysym::r | Keysym::R => {
                if self.phase == Phase::Preview {
                    self.refine.clear();
                    self.phase = Phase::Refine;
                    self.dirty = true;
                    self.draw(qh);
                    return;
                }
            }
            Keysym::BackSpace => {
                if self.phase == Phase::Prompt {
                    self.prompt.pop();
                    self.dirty = true;
                    self.draw(qh);
                } else if self.phase == Phase::Refine {
                    self.refine.pop();
                    self.dirty = true;
                    self.draw(qh);
                }
                return;
            }
            _ => {}
        }
        if self.phase == Phase::Prompt {
            if let Some(txt) = event.utf8 {
                if !txt.is_empty() && !txt.chars().any(|c| c.is_control()) {
                    self.prompt.push_str(&txt);
                    self.dirty = true;
                    self.draw(qh);
                }
            }
        } else if self.phase == Phase::Refine {
            if let Some(txt) = event.utf8 {
                if !txt.is_empty() && !txt.chars().any(|c| c.is_control()) {
                    self.refine.push_str(&txt);
                    self.dirty = true;
                    self.draw(qh);
                }
            }
        }
    }

    fn on_bar(&mut self, hit: draw::BarHit, qh: &QueueHandle<Self>) {
        match hit {
            draw::BarHit::Close => {
                self.end = Some(OverlayEnd::Cancelled);
            }
            draw::BarHit::Copy => {
                if let Some(sel) = self.held_selection() {
                    if selection::copy_selection(sel).is_ok() {
                        self.end = Some(OverlayEnd::Copied);
                    }
                }
            }
            draw::BarHit::Search => {
                if let Some(sel) = self.held_selection() {
                    if selection::search_selection(sel).is_ok() {
                        self.end = Some(OverlayEnd::Copied);
                    }
                }
            }
            draw::BarHit::Ask => {
                self.prompt_action = Some(PromptAction::Ask);
                self.grow_to_popover();
                if self.still_png.is_none() {
                    if let Ok(still) = crate::capture::capture_active_output(None) {
                        self.has_tile = true;
                        self.thumb = still.thumbnail_argb(120, 64).ok();
                        self.still_png = Some(still.png);
                    }
                }
                self.phase = Phase::Prompt;
                self.dirty = true;
                self.draw(qh);
            }
            draw::BarHit::Summarize => {
                self.prompt_action = Some(PromptAction::Summarize);
                self.prompt = "Summarize".into();
                self.grow_to_popover();
                self.run_agent(qh, false);
            }
            draw::BarHit::Explain => {
                self.prompt_action = Some(PromptAction::Explain);
                self.prompt = "Explain".into();
                self.grow_to_popover();
                self.run_agent(qh, false);
            }
        }
    }

    fn grow_to_popover(&mut self) {
        self.width = POPOVER_W;
        self.height = POPOVER_H;
        self.layer.set_size(POPOVER_W, POPOVER_H);
        self.layer.commit();
    }

    /// Re-probe secure role before using the held selection. Never log the text.
    fn held_selection(&self) -> Option<&TextSelection> {
        if !selection::allow_held_selection() {
            return None;
        }
        self.selection.as_ref()
    }

    fn run_agent(&mut self, qh: &QueueHandle<Self>, is_refine: bool) {
        if is_refine && self.refine.is_empty() {
            return;
        }
        self.phase = Phase::Running;
        self.draw(qh);
        if self.prompt.is_empty() {
            self.prompt = "hello from openatat".to_string();
        }
        if self.refine_session.is_none() {
            self.refine_session = Some(RefineSession::new(self.prompt.clone()));
        }
        let Some(sess) = self.refine_session.as_mut() else {
            return;
        };
        if is_refine {
            let sentence = if self.refine.is_empty() {
                return;
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
            // Selection is appended for the CLI only. Never written to history.
            launch_prompt = format!("{launch_prompt}\n\nSelected text:\n{}", sel.text());
        }
        let _ = history::append_prompt(self.entry, &history_prompt);
        let attachments = self.attachments();
        self.launch_agent(qh, &launch_prompt, &attachments);
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

    fn run_handoff(&mut self, qh: &QueueHandle<Self>) {
        if self.phase == Phase::Refine && self.refine.is_empty() && self.refine_session.is_none() {
            return;
        }
        if self.prompt.is_empty() {
            self.prompt = "hello from openatat".to_string();
        }
        if self.refine_session.is_none() {
            self.refine_session = Some(RefineSession::new(self.prompt.clone()));
        }
        let Some(sess) = self.refine_session.as_mut() else {
            return;
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
            }
            Err(e) => {
                self.preview = format!("handoff error: {e}");
                self.phase = Phase::Preview;
                self.grow_to_popover();
                self.dirty = true;
                self.draw(qh);
            }
        }
    }

    fn launch_agent(
        &mut self,
        qh: &QueueHandle<Self>,
        launch_prompt: &str,
        attachments: &[Attachment],
    ) {
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
        self.draw(qh);
    }
}

impl CompositorHandler for Overlay {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(&mut self, _: &Connection, qh: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {
        if self.dirty {
            self.draw(qh);
        }
    }

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for Overlay {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for Overlay {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.end = Some(OverlayEnd::Cancelled);
    }

    fn configure(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let fallback_w = match self.phase {
            Phase::Bar => BAR_W,
            Phase::Shelf => SHELF_W,
            _ => POPOVER_W,
        };
        let fallback_h = match self.phase {
            Phase::Bar => BAR_H,
            Phase::Shelf => SHELF_H,
            _ => POPOVER_H,
        };
        self.width = NonZeroU32::new(configure.new_size.0).map_or(fallback_w, NonZeroU32::get);
        self.height = NonZeroU32::new(configure.new_size.1).map_or(fallback_h, NonZeroU32::get);
        self.draw(qh);
        self.first_configure = false;
    }
}

impl SeatHandler for Overlay {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            if let Ok(k) = self.seat_state.get_keyboard(qh, &seat, None) {
                self.keyboard = Some(k);
            }
        }
        if capability == Capability::Pointer && self.pointer.is_none() {
            if let Ok(p) = self.seat_state.get_pointer(qh, &seat) {
                self.pointer = Some(p);
            }
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard {
            if let Some(k) = self.keyboard.take() {
                k.release();
            }
        }
        if capability == Capability::Pointer {
            if let Some(p) = self.pointer.take() {
                p.release();
            }
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl KeyboardHandler for Overlay {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
    }

    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
    ) {
    }

    fn press_key(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        self.handle_key(event, qh);
    }

    fn repeat_key(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        self.handle_key(event, qh);
    }

    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: KeyEvent,
    ) {
    }

    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        modifiers: Modifiers,
        _: RawModifiers,
        _: u32,
    ) {
        self.mods = modifiers;
    }
}

impl PointerHandler for Overlay {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            if &event.surface != self.layer.wl_surface() {
                continue;
            }
            if let PointerEventKind::Press { button, .. } = event.kind {
                // linux/input-event-codes.h BTN_LEFT
                if button != 0x110 {
                    continue;
                }
                let (x, y) = event.position;
                if self.ctl.is_some() {
                    let fx = self.ctl.as_mut().unwrap().handle_click(x, y);
                    self.apply_ctl(fx, qh);
                    continue;
                }
                if self.phase == Phase::Bar {
                    if let Some(hit) = draw::hit_bar(x, y, self.width) {
                        self.on_bar(hit, qh);
                    }
                    continue;
                }
                self.poll_studio(qh);
                if draw::hit_close(x, y, self.width) {
                    self.end = Some(OverlayEnd::Cancelled);
                } else if draw::hit_handoff(x, y, self.phase) {
                    self.run_handoff(qh);
                } else if draw::hit_edit(x, y, self.has_tile) {
                    if let Some(png) = self.still_png.clone() {
                        self.studio.edit(&png);
                    }
                    self.dirty = true;
                    self.draw(qh);
                } else if draw::hit_remove(x, y, self.has_tile) {
                    self.has_tile = false;
                    self.thumb = None;
                    self.still_png = None;
                    self.dirty = true;
                    self.draw(qh);
                }
            }
        }
    }
}

impl ShmHandler for Overlay {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_compositor!(Overlay);
delegate_output!(Overlay);
delegate_shm!(Overlay);
delegate_seat!(Overlay);
delegate_keyboard!(Overlay);
delegate_pointer!(Overlay);
delegate_layer!(Overlay);
delegate_registry!(Overlay);

impl ProvidesRegistryState for Overlay {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

// silence unused TryInto on some sctk versions
#[allow(dead_code)]
fn _touch_tryinto(v: u32) -> [u8; 4] {
    v.to_le_bytes().try_into().unwrap()
}
