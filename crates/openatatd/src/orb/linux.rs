//! `zwlr_layer_shell_v1` Orb. Software `wl_shm`. Overlay layer, no exclusive
//! zone, input region = the circle. `KeyboardInteractivity::None` at idle.
//! Not a Quickshell item.

use std::io::Read;
use std::os::fd::{AsFd, AsRawFd};
use std::time::{Duration, Instant};

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
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
    delegate_compositor, delegate_layer, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_shm, registry_handlers,
};
use wayland_client::globals::{registry_queue_init, GlobalList};
use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
use wayland_client::protocol::wl_data_device_manager::{self, WlDataDeviceManager};
use wayland_client::protocol::wl_data_offer::{self, WlDataOffer};
use wayland_client::protocol::wl_region::WlRegion;
use wayland_client::protocol::{wl_output, wl_pointer, wl_seat, wl_shm, wl_surface};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};

use super::draw::{self, OrbFace, OrbFrame, ORB_ERROR_H, ORB_ERROR_W, ORB_IDLE};
use super::policy::{self, OrbDrop};
use super::position::{self, OrbPos, OrbPositions};
use super::OrbCommand;
use crate::error::{Error, Result};

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const DRAG_PX: f64 = 6.0;

pub fn start() {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        eprintln!("openatatd: Orb skipped (no WAYLAND_DISPLAY)");
        return;
    }
    std::thread::Builder::new()
        .name("openatat-orb".into())
        .spawn(|| {
            if let Err(e) = run_host() {
                eprintln!("openatatd: Orb: {e}");
            }
        })
        .ok();
}

fn run_host() -> Result<()> {
    let conn = Connection::connect_to_env().map_err(|e| Error::msg(format!("wayland: {e}")))?;
    let (globals, mut event_queue) =
        registry_queue_init(&conn).map_err(|e| Error::msg(format!("wayland registry: {e}")))?;
    let qh = event_queue.handle();
    ORB_QH.with(|slot| *slot.borrow_mut() = Some(qh.clone()));

    let compositor = CompositorState::bind(&globals, &qh)
        .map_err(|_| Error::msg("wl_compositor is not available"))?;
    let layer_shell = LayerShell::bind(&globals, &qh)
        .map_err(|_| Error::msg("zwlr_layer_shell_v1 is not available"))?;
    let shm = Shm::bind(&globals, &qh).map_err(|_| Error::msg("wl_shm is not available"))?;
    let ddm = globals.bind(&qh, 1..=3, ()).ok();

    let pool = SlotPool::new((ORB_ERROR_W * ORB_ERROR_H * 4) as usize, &shm)
        .map_err(|e| Error::msg(format!("wl_shm pool: {e}")))?;

    let store = OrbPositions::load();
    let (x, y) = initial_pos(&store);

    let mut orb = Orb {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        compositor,
        shm,
        pool,
        layer_shell,
        layer: None,
        ddm,
        data_device: None,
        offer: None,
        offer_mimes: Vec::new(),
        drop_serial: 0,
        width: ORB_IDLE,
        height: ORB_IDLE,
        pointer: None,
        pos: OrbPos { x, y },
        store,
        output_name: crate::focus::active_output().unwrap_or_else(|| "default".into()),
        face: OrbFace::Idle,
        look_x: ORB_IDLE as f32 / 2.0,
        look_y: ORB_IDLE as f32 / 2.0,
        press: None,
        dragging: false,
        error: String::new(),
        started: Instant::now(),
        mapped: false,
        hidden: !super::is_shown(),
    };

    if !orb.hidden {
        orb.map(&qh);
    }
    eprintln!(
        "openatatd: idle — Orb mapped, overlay unmapped, KeyboardInteractivity::None, no GPU"
    );

    loop {
        match super::take_command() {
            OrbCommand::Hide => {
                orb.hidden = true;
                orb.unmap();
            }
            OrbCommand::Show => {
                orb.hidden = false;
                if !orb.mapped {
                    orb.map(&qh);
                }
            }
            OrbCommand::DismissError => {
                orb.error.clear();
                orb.sync_face(&qh);
            }
            OrbCommand::None => {}
        }
        if !super::is_shown() && orb.mapped {
            orb.hidden = true;
            orb.unmap();
        }

        orb.tick(&qh);
        let _ = event_queue.flush();
        let _ = event_queue.dispatch_pending(&mut orb);
        if wait_wayland(&conn, 20) {
            if let Some(guard) = event_queue.prepare_read() {
                let _ = guard.read();
            }
            let _ = event_queue.dispatch_pending(&mut orb);
        }
    }
}

