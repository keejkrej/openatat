//! NSPanel + `NSWindowStyleMaskNonactivatingPanel` Orb.
//!
//! Hit-tests the circle. Never becomes the active app. Menu extra toggles
//! Show Orb for this launch. Drag-and-drop uses the pasteboard types, never
//! a Finder title bar.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Mutex;

use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread, ClassType, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSBitmapImageFileType, NSBitmapImageRep, NSColor, NSControlStateValueOff,
    NSControlStateValueOn, NSDraggingInfo, NSDragOperation, NSEvent, NSEventType, NSImage,
    NSImageView, NSMenu, NSMenuItem, NSPanel, NSPasteboard, NSPasteboardTypeFileURL,
    NSPasteboardTypePNG, NSPasteboardTypeString, NSStatusBar, NSStatusItem, NSView,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSArray, NSData, NSPoint, NSRect, NSSize, NSURL,
};

use super::draw::{self, OrbFace, OrbFrame, ORB_BUSY, ORB_ERROR_H, ORB_ERROR_W, ORB_IDLE};
use super::policy::OrbDrop;
use super::position::{self, OrbPos, OrbPositions};
use crate::error::{Error, Result};
use crate::macos_runtime;

static STATUS_ITEM: Mutex<Option<Retained<NSStatusItem>>> = Mutex::new(None);
static ORB_PANEL: Mutex<Option<Retained<NSPanel>>> = Mutex::new(None);
static ORB_VIEW: Mutex<Option<Retained<NSImageView>>> = Mutex::new(None);
static SHOW_ITEM: Mutex<Option<Retained<NSMenuItem>>> = Mutex::new(None);
static POS: Mutex<OrbPos> = Mutex::new(OrbPos { x: 80, y: 80 });
static STORE: Mutex<OrbPositions> = Mutex::new(OrbPositions {
    by_output: std::collections::BTreeMap::new(),
});
static FACE: Mutex<OrbFace> = Mutex::new(OrbFace::Idle);
static ERROR: Mutex<String> = Mutex::new(String::new());
static DRAG: Mutex<Option<(f64, f64, i32, i32)>> = Mutex::new(None);
static DRAGGING: Mutex<bool> = Mutex::new(false);

pub fn start() {
    if !macos_runtime::is_main_thread() {
        macos_runtime::enqueue_main(|| {
            if let Err(e) = start_on_main() {
                eprintln!("openatatd: Orb: {e}");
            }
        });
        return;
    }
    if let Err(e) = start_on_main() {
        eprintln!("openatatd: Orb: {e}");
    }
}

fn start_on_main() -> Result<()> {
    let mtm = macos_runtime::ensure_app()?;
    *STORE.lock().unwrap_or_else(|e| e.into_inner()) = OrbPositions::load();
    let pos = initial_pos();
    *POS.lock().unwrap_or_else(|e| e.into_inner()) = pos;

    let panel = create_panel(mtm, pos)?;
    let view = attach_view(&panel, ORB_IDLE, ORB_IDLE);
    paint_current(&view);
    if super::is_shown() {
        panel.orderFrontRegardless();
    }
    *ORB_PANEL.lock().unwrap_or_else(|e| e.into_inner()) = Some(panel);
    *ORB_VIEW.lock().unwrap_or_else(|e| e.into_inner()) = Some(view);
    install_status_item(mtm)?;
    eprintln!("openatatd: idle — Orb NSPanel mapped, overlay unmapped, accessory policy");
    Ok(())
}

fn initial_pos() -> OrbPos {
    let store = STORE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(p) = store.get("main") {
        return p;
    }
    if let Some(screen) = objc2_app_kit::NSScreen::mainScreen() {
        let f = screen.visibleFrame();
        return position::default_pos(f.size.width as i32, f.size.height as i32, ORB_IDLE);
    }
    OrbPos { x: 80, y: 80 }
}

fn create_panel(mtm: MainThreadMarker, pos: OrbPos) -> Result<Retained<NSPanel>> {
    let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
    let rect = NSRect::new(
        NSPoint::new(pos.x as f64, pos.y as f64),
        NSSize::new(ORB_IDLE as f64, ORB_IDLE as f64),
    );
    let panel = unsafe {
        NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            rect,
            style,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    panel.setFloatingPanel(true);
    panel.setBecomesKeyOnlyIfNeeded(false);
    panel.setHidesOnDeactivate(false);
    panel.setOpaque(false);
    panel.setHasShadow(true);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setLevel(3);
    panel.setIgnoresMouseEvents(false);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    panel.setTitle(ns_string!("OpenAtat Orb"));
    let types = NSArray::from_slice(&[
        unsafe { NSPasteboardTypeFileURL },
        unsafe { NSPasteboardTypeString },
        unsafe { NSPasteboardTypePNG },
    ]);
    panel.registerForDraggedTypes(&types);
    Ok(panel)
}

fn attach_view(panel: &NSPanel, w: u32, h: u32) -> Retained<NSImageView> {
    let mtm = MainThreadMarker::new().expect("main");
    let view = unsafe {
        NSImageView::initWithFrame(
            NSImageView::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w as f64, h as f64)),
        )
    };
    if let Some(content) = panel.contentView() {
        content.addSubview(&view);
    }
    view
}

