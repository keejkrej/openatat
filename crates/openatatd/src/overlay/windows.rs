//! `WS_EX_NOACTIVATE | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED`.
//!
//! Never call `SetForegroundWindow`. Keyboard arrives from the process-local
//! hook (or Raw Input with `RIDEV_INPUTSINK` when the hook is missing).
//! Session / Tab / R / handoff live in [`super::controller`].

use std::mem::size_of;
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use std::sync::Mutex;

use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, EndPaint,
    SelectObject, SetDIBits, StretchBlt, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    HBITMAP, HDC, PAINTSTRUCT, SRCCOPY,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VK_BACK, VK_ESCAPE, VK_OEM_2, VK_RETURN, VK_TAB,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
    GetCursorPos, GetMessageW, LoadCursorW, PeekMessageW, PostMessageW, RegisterClassExW,
    SetLayeredWindowAttributes, SetWindowDisplayAffinity, SetWindowPos, ShowWindow,
    TranslateMessage, CS_HREDRAW, CS_VREDRAW, HMENU, IDC_ARROW, LWA_ALPHA,
    MSG, PM_REMOVE, SWP_NOACTIVATE, SWP_NOZORDER, SW_SHOWNOACTIVATE, WDA_EXCLUDEFROMCAPTURE,
    WM_DESTROY, WM_LBUTTONDOWN, WM_MOUSEACTIVATE, WM_PAINT, WNDCLASSEXW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, MA_NOACTIVATE,
};

use super::controller::{OverlayController, OverlayEffect, OverlayKey};
use super::draw::{self, BAR_H, BAR_W, POPOVER_H, POPOVER_W};
use super::{OverlayEnd, OverlayKind};
use crate::error::{Error, Result};
use crate::session::Session;

const WM_OPENATAT_KEY: u32 = 0x8000 + 41; // WM_APP + 41
const CLASS: windows::core::PCWSTR = w!("OpenAtatOverlay");

static OVERLAY_HWND: AtomicIsize = AtomicIsize::new(0);
static OVERLAY_W: AtomicU32 = AtomicU32::new(POPOVER_W);
static OVERLAY_H: AtomicU32 = AtomicU32::new(POPOVER_H);

struct LiveOverlay {
    ctl: OverlayController,
    #[allow(dead_code)]
    hwnd: isize,
}

static LIVE: Mutex<Option<LiveOverlay>> = Mutex::new(None);

pub fn overlay_hwnd() -> Option<isize> {
    let v = OVERLAY_HWND.load(Ordering::SeqCst);
    (v != 0).then_some(v)
}

pub fn overlay_wants_keys() -> bool {
    overlay_hwnd().is_some()
}

pub fn feed_vk(vk: u32, _scan: u32) {
    if let Some(hwnd) = overlay_hwnd() {
        unsafe {
            let _ = PostMessageW(
                Some(HWND(hwnd as *mut _)),
                WM_OPENATAT_KEY,
                WPARAM(vk as usize),
                LPARAM(0),
            );
        }
    }
}

pub fn run(session: &mut Session, kind: OverlayKind) -> Result<OverlayEnd> {
    crate::windows_runtime::ensure_com();
    let ctl = OverlayController::from_session(session, kind);
    let (w, h) = (ctl.width, ctl.height);
    let hwnd = create_overlay(w, h, kind)?;
    OVERLAY_HWND.store(hwnd.0 as isize, Ordering::SeqCst);
    OVERLAY_W.store(w, Ordering::SeqCst);
    OVERLAY_H.store(h, Ordering::SeqCst);
    *LIVE.lock().unwrap_or_else(|e| e.into_inner()) = Some(LiveOverlay {
        ctl,
        hwnd: hwnd.0 as isize,
    });

    exclude_from_capture(hwnd);
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        // Never SetForegroundWindow. Never SetActiveWindow.
    }
    paint_now(hwnd);

    let mut msg = MSG::default();
    loop {
        let ended = {
            let g = LIVE.lock().unwrap_or_else(|e| e.into_inner());
            g.as_ref().and_then(|l| l.ctl.end).is_some()
        };
        if ended {
            break;
        }
        unsafe {
            if PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == windows::Win32::UI::WindowsAndMessaging::WM_QUIT {
                    break;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            } else {
                // Sleep a tick so --demo does not spin.
                windows::Win32::System::Threading::Sleep(10);
            }
        }
        apply_pending_resize();
    }

    let end = {
        let mut g = LIVE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(live) = g.take() {
            live.ctl.write_back(session);
            live.ctl.end.unwrap_or(OverlayEnd::Cancelled)
        } else {
            OverlayEnd::Cancelled
        }
    };
    OVERLAY_HWND.store(0, Ordering::SeqCst);
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
    Ok(end)
}