fn initial_pos(store: &OrbPositions) -> (i32, i32) {
    if let Some(out) = crate::focus::focused_output() {
        if let Some(p) = store.get(&out.name) {
            return (p.x, p.y);
        }
        let p = position::default_pos(out.width as i32, out.height as i32, ORB_IDLE);
        return (p.x, p.y);
    }
    (80, 80)
}

fn wait_wayland(conn: &Connection, ms: i32) -> bool {
    let fd = conn.as_fd().as_raw_fd();
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe { libc::poll(&mut pfd, 1, ms) > 0 }
}

struct Orb {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    compositor: CompositorState,
    shm: Shm,
    pool: SlotPool,
    layer_shell: LayerShell,
    layer: Option<LayerSurface>,
    ddm: Option<WlDataDeviceManager>,
    data_device: Option<WlDataDevice>,
    offer: Option<WlDataOffer>,
    offer_mimes: Vec<String>,
    drop_serial: u32,
    width: u32,
    height: u32,
    pointer: Option<wl_pointer::WlPointer>,
    pos: OrbPos,
    store: OrbPositions,
    output_name: String,
    face: OrbFace,
    look_x: f32,
    look_y: f32,
    press: Option<(f64, f64, i32, i32)>,
    dragging: bool,
    error: String,
    started: Instant,
    mapped: bool,
    hidden: bool,
}

impl Orb {
    fn map(&mut self, qh: &QueueHandle<Self>) {
        if self.mapped || self.hidden {
            return;
        }
        let surface = self.compositor.create_surface(qh);
        let layer = self.layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Overlay,
            Some("openatat-orb"),
            None,
        );
        layer.set_anchor(Anchor::TOP | Anchor::LEFT);
        layer.set_margin(self.pos.y, 0, 0, self.pos.x);
        layer.set_exclusive_zone(0);
        // Idle Orb: never Exclusive, never OnDemand.
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_size(self.width, self.height);
        layer.commit();
        self.layer = Some(layer);
        self.mapped = true;
        self.apply_input_region();
        self.draw(qh);
    }

    fn unmap(&mut self) {
        if let Some(layer) = self.layer.take() {
            layer.wl_surface().attach(None, 0, 0);
            layer.wl_surface().commit();
            drop(layer);
        }
        self.mapped = false;
    }

    fn apply_input_region(&self) {
        let Some(layer) = self.layer.as_ref() else {
            return;
        };
        let compositor = self.compositor.wl_compositor();
        // create_region needs a QueueHandle; SCTK surfaces accept a dummy via
        // the existing layer queue. Use the compositor object directly.
        let region = compositor.create_region(&wayland_dummy_qh(), ());
        if self.face == OrbFace::Error {
            region.add(0, 0, self.width as i32, self.height as i32);
        } else {
            for (x, y, w, h) in draw::circle_rects(self.width.min(self.height)) {
                region.add(x, y, w, h);
            }
        }
        layer.wl_surface().set_input_region(Some(&region));
        region.destroy();
        layer.commit();
    }

    fn tick(&mut self, qh: &QueueHandle<Self>) {
        if let Some((sx, sy)) = crate::focus::cursor_pos() {
            if let Some(out) = crate::focus::focused_output() {
                self.look_x = (sx - out.x - self.pos.x) as f32;
                self.look_y = (sy - out.y - self.pos.y) as f32;
            } else {
                self.look_x = (sx - self.pos.x) as f32;
                self.look_y = (sy - self.pos.y) as f32;
            }
        }
        let busy = crate::daemon::is_session_busy();
        if let Some(msg) = crate::daemon::last_error_message() {
            self.error = msg;
        } else if !busy {
            self.error.clear();
        }
        let next = draw::face_from_presence(busy, (!self.error.is_empty()).then_some(self.error.as_str()));
        if next != self.face {
            self.face = next;
            let (w, h) = draw::size_for(self.face);
            self.width = w;
            self.height = h;
            if let Some(layer) = self.layer.as_ref() {
                layer.set_size(w, h);
                layer.commit();
            }
            self.apply_input_region();
        }
        if self.mapped {
            self.draw(qh);
        }
    }

    fn sync_face(&mut self, qh: &QueueHandle<Self>) {
        self.face = draw::face_from_presence(false, None);
        let (w, h) = draw::size_for(self.face);
        self.width = w;
        self.height = h;
        if let Some(layer) = self.layer.as_ref() {
            layer.set_size(w, h);
            layer.commit();
        }
        self.apply_input_region();
        self.draw(qh);
    }

    fn draw(&mut self, qh: &QueueHandle<Self>) {
        let Some(layer) = self.layer.as_ref() else {
            return;
        };
        let width = self.width.max(1);
        let height = self.height.max(1);
        let pulse = self.started.elapsed().as_secs_f32() * 6.0;
        let frame = OrbFrame {
            face: self.face,
            look_x: self.look_x,
            look_y: self.look_y,
            error: self.error.clone(),
            pulse,
        };
        let pixels = draw::render(width, height, &frame);
        let stride = width as i32 * 4;
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
        layer
            .wl_surface()
            .damage_buffer(0, 0, width as i32, height as i32);
        layer.wl_surface().frame(qh, layer.wl_surface().clone());
        let _ = buffer.attach_to(layer.wl_surface());
        layer.commit();
    }

    fn persist_pos(&mut self) {
        self.store.set(self.output_name.clone(), self.pos);
        self.store.save();
    }

    fn move_to(&mut self, x: i32, y: i32) {
        self.pos = OrbPos {
            x: x.max(0),
            y: y.max(0),
        };
        if let Some(layer) = self.layer.as_ref() {
            layer.set_margin(self.pos.y, 0, 0, self.pos.x);
            layer.commit();
        }
    }

    fn on_left_up(&mut self, x: f64, y: f64) {
        if self.dragging {
            self.dragging = false;
            self.press = None;
            self.persist_pos();
            return;
        }
        self.press = None;
        if self.face == OrbFace::Error && draw::hit_error_esc(x, y, self.width, self.height, self.face)
        {
            crate::daemon::clear_last_error();
            self.error.clear();
            return;
        }
        if crate::daemon::is_session_busy() {
            return;
        }
        if self.face == OrbFace::Error {
            return;
        }
        if draw::hit_orb(x, y, self.width, self.height, self.face) {
            super::spawn_orb_click();
        }
    }

    fn accept_offer(&self, offer: &WlDataOffer, serial: u32) {
        let mime = preferred_mime(&self.offer_mimes);
        if let Some(m) = mime {
            offer.accept(serial, Some(m.to_string()));
            offer.set_actions(
                wayland_client::protocol::wl_data_device_manager::DndAction::Copy,
                wayland_client::protocol::wl_data_device_manager::DndAction::Copy,
            );
        }
    }

    fn finish_drop(&mut self) {
        let Some(offer) = self.offer.take() else {
            return;
        };
        let mimes = std::mem::take(&mut self.offer_mimes);
        let Some(mime) = preferred_mime(&mimes) else {
            offer.destroy();
            return;
        };
        if let Some(bytes) = receive_offer(&offer, mime) {
            let drops = drops_from_mime(mime, &bytes);
            if !drops.is_empty() {
                super::spawn_orb_drop(drops);
            }
        }
        offer.finish();
        offer.destroy();
    }
}

