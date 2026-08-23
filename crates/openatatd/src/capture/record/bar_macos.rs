//! NSPanel + NSWindowStyleMaskNonactivatingPanel stop bar.
//!
//! Elapsed + Stop. Esc cancels. Drag empty background. Never activates.
//! Keyboard OnDemand only while the bar is up.

use std::time::Instant;

use objc2::rc::Retained;
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSEvent, NSEventType, NSImage, NSImageView, NSPanel,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{ns_string, MainThreadMarker, NSData, NSDate, NSPoint, NSRect, NSSize};

use super::bar::{self, BarHit, BAR_H, BAR_W};
use super::BarEnd;
use crate::error::{Error, Result};
use crate::macos_runtime;

pub fn run() -> Result<BarEnd> {
    if !macos_runtime::is_main_thread() {
        return macos_runtime::call_on_main(run_on_main);
    }
    run_on_main()
}

fn run_on_main() -> Result<BarEnd> {
    let mtm = macos_runtime::ensure_app()?;
    let started = Instant::now();
    let panel = create_panel(mtm)?;
    let view = attach_image_view(&panel);
    paint(&view, started.elapsed().as_secs());
    panel.orderFrontRegardless();

    let mut drag: Option<(f64, f64, f64, f64)> = None;
    let mut end: Option<BarEnd> = None;
    let mut last_secs = 0u64;
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
                        end = Some(BarEnd::Cancel);
                    }
                }
                NSEventType::LeftMouseDown => {
                    let loc = event.locationInWindow();
                    let y = BAR_H as f64 - loc.y;
                    match bar::hit(loc.x, y) {
                        BarHit::Stop => end = Some(BarEnd::Stop),
                        BarHit::Drag => {
                            let frame = panel.frame();
                            drag = Some((loc.x, loc.y, frame.origin.x, frame.origin.y));
                        }
                    }
                }
                NSEventType::LeftMouseDragged => {
                    if let Some((px, py, ox, oy)) = drag {
                        let loc = event.locationInWindow();
                        let nx = ox + (loc.x - px);
                        let ny = oy + (loc.y - py);
                        let frame = panel.frame();
                        panel.setFrame_display(
                            NSRect::new(NSPoint::new(nx, ny), frame.size),
                            true,
                        );
                    }
                }
                NSEventType::LeftMouseUp => {
                    drag = None;
                }
                _ => {}
            }
            app.sendEvent(&event);
        }
        let secs = started.elapsed().as_secs();
        if secs != last_secs {
            last_secs = secs;
            paint(&view, secs);
        }
    }
    panel.orderOut(None);
    Ok(end.unwrap_or(BarEnd::Cancel))
}

fn create_panel(mtm: MainThreadMarker) -> Result<Retained<NSPanel>> {
    let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
    let screen = objc2_app_kit::NSScreen::mainScreen()
        .ok_or_else(|| Error::msg("no NSScreen for record bar"))?;
    let sf = screen.frame();
    let x = ((sf.size.width - BAR_W as f64) / 2.0).max(8.0);
    let y = sf.origin.y + sf.size.height - BAR_H as f64 - 24.0;
    let rect = NSRect::new(
        NSPoint::new(x, y),
        NSSize::new(BAR_W as f64, BAR_H as f64),
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
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    panel.setTitle(ns_string!("OpenAtat Record"));
    panel.setFrame_display(rect, true);
    Ok(panel)
}

fn attach_image_view(panel: &NSPanel) -> Retained<NSImageView> {
    let mtm = MainThreadMarker::new().expect("main");
    let view = unsafe {
        NSImageView::initWithFrame(
            NSImageView::alloc(mtm),
            NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(BAR_W as f64, BAR_H as f64),
            ),
        )
    };
    if let Some(content) = panel.contentView() {
        content.addSubview(&view);
    }
    view
}

fn paint(view: &NSImageView, elapsed: u64) {
    let pixels = bar::render(BAR_W, BAR_H, elapsed);
    if let Ok(png) = bgra_to_png(BAR_W, BAR_H, &pixels) {
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
        .ok_or_else(|| Error::msg("record bar RGBA size mismatch"))?;
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;
    Ok(png)
}
