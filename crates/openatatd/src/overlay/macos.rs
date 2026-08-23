//! NSPanel + NSWindowStyleMaskNonactivatingPanel.
//!
//! CollectionBehavior: canJoinAllSpaces, fullScreenAuxiliary.
//! Never activates the app. gpui-ce PopUp is not this.

use objc2::rc::Retained;
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSEvent, NSEventModifierFlags, NSEventType, NSImage, NSImageView,
    NSPanel, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSData, NSDate, NSPoint, NSRect, NSSize,
};

use super::controller::{OverlayController, OverlayKey};
use super::draw::{self, BAR_H, BAR_W, POPOVER_H, POPOVER_W};
use super::{OverlayEnd, OverlayKind};
use crate::error::{Error, Result};
use crate::macos_runtime;
use crate::session::Session;

pub fn run(session: &mut Session, kind: OverlayKind) -> Result<OverlayEnd> {
    if !macos_runtime::is_main_thread() {
        let mut owned = session.clone();
        let end = macos_runtime::call_on_main(move || run_on_main(&mut owned, kind));
        *session = owned;
        return end;
    }
    run_on_main(session, kind)
}

fn run_on_main(session: &mut Session, kind: OverlayKind) -> Result<OverlayEnd> {
    let mtm = macos_runtime::ensure_app()?;
    let mut ctl = OverlayController::from_session(session, kind);
    let (w, h) = (ctl.width, ctl.height);
    let panel = create_panel(mtm, w, h)?;
    let view = attach_image_view(&panel, w, h);
    paint(&ctl, &view);

    // Key without activating the host app. NonactivatingPanel keeps the
    // client as the active application.
    panel.makeKeyAndOrderFront(None);
    panel.orderFrontRegardless();

    let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
    while ctl.end.is_none() {
        let mode = ns_string!("kCFRunLoopDefaultMode");
        let until = NSDate::dateWithTimeIntervalSinceNow(0.05);
        if let Some(event) = app.nextEventMatchingMask_untilDate_inMode_dequeue(
            objc2_app_kit::NSEventMask::Any,
            Some(&until),
            mode,
            true,
        ) {
            handle_event(&mut ctl, &event, &panel);
            app.sendEvent(&event);
        }
        if ctl.dirty {
            paint(&ctl, &view);
            ctl.dirty = false;
        }
        apply_resize(&ctl, &panel, &view);
    }

    panel.orderOut(None);
    ctl.write_back(session);
    Ok(ctl.end.unwrap_or(OverlayEnd::Cancelled))
}

fn create_panel(mtm: MainThreadMarker, w: u32, h: u32) -> Result<Retained<NSPanel>> {
    let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
    let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w as f64, h as f64));
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
    panel.setOpaque(true);
    panel.setHasShadow(true);
    panel.setBackgroundColor(Some(&NSColor::blackColor()));
    // NSFloatingWindowLevel = 3. Stays above documents without activating.
    panel.setLevel(3);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    panel.setTitle(ns_string!("OpenAtat"));
    if let Some(screen) = objc2_app_kit::NSScreen::mainScreen() {
        let frame = screen.visibleFrame();
        let x = frame.origin.x + ((frame.size.width - w as f64) / 2.0).max(24.0);
        let y = frame.origin.y + frame.size.height - h as f64 - 80.0;
        panel.setFrameTopLeftPoint(NSPoint::new(x, y + h as f64));
    }
    Ok(panel)
}

fn attach_image_view(panel: &NSPanel, w: u32, h: u32) -> Retained<NSImageView> {
    let mtm = MainThreadMarker::new().expect("main");
    let view = unsafe {
        NSImageView::initWithFrame(NSImageView::alloc(mtm), panel.contentView().map(|v| v.bounds()).unwrap_or(
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w as f64, h as f64)),
        ))
    };
    if let Some(content) = panel.contentView() {
        content.addSubview(&view);
    }
    view
}

fn paint(ctl: &OverlayController, view: &NSImageView) {
    let thumb = ctl.thumb.as_ref().map(|(w, h, p)| (*w, *h, p.as_slice()));
    let pixels = draw::render(ctl.width, ctl.height, &ctl.ui_frame(), thumb);
    if let Ok(png) = bgra_to_png(ctl.width, ctl.height, &pixels) {
        let data = NSData::with_bytes(&png);
        if let Some(image) = NSImage::initWithData(&NSImage::alloc(), &data) {
            view.setImage(Some(&image));
        }
    }
}

fn apply_resize(ctl: &OverlayController, panel: &NSPanel, view: &NSImageView) {
    let frame = panel.frame();
    if frame.size.width as u32 != ctl.width || frame.size.height as u32 != ctl.height {
        let mut next = frame;
        next.size = NSSize::new(ctl.width as f64, ctl.height as f64);
        panel.setFrame_display(next, true);
        view.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(ctl.width as f64, ctl.height as f64),
        ));
    }
}

fn handle_event(ctl: &mut OverlayController, event: &NSEvent, panel: &NSPanel) {
    match event.r#type() {
        NSEventType::KeyDown => {
            let chars = event.characters().map(|s| s.to_string()).unwrap_or_default();
            let flags = event.modifierFlags();
            let meta = flags.contains(NSEventModifierFlags::Command)
                || flags.contains(NSEventModifierFlags::Function);
            let key = map_key(&chars, event.keyCode(), meta);
            let effects = ctl.handle_key(key, Some(chars.as_str()));
            let _ = effects;
            let _ = panel;
        }
        NSEventType::LeftMouseDown => {
            let loc = event.locationInWindow();
            // AppKit y is flipped vs our software buffer (top-left origin).
            let y = ctl.height as f64 - loc.y;
            let _ = ctl.handle_click(loc.x, y);
        }
        _ => {}
    }
}

fn map_key(chars: &str, keycode: u16, meta: bool) -> OverlayKey {
    if chars == "\u{1b}" || keycode == 53 {
        return OverlayKey::Escape;
    }
    if chars == "\r" || chars == "\n" || keycode == 36 || keycode == 76 {
        return OverlayKey::Return { meta };
    }
    if chars == "\t" || keycode == 48 {
        return OverlayKey::Tab;
    }
    if keycode == 51 {
        return OverlayKey::Backspace;
    }
    if chars.eq_ignore_ascii_case("r") {
        return OverlayKey::R;
    }
    if let Some(d) = chars.chars().next().and_then(|c| c.to_digit(10)) {
        if (1..=5).contains(&d) {
            return OverlayKey::Digit(d as u8);
        }
    }
    OverlayKey::Text
}

fn bgra_to_png(width: u32, height: u32, bgra: &[u8]) -> Result<Vec<u8>> {
    let mut rgba = Vec::with_capacity(bgra.len());
    for px in bgra.chunks_exact(4) {
        rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
    }
    let img = image::RgbaImage::from_raw(width, height, rgba)
        .ok_or_else(|| Error::msg("overlay bitmap size mismatch"))?;
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;
    Ok(png)
}

// Keep the constants referenced so a bar-sized panel still type-checks.
#[allow(dead_code)]
fn _bar_dims() -> (u32, u32) {
    (BAR_W.min(POPOVER_W), BAR_H.min(POPOVER_H))
}
