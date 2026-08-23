//! Native `zwlr_layer_shell_v1` stop bar. Overlay layer, software `wl_shm`.
//! `KeyboardInteractivity::OnDemand` only while mapped. Esc cancels.
//! Stop writes. Drag empty background to move. Not gpui. Not iced.

use std::os::fd::AsRawFd;
use std::time::Instant;

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

use super::bar::{self, BarHit, BAR_H, BAR_W};
use super::BarEnd;
use crate::error::{Error, Result};

const BTN_LEFT: u32 = 0x110;

pub fn run() -> Result<BarEnd> {
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
        Some("openatat-record"),
        None,
    );
    layer.set_anchor(Anchor::TOP | Anchor::LEFT);
    layer.set_exclusive_zone(0);
    let mut margin_left = 80i32;
    if let Some(out) = crate::focus::output_under_pointer().or_else(crate::focus::focused_output) {
        margin_left = ((out.width as i32) - BAR_W as i32).max(0) / 2;
    }
    layer.set_margin(16, 0, 0, margin_left);
    layer.set_keyboard_interactivity(KeyboardInteractivity::OnDemand);
    layer.set_size(BAR_W, BAR_H);
    layer.commit();

    let pool = SlotPool::new((BAR_W * BAR_H * 4) as usize, &shm)
        .map_err(|e| Error::msg(format!("wl_shm pool: {e}")))?;

    let mut host = BarHost {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        pool,
        layer,
        width: BAR_W,
        height: BAR_H,
        keyboard: None,
        pointer: None,
        started: Instant::now(),
        last_secs: 0,
        drag: None,
        margin_top: 16,
        margin_left,
        end: None,
        dirty: true,
    };

    while host.end.is_none() {
        event_queue
            .flush()
            .map_err(|e| Error::msg(format!("wayland flush: {e}")))?;
        let _ = event_queue.dispatch_pending(&mut host);
        if host.end.is_some() {
            break;
        }
        let secs = host.started.elapsed().as_secs();
        if secs != host.last_secs {
            host.last_secs = secs;
            host.dirty = true;
            host.draw(&qh);
        } else if host.dirty {
            host.draw(&qh);
        }
        if wait_wayland(&conn, 200) {
            if let Some(guard) = event_queue.prepare_read() {
                let _ = guard.read();
            }
            let _ = event_queue.dispatch_pending(&mut host);
        }
    }

    match host.end {
        Some(BarEnd::Stop) => Ok(BarEnd::Stop),
        _ => Ok(BarEnd::Cancel),
    }
}

fn wait_wayland(conn: &Connection, timeout_ms: i32) -> bool {
    let fd = conn.backend().poll_fd().as_raw_fd();
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe { libc::poll(&mut pfd, 1, timeout_ms) > 0 }
}

struct BarHost {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    layer: LayerSurface,
    width: u32,
    height: u32,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    started: Instant,
    last_secs: u64,
    drag: Option<(f64, f64, i32, i32)>,
    margin_top: i32,
    margin_left: i32,
    end: Option<BarEnd>,
    dirty: bool,
}

impl BarHost {
    fn apply_margin(&mut self) {
        self.layer
            .set_margin(self.margin_top, 0, 0, self.margin_left);
        self.layer.commit();
    }

    fn draw(&mut self, qh: &QueueHandle<Self>) {
        let width = self.width.max(1);
        let height = self.height.max(1);
        let stride = width as i32 * 4;
        let pixels = bar::render(width, height, self.started.elapsed().as_secs());
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
}

impl CompositorHandler for BarHost {
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

impl OutputHandler for BarHost {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for BarHost {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.end = Some(BarEnd::Cancel);
    }
    fn configure(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        if configure.new_size.0 > 0 {
            self.width = configure.new_size.0;
        }
        if configure.new_size.1 > 0 {
            self.height = configure.new_size.1;
        }
        self.draw(qh);
    }
}

impl SeatHandler for BarHost {
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

impl KeyboardHandler for BarHost {
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
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        if event.keysym == Keysym::Escape {
            self.end = Some(BarEnd::Cancel);
        }
    }
    fn repeat_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: KeyEvent,
    ) {
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

impl PointerHandler for BarHost {
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
            match event.kind {
                PointerEventKind::Press { button, .. } if button == BTN_LEFT => {
                    match bar::hit(event.position.0, event.position.1) {
                        BarHit::Stop => self.end = Some(BarEnd::Stop),
                        BarHit::Drag => {
                            self.drag = Some((
                                event.position.0,
                                event.position.1,
                                self.margin_left,
                                self.margin_top,
                            ));
                        }
                    }
                }
                PointerEventKind::Motion { .. } => {
                    if let Some((px, py, left, top)) = self.drag {
                        let dx = event.position.0 - px;
                        let dy = event.position.1 - py;
                        self.margin_left = (left + dx.round() as i32).max(0);
                        self.margin_top = (top + dy.round() as i32).max(0);
                        self.apply_margin();
                    }
                }
                PointerEventKind::Release { button, .. } if button == BTN_LEFT => {
                    self.drag = None;
                    self.dirty = true;
                    self.draw(qh);
                }
                _ => {}
            }
        }
    }
}

impl ShmHandler for BarHost {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_compositor!(BarHost);
delegate_output!(BarHost);
delegate_shm!(BarHost);
delegate_seat!(BarHost);
delegate_keyboard!(BarHost);
delegate_pointer!(BarHost);
delegate_layer!(BarHost);
delegate_registry!(BarHost);

impl ProvidesRegistryState for BarHost {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}
