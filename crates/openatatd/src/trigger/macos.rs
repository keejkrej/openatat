//! Listen-only CGEvent tap. Feeds [`ImeFilter`]. Not a global summon hotkey.
//!
//! Input Monitoring is optional: a missing tap is logged and `--demo` / the
//! unix socket keep working.

use std::ffi::c_void;
use std::sync::Mutex;

use crate::a11y;
use crate::daemon;
use crate::trigger::macos_policy::{
    feed_filter, input_monitoring_hint, is_ime_composing, should_swallow_trigger, MacKeyInput,
};
use crate::trigger::{ImeAction, ImeBackend, ImeFilter};
use openatat_ipc::TriggerSource;

static FILTER: Mutex<ImeFilter> = Mutex::new(ImeFilter::new());

pub struct MacTapBackend {
    started: bool,
}

impl Default for MacTapBackend {
    fn default() -> Self {
        Self { started: false }
    }
}

impl ImeBackend for MacTapBackend {
    fn name(&self) -> &'static str {
        "macos-event-tap"
    }

    fn start(&mut self) -> Result<(), String> {
        if self.started {
            return Ok(());
        }
        self.started = true;
        match start_listen_only_tap() {
            Ok(()) => {
                eprintln!(
                    "openatatd: listen-only event tap installed — product @@ trigger is ImeFilter"
                );
                Ok(())
            }
            Err(e) => {
                eprintln!("{e}");
                // Never fail the daemon. Socket / --demo stay available.
                Ok(())
            }
        }
    }
}

/// One key from the tap. Used by tests via [`apply_key`].
pub fn apply_key(input: MacKeyInput) -> ImeAction {
    let mut g = FILTER.lock().unwrap_or_else(|e| e.into_inner());
    feed_filter(&mut g, &input)
}

pub fn on_trigger_fired(kind: crate::trigger::FieldKind, action: ImeAction) {
    if !should_swallow_trigger(kind, action) {
        return;
    }
    if let Err(e) = a11y::macos_swallow_trigger() {
        eprintln!("openatatd: AX swallow of @@ failed ({e}); overlay still opens");
    }
    daemon::summon_from_ime();
}

mod ffi {
    use std::ffi::c_void;

    pub type CGEventRef = *mut c_void;
    pub type CGEventTapProxy = *mut c_void;
    pub type CFMachPortRef = *mut c_void;
    pub type CFRunLoopSourceRef = *mut c_void;
    pub type CFRunLoopRef = *mut c_void;
    pub type CFStringRef = *const c_void;

    pub const K_CG_SESSION_EVENT_TAP: u32 = 1;
    pub const K_CG_HEAD_INSERT_EVENT_TAP: u32 = 0;
    pub const K_CG_EVENT_TAP_OPTION_LISTEN_ONLY: u32 = 1;
    pub const K_CG_EVENT_KEY_DOWN: u32 = 10;
    pub const K_CG_KEYBOARD_EVENT_KEYCODE: u32 = 9;

    pub type CGEventTapCallBack = Option<
        unsafe extern "C" fn(
            proxy: CGEventTapProxy,
            etype: u32,
            event: CGEventRef,
            user: *mut c_void,
        ) -> CGEventRef,
    >;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        pub fn CGEventTapCreate(
            tap: u32,
            place: u32,
            options: u32,
            events_of_interest: u64,
            callback: CGEventTapCallBack,
            user_info: *mut c_void,
        ) -> CFMachPortRef;
        pub fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
        pub fn CGEventKeyboardGetUnicodeString(
            event: CGEventRef,
            max_len: usize,
            actual: *mut usize,
            buffer: *mut u16,
        );
        pub fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
        pub fn CGEventGetFlags(event: CGEventRef) -> u64;
        pub fn CGEventGetType(event: CGEventRef) -> u32;
        pub fn CFMachPortCreateRunLoopSource(
            alloc: *mut c_void,
            port: CFMachPortRef,
            order: isize,
        ) -> CFRunLoopSourceRef;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        pub fn CFRunLoopGetCurrent() -> CFRunLoopRef;
        pub fn CFRunLoopAddSource(rl: CFRunLoopRef, src: CFRunLoopSourceRef, mode: CFStringRef);
        pub fn CFRelease(cf: *mut c_void);
        pub static kCFRunLoopCommonModes: CFStringRef;
    }

    #[link(name = "Carbon", kind = "framework")]
    extern "C" {
        pub fn IsSecureEventInputEnabled() -> u8;
        pub fn TISCopyCurrentKeyboardInputSource() -> *mut c_void;
        pub fn TISGetInputSourceProperty(source: *mut c_void, key: CFStringRef) -> *const c_void;
        pub static kTISPropertyInputSourceType: CFStringRef;
        pub static kTISTypeKeyboardLayout: CFStringRef;
    }
}