fn create_overlay(w: u32, h: u32, kind: OverlayKind) -> Result<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None).map_err(|e| Error::msg(format!("GetModuleHandleW: {e}")))?;
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        let _ = RegisterClassExW(&class);

        let (x, y) = place(w, h, kind);
        let ex = WS_EX_NOACTIVATE | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED;
        let hwnd = CreateWindowExW(
            ex,
            CLASS,
            w!("OpenAtat"),
            WS_POPUP,
            x,
            y,
            w as i32,
            h as i32,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .map_err(|e| Error::msg(format!("CreateWindowExW: {e}")))?;
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
        let _ = SetWindowPos(
            hwnd,
            Some(windows::Win32::UI::WindowsAndMessaging::HWND_TOPMOST),
            x,
            y,
            w as i32,
            h as i32,
            SWP_NOACTIVATE,
        );
        Ok(hwnd)
    }
}

fn place(w: u32, h: u32, kind: OverlayKind) -> (i32, i32) {
    let (sx, sy, sw, sh) = work_area();
    match kind {
        OverlayKind::Prompt => {
            let x = sx + ((sw - w as i32) / 2).max(24);
            let y = sy + 80;
            (x, y)
        }
        OverlayKind::SelectionBar => {
            let _ = (h, BAR_W, BAR_H, POPOVER_W, POPOVER_H);
            let mut pt = POINT::default();
            unsafe {
                let _ = GetCursorPos(&mut pt);
            }
            (pt.x.saturating_add(12), pt.y.saturating_add(16).min(sy + sh - 48))
        }
    }
}

fn work_area() -> (i32, i32, i32, i32) {
    unsafe {
        let mut rc = RECT::default();
        let _ = windows::Win32::UI::WindowsAndMessaging::SystemParametersInfoW(
            windows::Win32::UI::WindowsAndMessaging::SPI_GETWORKAREA,
            0,
            Some((&mut rc as *mut RECT).cast()),
            windows::Win32::UI::WindowsAndMessaging::SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
        (rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top)
    }
}

fn exclude_from_capture(hwnd: HWND) {
    // WDA_EXCLUDEFROMCAPTURE = 0x11 (Windows 10 2004+). Ignore failure on older builds.
    unsafe {
        let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
    }
}

fn apply_pending_resize() {
    let Some(hwnd) = overlay_hwnd().map(|v| HWND(v as *mut _)) else {
        return;
    };
    let (w, h) = {
        let g = LIVE.lock().unwrap_or_else(|e| e.into_inner());
        match g.as_ref() {
            Some(live) => (live.ctl.width, live.ctl.height),
            None => return,
        }
    };
    if OVERLAY_W.load(Ordering::SeqCst) == w && OVERLAY_H.load(Ordering::SeqCst) == h {
        return;
    }
    OVERLAY_W.store(w, Ordering::SeqCst);
    OVERLAY_H.store(h, Ordering::SeqCst);
    unsafe {
        let mut rc = RECT::default();
        let _ = windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut rc);
        let _ = SetWindowPos(
            hwnd,
            None,
            rc.left,
            rc.top,
            w as i32,
            h as i32,
            SWP_NOACTIVATE | SWP_NOZORDER,
        );
    }
    paint_now(hwnd);
}