fn preferred_mime(mimes: &[String]) -> Option<&str> {
    const WANT: &[&str] = &[
        "text/uri-list",
        "text/plain;charset=utf-8",
        "text/plain",
        "image/png",
        "image/jpeg",
    ];
    for w in WANT {
        if mimes.iter().any(|m| m == w) {
            return Some(*w);
        }
    }
    None
}

fn drops_from_mime(mime: &str, bytes: &[u8]) -> Vec<OrbDrop> {
    if mime == "text/uri-list" {
        let raw = String::from_utf8_lossy(bytes);
        return policy::parse_uri_list(&raw)
            .into_iter()
            .map(OrbDrop::File)
            .collect();
    }
    if mime.starts_with("image/") {
        return vec![OrbDrop::Image {
            png: bytes.to_vec(),
        }];
    }
    let text = String::from_utf8_lossy(bytes).trim().to_string();
    if text.is_empty() {
        Vec::new()
    } else {
        vec![OrbDrop::Text(text)]
    }
}

fn receive_offer(offer: &WlDataOffer, mime: &str) -> Option<Vec<u8>> {
    let (mut reader, writer) = std::io::pipe().ok()?;
    offer.receive(mime.to_string(), writer.as_fd());
    drop(writer);
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf).ok()?;
    Some(buf)
}

/// Queue handle for `wl_region` (no user data). Built from a leaked empty
/// queue so region create works off the Orb event queue's types.
fn wayland_dummy_qh() -> QueueHandle<Orb> {
    // Replaced at map() time — we never call this standalone. The real qh is
    // recovered from the layer surface's queue via a thread-local set in map.
    ORB_QH.with(|slot| {
        slot.borrow()
            .clone()
            .expect("orb queue handle")
    })
}

thread_local! {
    static ORB_QH: std::cell::RefCell<Option<QueueHandle<Orb>>> = const { std::cell::RefCell::new(None) };
}