fn paint_current(view: &NSImageView) {
    let face = *FACE.lock().unwrap_or_else(|e| e.into_inner());
    let (w, h) = draw::size_for(face);
    let look = crate::focus::cursor_pos()
        .map(|(x, y)| {
            let p = *POS.lock().unwrap_or_else(|e| e.into_inner());
            ((x - p.x) as f32, (y - p.y) as f32)
        })
        .unwrap_or((w as f32 / 2.0, h as f32 / 2.0));
    let error = ERROR.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let frame = OrbFrame {
        face,
        look_x: look.0,
        look_y: look.1,
        error,
        pulse: 0.0,
    };
    let pixels = draw::render(w, h, &frame);
    if let Ok(png) = bgra_to_png(w, h, &pixels) {
        let data = NSData::with_bytes(&png);
        if let Some(image) = NSImage::initWithData(&NSImage::alloc(), &data) {
            view.setImage(Some(&image));
        }
    }
}

fn bgra_to_png(width: u32, height: u32, bgra: &[u8]) -> Result<Vec<u8>> {
    let mut rgba = Vec::with_capacity(bgra.len());
    for px in bgra.chunks_exact(4) {
        rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
    }
    let img = image::RgbaImage::from_raw(width, height, rgba)
        .ok_or_else(|| Error::msg("orb bitmap size mismatch"))?;
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;
    Ok(png)
}

pub fn pump() {
    if !super::is_shown() {
        if let Some(panel) = ORB_PANEL.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            panel.orderOut(None);
        }
        return;
    }
    let busy = crate::daemon::is_session_busy();
    let err = crate::daemon::last_error_message();
    {
        let mut e = ERROR.lock().unwrap_or_else(|e| e.into_inner());
        *e = err.clone().unwrap_or_default();
    }
    *FACE.lock().unwrap_or_else(|e| e.into_inner()) =
        draw::face_from_presence(busy, err.as_deref());
    if let Some(view) = ORB_VIEW.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        paint_current(view);
    }
    if let Some(panel) = ORB_PANEL.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let face = *FACE.lock().unwrap_or_else(|e| e.into_inner());
        let (w, h) = draw::size_for(face);
        let pos = *POS.lock().unwrap_or_else(|e| e.into_inner());
        panel.setFrame_display(
            NSRect::new(
                NSPoint::new(pos.x as f64, pos.y as f64),
                NSSize::new(w as f64, h as f64),
            ),
            true,
        );
        panel.orderFrontRegardless();
    }
    if let Some(item) = SHOW_ITEM.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        item.setState(if super::is_shown() {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
    }
}

pub fn handle_event(event: &NSEvent) -> bool {
    let Some(panel) = ORB_PANEL.lock().unwrap_or_else(|e| e.into_inner()).clone() else {
        return false;
    };
    let Some(win) = event.window() else {
        return false;
    };
    if win != *panel {
        return false;
    }
    let face = *FACE.lock().unwrap_or_else(|e| e.into_inner());
    let (w, h) = draw::size_for(face);
    match event.r#type() {
        NSEventType::LeftMouseDown => {
            let loc = event.locationInWindow();
            let y = h as f64 - loc.y;
            if !draw::hit_orb(loc.x, y, w, h, face) {
                return true;
            }
            let pos = *POS.lock().unwrap_or_else(|e| e.into_inner());
            *DRAG.lock().unwrap_or_else(|e| e.into_inner()) =
                Some((loc.x, loc.y, pos.x, pos.y));
            *DRAGGING.lock().unwrap_or_else(|e| e.into_inner()) = false;
            true
        }
        NSEventType::LeftMouseDragged => {
            if let Some((px, py, ox, oy)) = *DRAG.lock().unwrap_or_else(|e| e.into_inner()) {
                let loc = event.locationInWindow();
                let dx = loc.x - px;
                let dy = loc.y - py;
                if dx * dx + dy * dy >= 36.0 {
                    *DRAGGING.lock().unwrap_or_else(|e| e.into_inner()) = true;
                }
                if *DRAGGING.lock().unwrap_or_else(|e| e.into_inner()) {
                    let next = OrbPos {
                        x: ox + dx as i32,
                        y: oy + dy as i32,
                    };
                    *POS.lock().unwrap_or_else(|e| e.into_inner()) = next;
                }
            }
            true
        }
        NSEventType::LeftMouseUp => {
            let loc = event.locationInWindow();
            let y = h as f64 - loc.y;
            let was_drag = *DRAGGING.lock().unwrap_or_else(|e| e.into_inner());
            *DRAG.lock().unwrap_or_else(|e| e.into_inner()) = None;
            *DRAGGING.lock().unwrap_or_else(|e| e.into_inner()) = false;
            if was_drag {
                let pos = *POS.lock().unwrap_or_else(|e| e.into_inner());
                let mut store = STORE.lock().unwrap_or_else(|e| e.into_inner());
                store.set("main".into(), pos);
                store.save();
                return true;
            }
            if face == OrbFace::Error && draw::hit_error_esc(loc.x, y, w, h, face) {
                crate::daemon::clear_last_error();
                return true;
            }
            if crate::daemon::is_session_busy() || face == OrbFace::Error {
                return true;
            }
            super::spawn_orb_click();
            true
        }
        NSEventType::RightMouseUp | NSEventType::RightMouseDown => {
            super::hide();
            panel.orderOut(None);
            true
        }
        _ => false,
    }
}