fn paint_now(hwnd: HWND) {
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
        let _ = windows::Win32::Graphics::Gdi::UpdateWindow(hwnd);
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_PAINT => {
            paint_hwnd(hwnd);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = (lparam.0 as u32 & 0xffff) as f64;
            let y = ((lparam.0 as u32 >> 16) & 0xffff) as f64;
            with_live(|live| {
                let fx = live.ctl.handle_click(x, y);
                apply_effects(hwnd, &fx, &mut live.ctl);
            });
            LRESULT(0)
        }
        m if m == WM_OPENATAT_KEY => {
            let vk = wparam.0 as u32;
            with_live(|live| {
                let (key, text) = map_vk(vk);
                let fx = live.ctl.handle_key(key, text.as_deref());
                apply_effects(hwnd, &fx, &mut live.ctl);
            });
            LRESULT(0)
        }
        WM_DESTROY => LRESULT(0),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn with_live(f: impl FnOnce(&mut LiveOverlay)) {
    if let Ok(mut g) = LIVE.lock() {
        if let Some(live) = g.as_mut() {
            f(live);
        }
    }
}

fn apply_effects(hwnd: HWND, fx: &[OverlayEffect], ctl: &mut OverlayController) {
    for e in fx {
        match e {
            OverlayEffect::Redraw => {
                ctl.dirty = false;
                paint_now(hwnd);
            }
            OverlayEffect::Resize { w, h } => {
                ctl.width = *w;
                ctl.height = *h;
            }
        }
    }
    if ctl.dirty {
        ctl.dirty = false;
        paint_now(hwnd);
    }
}

fn map_vk(vk: u32) -> (OverlayKey, Option<String>) {
    if vk == u32::from(VK_ESCAPE.0) {
        return (OverlayKey::Escape, None);
    }
    if vk == u32::from(VK_RETURN.0) {
        let meta = crate::windows_runtime::win_logo_down();
        return (OverlayKey::Return { meta }, None);
    }
    if vk == u32::from(VK_TAB.0) {
        return (OverlayKey::Tab, None);
    }
    if vk == u32::from(VK_BACK.0) {
        return (OverlayKey::Backspace, None);
    }
    if vk == 0x52 {
        // R
        return (OverlayKey::R, Some("r".into()));
    }
    if (0x31..=0x35).contains(&vk) {
        return (OverlayKey::Digit((vk - 0x30) as u8), None);
    }
    if let Some(ch) = vk_to_text(vk) {
        return (OverlayKey::Text, Some(ch));
    }
    let _ = VK_OEM_2;
    (OverlayKey::Text, None)
}

fn vk_to_text(vk: u32) -> Option<String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyboardState, ToUnicode};
    unsafe {
        let mut state = [0u8; 256];
        let _ = GetKeyboardState(&mut state);
        let mut buf = [0u16; 8];
        let n = ToUnicode(vk, 0, Some(&state), &mut buf, 0);
        if n <= 0 {
            return None;
        }
        String::from_utf16(&buf[..n as usize]).ok()
    }
}

fn paint_hwnd(hwnd: HWND) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        let _ = hdc;
        let pixels = {
            let g = LIVE.lock().unwrap_or_else(|e| e.into_inner());
            g.as_ref().map(|live| {
                let thumb = live
                    .ctl
                    .thumb
                    .as_ref()
                    .map(|(w, h, p)| (*w, *h, p.as_slice()));
                (
                    live.ctl.width,
                    live.ctl.height,
                    draw::render(live.ctl.width, live.ctl.height, &live.ctl.ui_frame(), thumb),
                )
            })
        };
        if let Some((w, h, bgra)) = pixels {
            blit_bgra(hdc, w, h, &bgra);
        }
        let _ = EndPaint(hwnd, &ps);
        let _ = (
            GetClientRect,
            CreateCompatibleDC,
            CreateDIBSection,
            DeleteDC,
            DeleteObject,
            SelectObject,
            SetDIBits,
            StretchBlt,
            HBITMAP::default(),
            SRCCOPY,
            HMENU::default(),
        );
    }
}

fn blit_bgra(hdc: HDC, w: u32, h: u32, bgra: &[u8]) {
    unsafe {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                biHeight: -(h as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            bmiColors: [Default::default()],
        };
        let _ = windows::Win32::Graphics::Gdi::SetDIBitsToDevice(
            hdc,
            0,
            0,
            w,
            h,
            0,
            0,
            0,
            h,
            bgra.as_ptr().cast(),
            &info,
            DIB_RGB_COLORS,
        );
    }
}

#[allow(dead_code)]
fn _keep_getmessage(_: MSG) {
    let _ = GetMessageW;
}
