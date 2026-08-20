//! Native `zwlr_layer_shell_v1` popover. Software `wl_shm`. No GPU.
//!
//! Keyboard is `OnDemand` only while this surface is mapped. Idle `openatatd`
//! destroys the surface. This is not an xdg-toplevel and not gpui.

use std::convert::TryInto;
use std::num::NonZeroU32;

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers};
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

use super::draw::{self, Frame, Phase, POPOVER_H, POPOVER_W};
use super::OverlayEnd;
use crate::agent;
use crate::error::{Error, Result};
use crate::history;
use crate::session::Session;
use openatat_ipc::EntryPoint;

pub fn run(session: &mut Session) -> Result<OverlayEnd> {
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
    let layer = layer_shell.create_layer_surface(
        &qh,
        surface,
        Layer::Overlay,
        Some("openatat"),
        None,
    );
    // Small popover, not a reserved bar, not Exclusive (that steals the seat).
    layer.set_anchor(Anchor::TOP);
    layer.set_margin(80, 0, 0, 0);
    layer.set_exclusive_zone(0);
    layer.set_keyboard_interactivity(KeyboardInteractivity::OnDemand);
    layer.set_size(POPOVER_W, POPOVER_H);
    layer.commit();

    let pool = SlotPool::new((POPOVER_W * POPOVER_H * 4) as usize, &shm)
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
        width: POPOVER_W,
        height: POPOVER_H,
        first_configure: true,
        keyboard: None,
        pointer: None,
        prompt: String::new(),
        preview: String::new(),
        phase: Phase::Prompt,
        has_tile: session.still.is_some(),
        thumb,
        end: None,
        dirty: true,
        entry: session.entry,
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
    if !overlay.has_tile {
        session.still = None;
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
    phase: Phase,
    has_tile: bool,
    thumb: Option<(u32, u32, Vec<u8>)>,
    end: Option<OverlayEnd>,
    dirty: bool,
    entry: EntryPoint,
}

impl Overlay {
    fn ui_frame(&self) -> Frame {
        let status = match self.phase {
            Phase::Prompt => {
                if self.has_tile {
                    "C1 still attached · click remove to drop it"
                } else {
                    "no still · grim missing or tile removed"
                }
            }
            Phase::Running => "dummy CLI (echo / OPENATAT_AGENT)",
            Phase::Preview => "result is not in your document until Tab",
        };
        Frame {
            phase: self.phase,
            prompt: self.prompt.clone(),
            preview: self.preview.clone(),
            has_tile: self.has_tile,
            status: status.into(),
        }
    }

    fn draw(&mut self, qh: &QueueHandle<Self>) {
        let width = self.width.max(1);
        let height = self.height.max(1);
        let stride = width as i32 * 4;
        let thumb = self.thumb.as_ref().map(|(w, h, p)| (*w, *h, p.as_slice()));
        let pixels = draw::render(width, height, &self.ui_frame(), thumb);

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
        match event.keysym {
            Keysym::Escape => {
                self.end = Some(OverlayEnd::Cancelled);
                return;
            }
            Keysym::Return | Keysym::KP_Enter => {
                if self.phase == Phase::Prompt {
                    self.run_agent(qh);
                }
                return;
            }
            Keysym::Tab => {
                if self.phase == Phase::Preview {
                    self.end = Some(OverlayEnd::Tab);
                }
                return;
            }
            Keysym::BackSpace => {
                if self.phase == Phase::Prompt {
                    self.prompt.pop();
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
        }
    }

    fn run_agent(&mut self, qh: &QueueHandle<Self>) {
        self.phase = Phase::Running;
        self.draw(qh);
        let prompt = if self.prompt.is_empty() {
            "hello from openatat".to_string()
        } else {
            self.prompt.clone()
        };
        self.prompt = prompt.clone();
        let _ = history::append_prompt(self.entry, &prompt);
        match agent::run_dummy(&prompt) {
            Ok(out) => self.preview = out,
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

    fn frame(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: u32,
    ) {
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
        self.width = NonZeroU32::new(configure.new_size.0).map_or(POPOVER_W, NonZeroU32::get);
        self.height = NonZeroU32::new(configure.new_size.1).map_or(POPOVER_H, NonZeroU32::get);
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
        _: Modifiers,
        _: RawModifiers,
        _: u32,
    ) {
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
                if draw::hit_close(x, y, self.width) {
                    self.end = Some(OverlayEnd::Cancelled);
                } else if draw::hit_remove(x, y, self.has_tile) {
                    self.has_tile = false;
                    self.thumb = None;
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