pub fn ingest_pasteboard(pb: &NSPasteboard) {
    let mut drops = Vec::new();
    if let Some(urls) = unsafe { pb.readObjectsForClasses_options(&NSArray::from_slice(&[NSURL::class()]), None) }
    {
        for obj in urls.iter() {
            let url = obj.downcast_ref::<NSURL>();
            if let Some(url) = url {
                if let Some(path) = unsafe { url.path() } {
                    let p = PathBuf::from(path.to_string());
                    if p.is_absolute() {
                        drops.push(OrbDrop::File(p));
                    }
                }
            }
        }
    }
    if drops.is_empty() {
        if let Some(s) = unsafe { pb.stringForType(NSPasteboardTypeString) } {
            let t = s.to_string();
            if !t.is_empty() {
                drops.push(OrbDrop::Text(t));
            }
        }
    }
    if drops.is_empty() {
        if let Some(data) = unsafe { pb.dataForType(NSPasteboardTypePNG) } {
            drops.push(OrbDrop::Image {
                png: data.to_vec(),
            });
        }
    }
    if !drops.is_empty() && !crate::daemon::is_session_busy() {
        super::spawn_orb_drop(drops);
    }
}

fn install_status_item(mtm: MainThreadMarker) -> Result<()> {
    let bar = NSStatusBar::systemStatusBar();
    let item = unsafe { bar.statusItemWithLength(-1.0) };
    if let Some(button) = unsafe { item.button() } {
        button.setTitle(ns_string!("@@"));
        button.setToolTip(Some(ns_string!("OpenAtat Orb")));
    }
    let menu = NSMenu::new(mtm);
    let show = NSMenuItem::new(mtm);
    show.setTitle(ns_string!("Show Orb"));
    show.setState(NSControlStateValueOn);
    let target = OrbMenuTarget::new(mtm);
    unsafe {
        show.setTarget(Some(&target));
        show.setAction(Some(objc2::sel!(toggleShowOrb:)));
    }
    menu.addItem(&show);
    item.setMenu(Some(&menu));
    *SHOW_ITEM.lock().unwrap_or_else(|e| e.into_inner()) = Some(show);
    *STATUS_ITEM.lock().unwrap_or_else(|e| e.into_inner()) = Some(item);
    let _ = (ORB_BUSY, ORB_ERROR_W, ORB_ERROR_H, NSBitmapImageFileType::PNG, NSBitmapImageRep::class());
    Ok(())
}

struct TargetIvars;

define_class!(
    #[unsafe(super(objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = TargetIvars]
    struct OrbMenuTarget;

    impl OrbMenuTarget {
        #[unsafe(method(toggleShowOrb:))]
        fn toggle_show_orb(&self, _sender: Option<&AnyObject>) {
            if super::is_shown() {
                super::hide();
            } else {
                super::show();
            }
        }
    }
);

impl OrbMenuTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this: Allocated<Self> = mtm.alloc().set_ivars(TargetIvars);
        unsafe { msg_send![super(this), init] }
    }
}

// Keep dragging types referenced so a drop onto the panel is a real target.
#[allow(dead_code)]
fn _drag_ops() -> NSDragOperation {
    NSDragOperation::Copy
}

#[allow(dead_code)]
fn _dragging_info(_: Option<&ProtocolObject<dyn NSDraggingInfo>>) {}
