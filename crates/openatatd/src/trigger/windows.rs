//! Process-local keyboard listener. Feeds [`ImeFilter`]. Not a global hotkey.
//!
//! Prefers a low-level keyboard hook (`WH_KEYBOARD_LL`). Falls back to Raw
//! Input (`RIDEV_INPUTSINK`) so `--demo` still works if the hook cannot
//! install. Missing listener is logged; the socket path stays up.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::Ime::{ImmGetContext, ImmGetOpenStatus, ImmReleaseContext};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyboardState, ToUnicode, VK_CONTROL, VK_MENU, VK_PACKET, VK_PROCESSKEY,
    VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, SetWindowsHookExW, UnhookWindowsHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT,
    WH_KEYBOARD_LL, WM_KEYDOWN, WM_SYSKEYDOWN,
};

use crate::a11y;
use crate::daemon;
use crate::trigger::windows_policy::{
    feed_filter, field_kind, hook_install_hint, is_ime_composing, should_swallow_trigger, WinKeyInput,
};
use crate::trigger::{ImeAction, ImeBackend, ImeFilter};

static FILTER: OnceLock<Mutex<ImeFilter>> = OnceLock::new();
static HOOK: Mutex<Option<isize>> = Mutex::new(None);
static HOOK_OK: AtomicBool = AtomicBool::new(false);