fn start_listen_only_tap() -> Result<(), String> {
    unsafe {
        let mask = 1u64 << ffi::K_CG_EVENT_KEY_DOWN;
        let tap = ffi::CGEventTapCreate(
            ffi::K_CG_SESSION_EVENT_TAP,
            ffi::K_CG_HEAD_INSERT_EVENT_TAP,
            ffi::K_CG_EVENT_TAP_OPTION_LISTEN_ONLY,
            mask,
            Some(tap_callback),
            std::ptr::null_mut(),
        );
        if tap.is_null() {
            return Err(input_monitoring_hint().to_string());
        }
        let src = ffi::CFMachPortCreateRunLoopSource(std::ptr::null_mut(), tap, 0);
        if src.is_null() {
            ffi::CFRelease(tap);
            return Err(input_monitoring_hint().to_string());
        }
        ffi::CFRunLoopAddSource(
            ffi::CFRunLoopGetCurrent(),
            src,
            ffi::kCFRunLoopCommonModes,
        );
        ffi::CGEventTapEnable(tap, true);
        ffi::CFRelease(src);
        // tap is retained by the run-loop source.
        Ok(())
    }
}

unsafe extern "C" fn tap_callback(
    _proxy: ffi::CGEventTapProxy,
    etype: u32,
    event: ffi::CGEventRef,
    _user: *mut c_void,
) -> ffi::CGEventRef {
    if etype != ffi::K_CG_EVENT_KEY_DOWN {
        return event;
    }
    if daemon::is_session_busy() {
        return event;
    }
    let flags = ffi::CGEventGetFlags(event);
    let keycode = ffi::CGEventGetIntegerValueField(event, ffi::K_CG_KEYBOARD_EVENT_KEYCODE);
    // ⌘⇧V opens the shelf. Not a @@ summon. Listen-only: we do not swallow.
    let cmd = flags & 0x0010_0000 != 0;
    let shift = flags & 0x0002_0000 != 0;
    let ctrl = flags & 0x0004_0000 != 0;
    let alt = flags & 0x0008_0000 != 0;
    if crate::shelf::is_shelf_shortcut(
        crate::shelf::ShelfOs::Mac,
        cmd,
        shift,
        ctrl,
        alt,
        false,
        if keycode == 9 { 'v' } else { '?' },
    ) {
        daemon::summon_shelf();
        return event;
    }
    // ⌘⇧3 / ⌘⇧4 / ⌘⇧5 — listen-only, only because this tap already exists
    // (Input Monitoring granted). Do not swallow OS screenshot keys.
    let capture_key = match keycode {
        20 => Some('3'), // kVK_ANSI_3
        21 => Some('4'), // kVK_ANSI_4
        23 => Some('5'), // kVK_ANSI_5
        _ => None,
    };
    if let Some(key) = capture_key {
        if let Some(kind) = crate::capture::policy::capture_kind_for_key(key) {
            if crate::capture::policy::is_capture_shortcut(
                crate::capture::policy::CaptureOs::Mac,
                kind,
                cmd,
                shift,
                ctrl,
                alt,
                false,
                key,
            ) {
                daemon::summon_capture(kind);
                return event;
            }
        }
    }
    // Listen-only: we never modify or swallow the CGEvent. @@ is removed via AX.
    let secure_event_input = ffi::IsSecureEventInputEnabled() != 0;
    let ax_secure = a11y::probe_field_kind() == crate::trigger::FieldKind::Secure;
    let committed = unicode_from_event(event);
    let marked = a11y::macos_has_marked_text();
    let input_method = tis_is_input_method();
    let composing = is_ime_composing(committed.as_deref(), marked, input_method);
    let input = MacKeyInput {
        secure_event_input,
        ax_secure,
        composing,
        committed,
    };
    let action = apply_key(input);
    if action == ImeAction::FireTrigger {
        let kind = crate::trigger::macos_policy::field_kind(secure_event_input, ax_secure);
        on_trigger_fired(kind, action);
    }
    event
}

fn unicode_from_event(event: ffi::CGEventRef) -> Option<String> {
    unsafe {
        let mut actual = 0usize;
        let mut buf = [0u16; 8];
        ffi::CGEventKeyboardGetUnicodeString(event, buf.len(), &mut actual, buf.as_mut_ptr());
        if actual == 0 {
            return None;
        }
        String::from_utf16(&buf[..actual.min(buf.len())]).ok()
    }
}

fn tis_is_input_method() -> bool {
    // Input Method / Input Mode sources are composing-capable. A plain
    // keyboard layout is not. Empty committed unicode + IM ⇒ compose
    // (see macos_policy::is_ime_composing).
    unsafe {
        let src = ffi::TISCopyCurrentKeyboardInputSource();
        if src.is_null() {
            return false;
        }
        let ty = ffi::TISGetInputSourceProperty(src, ffi::kTISPropertyInputSourceType);
        let is_im = !ty.is_null() && ty != ffi::kTISTypeKeyboardLayout as *const c_void;
        ffi::CFRelease(src);
        is_im
    }
}

/// Re-probe every call. Never cache.
pub fn is_secure_event_input() -> bool {
    unsafe { ffi::IsSecureEventInputEnabled() != 0 }
}
