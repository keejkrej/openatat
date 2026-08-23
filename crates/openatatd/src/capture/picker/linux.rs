//! Native `zwlr_layer_shell_v1` region picker. Overlay layer, software
//! `wl_shm`, dimmed rubber-band. `KeyboardInteractivity::OnDemand` only
//! while this surface is mapped. Esc cancels. Not slurp. Not grim -g.
//! Not an xdg-toplevel and not gpui.

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

use super::draw::{self, Rect};
use super::PickedRegion;
use crate::error::{Error, Result};
use crate::focus::OutputGeom;

const BTN_LEFT: u32 = 0x110;

#[derive(Debug, Clone)]
enum End {
    Cancel,
    Region(PickedRegion),
}

pub fn pick_region() -> Result<Option<PickedRegion>> {
    let target = super::output_for_picker();
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
        Some("openatat-pick"),
        None,
    );
    layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
    layer.set_exclusive_zone(-1);
    layer.set_margin(0, 0, 0, 0);
    layer.set_keyboard_interactivity(KeyboardInteractivity::OnDemand);
    layer.set_size(0, 0);
    layer.commit();

    let pool = SlotPool::new((1920 * 1080 * 4) as usize, &shm)
        .map_err(|e| Error::msg(format!("wl_shm pool: {e}")))?;

    let mut picker = Picker {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        pool,
        layer,
        width: 1,
        height: 1,
        keyboard: None,
        pointer: None,
        press: None,
        current: None,
        end: None,
        dirty: true,
        target,
    };

    while picker.end.is_none() {
        event_queue
            .blocking_dispatch(&mut picker)
            .map_err(|e| Error::msg(format!("wayland dispatch: {e}")))?;
    }

    match picker.end {
        Some(End::Region(r)) => Ok(Some(r)),
        _ => Ok(None),
    }
}

struct Picker {
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
    press: Option<(f64, f64)>,
    current: Option<(f64, f64)>,
    end: Option<End>,
    dirty: bool,
    target: Option<OutputGeom>,
}

impl Picker {
    fn selection(&self) -> Option<Rect> {
        let (x0, y0) = self.press?;
        let (x1, y1) = self.current?;
        let (x, y, w, h) = super::super::policy::normalize_rect(
            x0.round() as i32,
            y0.round() as i32,
            x1.round() as i32,
            y1.round() as i32,
        )?;
        Some(Rect { x, y, w, h })
    }

    fn finish_from_surface(&self, local: Rect) -> PickedRegion {
        let (gx, gy, gw, gh) = if let Some(out) = self.target.as_ref() {
            let mapped = super::super::policy::map_surface_rect_to_image(
                local.x,
                local.y,
                local.w,
                local.h,
                self.width,
                self.height,
                out.width,
                out.height,
            )
            .unwrap_or((
                local.x.max(0) as u32,
                local.y.max(0) as u32,
                local.w,
                local.h,
            ));
            (
                out.x + mapped.0 as i32,
                out.y + mapped.1 as i32,
                mapped.2,
                mapped.3,
            )
        } else {
            (local.x, local.y, local.w, local.h)
        };
        PickedRegion {
            x: gx,
            y: gy,
            width: gw,
            height: gh,
            output: self.target.as_ref().map(|o| o.name.clone()),
            surface_w: self.width,
            surface_h: self.height,
        }
    }

    fn draw(&mut self, qh: &QueueHandle<Self>) {
        let width = self.width.max(1);
        let height = self.height.max(1);
        let stride = width as i32 * 4;
        let pixels = draw::render(width, height, self.selection());
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

impl CompositorHandler for Picker {
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

impl OutputHandler for Picker {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for Picker {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.end = Some(End::Cancel);
    }

    fn configure(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        self.width = NonZeroU32::new(configure.new_size.0)
            .map(NonZeroU32::get)
            .or_else(|| self.target.as_ref().map(|o| o.width))
            .unwrap_or(1920);
        self.height = NonZeroU32::new(configure.new_size.1)
            .map(NonZeroU32::get)
            .or_else(|| self.target.as_ref().map(|o| o.height))
            .unwrap_or(1080);
        self.draw(qh);
    }
}

impl SeatHandler for Picker {
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

impl KeyboardHandler for Picker {
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
            self.end = Some(End::Cancel);
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

impl PointerHandler for Picker {
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
                    self.press = Some(event.position);
                    self.current = Some(event.position);
                    self.dirty = true;
                    self.draw(qh);
                }
                PointerEventKind::Motion { .. } if self.press.is_some() => {
                    self.current = Some(event.position);
                    self.dirty = true;
                    self.draw(qh);
                }
                PointerEventKind::Release { button, .. } if button == BTN_LEFT => {
                    self.current = Some(event.position);
                    if let Some(local) = self.selection() {
                        self.end = Some(End::Region(self.finish_from_surface(local)));
                    } else {
                        self.end = Some(End::Cancel);
                    }
                }
                _ => {}
            }
        }
    }
}

impl ShmHandler for Picker {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_compositor!(Picker);
delegate_output!(Picker);
delegate_shm!(Picker);
delegate_seat!(Picker);
delegate_keyboard!(Picker);
delegate_pointer!(Picker);
delegate_layer!(Picker);
delegate_registry!(Picker);

impl ProvidesRegistryState for Picker {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}
