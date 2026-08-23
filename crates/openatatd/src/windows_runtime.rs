//! Win32 host: STA COM, message loop for hooks, optional Raw Input.
//!
//! OpenAtat never becomes the foreground app. Overlay windows are
//! `WS_EX_NOACTIVATE`. This module does not call `SetForegroundWindow`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::OnceLock;

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_SINGLETHREADED};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LWIN, VK_RWIN};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, PeekMessageW, SetWindowsHookExW,
    TranslateMessage, HC_ACTION, MSLLHOOKSTRUCT, PM_REMOVE, WH_MOUSE_LL, WM_LBUTTONUP,
    WM_QUIT,
};

use crate::a11y::{self, MouseUpHit};
use crate::error::Result;
use crate::trigger::{ImeBackend, WinHookBackend};

static COM_OK: AtomicBool = AtomicBool::new(false);
static MOUSE_HOOK: std::sync::Mutex<Option<isize>> = std::sync::Mutex::new(None);
static MOUSE_TX: OnceLock<Sender<MouseUpHit>> = OnceLock::new();

pub fn ensure_com() {
    if COM_OK.load(Ordering::SeqCst) {
        return;
    }
    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        // S_OK or S_FALSE (already initialized) are fine.
        if hr.is_ok() || hr == windows::Win32::Foundation::S_FALSE {
            COM_OK.store(true, Ordering::SeqCst);
            let _ = RoInitialize(RO_INIT_SINGLETHREADED);
        }
    }
}

pub fn win_logo_down() -> bool {
    unsafe { GetAsyncKeyState(i32::from(VK_LWIN.0)) < 0 || GetAsyncKeyState(i32::from(VK_RWIN.0)) < 0 }
}

pub fn install_mouse_up(tx: Sender<MouseUpHit>) {
    let _ = MOUSE_TX.set(tx);
    ensure_com();
    unsafe {
        match SetWindowsHookExW(WH_MOUSE_LL, Some(ll_mouse_proc), None, 0) {
            Ok(hook) if !hook.is_invalid() => {
                *MOUSE_HOOK.lock().unwrap_or_else(|e| e.into_inner()) = Some(hook.0 as isize);
                eprintln!("openatatd: mouse-up hook installed — C10 is UIA TextPattern");
            }
            _ => {
                eprintln!(
                    "openatatd: mouse-up hook failed; `openatatd selection` still works"
                );
            }
        }
    }
}

unsafe extern "system" fn ll_mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && wparam.0 as u32 == WM_LBUTTONUP {
        if !crate::daemon::is_session_busy() {
            if let Some(tx) = MOUSE_TX.get() {
                let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
                let pointer = Some((info.pt.x, info.pt.y));
                let probe = a11y::probe_selection();
                let _ = tx.send(MouseUpHit { probe, pointer });
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

pub fn run_daemon_host(start_socket: impl FnOnce() + Send + 'static) -> Result<()> {
    ensure_com();
    let mut hook = WinHookBackend::default();
    let _ = hook.start();
    crate::daemon::start_presence_and_selection();
    std::thread::Builder::new()
        .name("openatat-sock".into())
        .spawn(start_socket)
        .map_err(|e| crate::error::Error::msg(format!("socket thread: {e}")))?;

    eprintln!("openatatd: idle — WS_EX_NOACTIVATE overlay unmapped, no GPU window");

    let mut msg = windows::Win32::UI::WindowsAndMessaging::MSG::default();
    loop {
        unsafe {
            if !GetMessageW(&mut msg, None, 0, 0).as_bool() {
                if msg.message == WM_QUIT {
                    break;
                }
            }
            if msg.message == WM_QUIT {
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

/// Pump a few messages. Used by `--demo` when no daemon loop is running.
pub fn pump_once() {
    let mut msg = windows::Win32::UI::WindowsAndMessaging::MSG::default();
    unsafe {
        if PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