impl CompositorHandler for Orb {
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
        if self.mapped {
            self.draw(qh);
        }
    }
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        output: &wl_output::WlOutput,
    ) {
        if let Some(info) = self.output_state.info(output) {
            if let Some(name) = info.name {
                self.output_name = name.clone();
                if let Some(p) = self.store.get(&name) {
                    self.move_to(p.x, p.y);
                }
            }
        }
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

impl OutputHandler for Orb {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for Orb {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.mapped = false;
        self.layer = None;
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
        self.apply_input_region();
        self.draw(qh);
    }
}

impl SeatHandler for Orb {
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
        // Pointer only. Do not bind a keyboard — idle Orb has no key grab.
        if capability == Capability::Pointer && self.pointer.is_none() {
            if let Ok(p) = self.seat_state.get_pointer(qh, &seat) {
                self.pointer = Some(p);
            }
        }
        if self.data_device.is_none() {
            if let Some(ddm) = self.ddm.as_ref() {
                self.data_device = Some(ddm.get_data_device(&seat, qh, ()));
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
        if capability == Capability::Pointer {
            if let Some(p) = self.pointer.take() {
                p.release();
            }
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl PointerHandler for Orb {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        let Some(layer) = self.layer.as_ref() else {
            return;
        };
        let surface = layer.wl_surface().clone();
        for event in events {
            if &event.surface != &surface {
                continue;
            }
            match event.kind {
                PointerEventKind::Motion { .. } => {
                    self.look_x = event.position.0 as f32;
                    self.look_y = event.position.1 as f32;
                    if let Some((px, py, ox, oy)) = self.press {
                        let dx = event.position.0 - px;
                        let dy = event.position.1 - py;
                        if self.dragging || dx * dx + dy * dy >= DRAG_PX * DRAG_PX {
                            self.dragging = true;
                            self.move_to(ox + dx as i32, oy + dy as i32);
                        }
                    }
                    self.draw(qh);
                }
                PointerEventKind::Press { button, .. } if button == BTN_LEFT => {
                    if self.face == OrbFace::Idle || self.face == OrbFace::Error {
                        self.press = Some((event.position.0, event.position.1, self.pos.x, self.pos.y));
                    }
                }
                PointerEventKind::Press { button, .. } if button == BTN_RIGHT => {
                    super::hide();
                    self.hidden = true;
                    self.unmap();
                }
                PointerEventKind::Release { button, .. } if button == BTN_LEFT => {
                    self.on_left_up(event.position.0, event.position.1);
                }
                _ => {}
            }
        }
    }
}

impl ShmHandler for Orb {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_compositor!(Orb);
delegate_output!(Orb);
delegate_shm!(Orb);
delegate_seat!(Orb);
delegate_pointer!(Orb);
delegate_layer!(Orb);
delegate_registry!(Orb);

impl ProvidesRegistryState for Orb {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

impl Dispatch<WlDataDeviceManager, ()> for Orb {
    fn event(
        _: &mut Self,
        _: &WlDataDeviceManager,
        _: wl_data_device_manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlDataDevice, ()> for Orb {
    fn event(
        state: &mut Self,
        _: &WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::Enter {
                serial,
                surface,
                id,
                ..
            } => {
                let ours = state
                    .layer
                    .as_ref()
                    .map(|l| l.wl_surface() == &surface)
                    .unwrap_or(false);
                if !ours {
                    if let Some(offer) = id {
                        offer.destroy();
                    }
                    return;
                }
                state.drop_serial = serial;
                state.offer_mimes.clear();
                if let Some(offer) = id {
                    state.accept_offer(&offer, serial);
                    state.offer = Some(offer);
                }
            }
            wl_data_device::Event::Leave => {
                if let Some(offer) = state.offer.take() {
                    offer.destroy();
                }
                state.offer_mimes.clear();
            }
            wl_data_device::Event::Drop => {
                if !crate::daemon::is_session_busy() && super::is_shown() {
                    state.finish_drop();
                } else if let Some(offer) = state.offer.take() {
                    offer.destroy();
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<WlDataOffer, ()> for Orb {
    fn event(
        state: &mut Self,
        _: &WlDataOffer,
        event: wl_data_offer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = event {
            state.offer_mimes.push(mime_type);
        }
    }
}

impl Dispatch<WlRegion, ()> for Orb {
    fn event(
        _: &mut Self,
        _: &WlRegion,
        _: wayland_client::protocol::wl_region::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

// Keep EventQueue in scope for type-check of prepare_read.
#[allow(dead_code)]
fn _eq_ty(_: &EventQueue<Orb>, _: &GlobalList, _: Duration) {}
