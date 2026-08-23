//! NSApplication accessory host. Overlay and the event tap live on the main
//! thread so OpenAtat never becomes the active app.

use std::ptr::NonNull;
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock};

use objc2::runtime::AnyObject;
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSEvent, NSEventMask};
use objc2_foundation::{MainThreadMarker, NSThread};

use crate::a11y::{self, MouseUpHit};
use crate::error::{Error, Result};
use crate::trigger::{ImeBackend, MacTapBackend};

static MAIN_JOBS: OnceLock<Mutex<Option<Sender<Box<dyn FnOnce() + Send>>>>> = OnceLock::new();
static MOUSE_MONITOR: Mutex<Option<Retained<AnyObject>>> = Mutex::new(None);

pub fn is_main_thread() -> bool {
    NSThread::isMainThread()
}

pub fn ensure_app() -> Result<MainThreadMarker> {
    let mtm = MainThreadMarker::new().ok_or_else(|| {
        Error::msg("NSPanel / event tap require the process main thread")
    })?;
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    Ok(mtm)
}

/// Run `f` on the AppKit main thread. Deadlock-safe if already there.
pub fn call_on_main<T, F>(f: F) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    if is_main_thread() {
        return f();
    }
    let (tx, rx) = mpsc::sync_channel(1);
    enqueue(Box::new(move || {
        let _ = tx.send(f());
    }));
    rx.recv().expect("main-thread job dropped")
}

pub fn enqueue_main(job: impl FnOnce() + Send + 'static) {
    enqueue(Box::new(job));
}

fn enqueue(job: Box<dyn FnOnce() + Send>) {
    if let Some(tx) = MAIN_JOBS.get().and_then(|m| m.lock().ok().and_then(|g| g.clone())) {
        let _ = tx.send(job);
        return;
    }
    // --demo / tests: we should already be on the main thread.
    job();
}

pub fn install_mouse_up_on_main(tx: Sender<MouseUpHit>) {
    enqueue(Box::new(move || unsafe { install_mouse_up_now(tx) }));
}

unsafe fn install_mouse_up_now(tx: Sender<MouseUpHit>) {
    let tx = std::sync::Arc::new(tx);
    let handler = block2::RcBlock::new(move |_event: NonNull<NSEvent>| {
        let pointer = crate::focus::cursor_pos();
        let probe = a11y::probe_selection();
        let _ = tx.send(MouseUpHit { probe, pointer });
    });
    let monitor = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
        NSEventMask::LeftMouseUp,
        &handler,
    );
    *MOUSE_MONITOR.lock().unwrap_or_else(|e| e.into_inner()) = monitor;
}

pub fn run_daemon_host(start_socket: impl FnOnce() + Send + 'static) -> Result<()> {
    let mtm = ensure_app()?;
    let (job_tx, job_rx) = mpsc::channel();
    let _ = MAIN_JOBS.set(Mutex::new(Some(job_tx)));

    let mut tap = MacTapBackend::default();
    let _ = tap.start();
    crate::daemon::start_presence_and_selection();
    crate::orb::start();
    std::thread::Builder::new()
        .name("openatat-sock".into())
        .spawn(start_socket)
        .map_err(|e| Error::msg(format!("socket thread: {e}")))?;

    eprintln!("openatatd: idle — Orb NSPanel mapped, overlay unmapped, accessory policy");

    let app = NSApplication::sharedApplication(mtm);
    // Drain jobs alongside NSApp. A zero-timeout poll keeps the socket
    // dispatch responsive without becoming the active app.
    loop {
        while let Ok(job) = job_rx.try_recv() {
            job();
        }
        let mode = objc2_foundation::ns_string!("kCFRunLoopDefaultMode");
        let until = objc2_foundation::NSDate::dateWithTimeIntervalSinceNow(0.05);
        crate::orb::macos_pump();
        if let Some(event) = app.nextEventMatchingMask_untilDate_inMode_dequeue(
            NSEventMask::Any,
            Some(&until),
            mode,
            true,
        ) {
            if !crate::orb::macos_handle_event(&event) {
                app.sendEvent(&event);
            }
        }
    }
}