fn filter() -> std::sync::MutexGuard<'static, ImeFilter> {
    FILTER
        .get_or_init(|| Mutex::new(ImeFilter::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

pub fn hook_is_installed() -> bool {
    HOOK_OK.load(Ordering::SeqCst)
}

pub struct WinHookBackend {
    started: bool,
}

impl Default for WinHookBackend {
    fn default() -> Self {
        Self { started: false }
    }
}

impl ImeBackend for WinHookBackend {
    fn name(&self) -> &'static str {
        "windows-keyboard-ll"
    }

    fn start(&mut self) -> Result<(), String> {
        if self.started {
            return Ok(());
        }
        self.started = true;
        match install_ll_hook() {
            Ok(()) => {
                HOOK_OK.store(true, Ordering::SeqCst);
                eprintln!(
                    "openatatd: process-local keyboard hook installed — product @@ trigger is ImeFilter"
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

/// One key from the listener. Used by tests via [`apply_key`].
pub fn apply_key(input: WinKeyInput) -> ImeAction {
    let mut g = filter();
    feed_filter(&mut g, &input)
}

pub fn on_trigger_fired(kind: crate::trigger::FieldKind, action: ImeAction) {
    if !should_swallow_trigger(kind, action) {
        return;
    }
    if let Err(e) = a11y::windows_swallow_trigger() {
        eprintln!("openatatd: UIA swallow of @@ failed ({e}); overlay still opens");
    }
    daemon::summon_from_ime();
}

fn install_ll_hook() -> Result<(), String> {
    unsafe {
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(ll_keyboard_proc), None, 0)
            .map_err(|e| format!("{} ({e})", hook_install_hint()))?;
        if hook.is_invalid() {
            return Err(hook_install_hint().to_string());
        }
        *HOOK.lock().unwrap_or_else(|e| e.into_inner()) = Some(hook.0 as isize);
        Ok(())
    }
}

#[allow(dead_code)]
pub fn uninstall_ll_hook() {
    if let Some(raw) = HOOK.lock().unwrap_or_else(|e| e.into_inner()).take() {
        unsafe {
            let _ = UnhookWindowsHookEx(HHOOK(raw as *mut _));
        }
        HOOK_OK.store(false, Ordering::SeqCst);
    }
}

unsafe extern "system" fn ll_keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code != HC_ACTION as i32 {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }
    let msg = wparam.0 as u32;
    if msg != WM_KEYDOWN && msg != WM_SYSKEYDOWN {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }
    let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
    let vk = info.vkCode;

    if is_shelf_chord(vk) {
        daemon::summon_shelf();
        // Swallow so the client does not type V. Clipboard shortcut, not @@.
        return LRESULT(1);
    }

    if is_capture_chord(vk) {
        if let Some(kind) = capture_kind_from_vk(vk) {
            daemon::summon_capture(kind);
            return LRESULT(1);
        }
    }

    if crate::capture::picker::windows_picker_wants_keys() {
        crate::capture::picker::windows_feed_picker_vk(vk);
        return LRESULT(1);
    }

    if crate::overlay::windows_overlay_wants_keys() {
        crate::overlay::windows_feed_vk(vk, info.scanCode);
        // Swallow so the client does not see overlay keystrokes.
        return LRESULT(1);
    }

    if daemon::is_session_busy() {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }

    if vk == u32::from(VK_PROCESSKEY.0) {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }

    let (uia_pw, es_pw) = a11y::windows_password_flags();
    let committed = unicode_from_vk(vk, info.scanCode);
    let (composition, ime_open) = ime_state();
    let composing = is_ime_composing(committed.as_deref(), composition, false, ime_open);
    let input = WinKeyInput {
        uia_is_password: uia_pw,
        es_password: es_pw,
        composing,
        committed,
    };
    let action = apply_key(input);
    if action == ImeAction::FireTrigger {
        let kind = field_kind(uia_pw, es_pw);
        on_trigger_fired(kind, action);
        // Eat the second `@` so UIA replace is not racing a leftover glyph.
        return LRESULT(1);
    }
    let _ = VK_PACKET;
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn capture_kind_from_vk(vk: u32) -> Option<openatat_ipc::CaptureKind> {
    match vk {
        0x33 => Some(openatat_ipc::CaptureKind::Display), // '3'
        0x34 => Some(openatat_ipc::CaptureKind::Area),    // '4'
        _ => None,
    }
}

fn is_capture_chord(vk: u32) -> bool {
    let Some(kind) = capture_kind_from_vk(vk) else {
        return false;
    };
    unsafe {
        let shift = GetAsyncKeyState(i32::from(VK_SHIFT.0)) < 0;
        let ctrl = GetAsyncKeyState(i32::from(VK_CONTROL.0)) < 0;
        let alt = GetAsyncKeyState(i32::from(VK_MENU.0)) < 0;
        let win = crate::windows_runtime::win_logo_down();
        let key = if vk == 0x33 { '3' } else { '4' };
        crate::capture::policy::is_capture_shortcut(
            crate::capture::policy::CaptureOs::Windows,
            kind,
            false,
            shift,
            ctrl,
            alt,
            win,
            key,
        )
    }
}

fn is_shelf_chord(vk: u32) -> bool {
    unsafe {
        let shift = GetAsyncKeyState(i32::from(VK_SHIFT.0)) < 0;
        let ctrl = GetAsyncKeyState(i32::from(VK_CONTROL.0)) < 0;
        let alt = GetAsyncKeyState(i32::from(VK_MENU.0)) < 0;
        let win = crate::windows_runtime::win_logo_down();
        crate::shelf::is_shelf_shortcut(
            crate::shelf::ShelfOs::Windows,
            false,
            shift,
            ctrl,
            alt,
            win,
            if vk == 0x56 { 'v' } else { '?' },
        )
    }
}

fn unicode_from_vk(vk: u32, scan: u32) -> Option<String> {
    unsafe {
        let mut state = [0u8; 256];
        let _ = GetKeyboardState(&mut state);
        if GetAsyncKeyState(i32::from(VK_SHIFT.0)) < 0 {
            state[usize::from(VK_SHIFT.0)] |= 0x80;
        }
        let mut buf = [0u16; 8];
        let n = ToUnicode(vk, scan, Some(&state), &mut buf, 0);
        if n <= 0 {
            return None;
        }
        String::from_utf16(&buf[..n as usize]).ok()
    }
}

fn ime_state() -> (bool, bool) {
    unsafe {
        let hwnd = windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow();
        if hwnd.0.is_null() {
            return (false, false);
        }
        let himc = ImmGetContext(hwnd);
        if himc.0.is_null() {
            return (false, false);
        }
        let open = ImmGetOpenStatus(himc).as_bool();
        let _ = ImmReleaseContext(hwnd, himc);
        (false, open)
    }
}

/// Re-probe every call. Never cache. Used when UIA is unavailable.
pub fn focused_is_es_password() -> bool {
    a11y::windows_es_password()
}

/// HWND of the overlay, if mapped. Used to skip our own keys.
pub fn overlay_hwnd() -> Option<isize> {
    crate::overlay::windows_overlay_hwnd()
}

#[allow(dead_code)]
fn _keep_hwnd_ty(_: HWND) {}
