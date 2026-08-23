//! NSPanel + NSWindowStyleMaskNonactivatingPanel region picker.
//!
//! Full-output dim + rubber-band. Never activates the host app. Esc cancels
//! without opening `@@`. gpui-ce PopUp is not this.

use objc2::rc::Retained;
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSEvent, NSEventType, NSImage, NSImageView, NSPanel,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{ns_string, MainThreadMarker, NSData, NSDate, NSPoint, NSRect, NSSize};

use super::draw::{self, Rect};
use super::PickedRegion;
use crate::error::{Error, Result};
use crate::macos_runtime;

enum End {
    Cancel,
    Region(PickedRegion),
}

pub fn pick_region() -> Result<Option<PickedRegion>> {
    if !macos_runtime::is_main_thread() {
        return macos_runtime::call_on_main(pick_on_main);
    }
    pick_on_main()
}

fn pick_on_main() -> Result<Option<PickedRegion>> {
    let mtm = macos_runtime::ensure_app()?;
    let screen = objc2_app_kit::NSScreen::mainScreen()
        .ok_or_else(|| Error::msg("no NSScreen for area picker"))?;
    let frame = screen.frame();
    let w = frame.size.width.max(1.0) as u32;
    let h = frame.size.height.max(1.0) as u32;
    let panel = create_panel(mtm, frame)?;
    let view = attach_image_view(&panel, w, h);
    paint(&view, w, h, None);
    panel.orderFrontRegardless();

    let mut press: Option<(f64, f64)> = None;
    let mut current: Option<(f64, f64)> = None;
    let mut end: Option<End> = None;
    let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
    while end.is_none() {
        let mode = ns_string!("kCFRunLoopDefaultMode");
        let until = NSDate::dateWithTimeIntervalSinceNow(0.05);
        if let Some(event) = app.nextEventMatchingMask_untilDate_inMode_dequeue(
            objc2_app_kit::NSEventMask::Any,
            Some(&until),
            mode,
            true,
        ) {
            match event.r#type() {
                NSEventType::KeyDown => {
                    let chars = event.characters().map(|s| s.to_string()).unwrap_or_default();
                    if chars == "\u{1b}" || event.keyCode() == 53 {
                        end = Some(End::Cancel);
                    }
                }
                NSEventType::LeftMouseDown => {
                    let loc = event.locationInWindow();
                    let y = h as f64 - loc.y;
                    press = Some((loc.x, y));
                    current = press;
                    paint(&view, w, h, selection(press, current));
                }
                NSEventType::LeftMouseDragged => {
                    let loc = event.locationInWindow();
                    let y = h as f64 - loc.y;
                    current = Some((loc.x, y));
                    paint(&view, w, h, selection(press, current));
                }
                NSEventType::LeftMouseUp => {
                    let loc = event.locationInWindow();
                    let y = h as f64 - loc.y;
                    current = Some((loc.x, y));
                    end = match selection(press, current) {
                        Some(r) => Some(End::Region(PickedRegion {
                            // Surface-local. SCK still is that display; CPU crop.
                            x: r.x,
                            y: r.y,
                            width: r.w,
                            height: r.h,
                            output: Some("main".into()),
                            surface_w: w,
                            surface_h: h,
                        })),
                        None => Some(End::Cancel),
                    };
                }
                _ => {}
            }
            app.sendEvent(&event);
        }
    }
    panel.orderOut(None);
    match end {
        Some(End::Region(r)) => Ok(Some(r)),
        _ => Ok(None),
    }
}

fn selection(press: Option<(f64, f64)>, current: Option<(f64, f64)>) -> Option<Rect> {
    let (x0, y0) = press?;
    let (x1, y1) = current?;
    let (x, y, w, h) = super::super::policy::normalize_rect(
        x0.round() as i32,
        y0.round() as i32,
        x1.round() as i32,
        y1.round() as i32,
    )?;
    Some(Rect { x, y, w, h })
}

fn create_panel(mtm: MainThreadMarker, frame: NSRect) -> Result<Retained<NSPanel>> {
    let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
    let panel = unsafe {
        NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            frame,
            style,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    panel.setFloatingPanel(true);
    panel.setBecomesKeyOnlyIfNeeded(false);
    panel.setHidesOnDeactivate(false);
    panel.setOpaque(false);
    panel.setHasShadow(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setLevel(3);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    panel.setTitle(ns_string!("OpenAtat"));
    panel.setFrame_display(frame, true);
    Ok(panel)
}

fn attach_image_view(panel: &NSPanel, w: u32, h: u32) -> Retained<NSImageView> {
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

fn paint(view: &NSImageView, w: u32, h: u32, sel: Option<Rect>) {
    let pixels = draw::render(w, h, sel);
    if let Ok(png) = bgra_to_png(w, h, &pixels) {
        let data = NSData::with_bytes(&png);
        if let Some(image) = NSImage::initWithData(&NSImage::alloc(), &data) {
            view.setImage(Some(&image));
        }
    }
}

fn bgra_to_png(w: u32, h: u32, bgra: &[u8]) -> Result<Vec<u8>> {
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for (i, px) in bgra.chunks_exact(4).enumerate() {
        let d = i * 4;
        rgba[d] = px[2];
        rgba[d + 1] = px[1];
        rgba[d + 2] = px[0];
        rgba[d + 3] = px[3];
    }
    let img = image::RgbaImage::from_raw(w, h, rgba)
        .ok_or_else(|| Error::msg("picker RGBA size mismatch"))?;
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;
    Ok(png)
}
