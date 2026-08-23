//! `WS_EX_NOACTIVATE` stop bar. Elapsed + Stop. Esc from the process-local
//! hook. Drag empty background. Never `SetForegroundWindow`. HWND excluded
//! from WGC (`WDA_EXCLUDEFROMCAPTURE`).

use std::mem::size_of;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, SetDIBitsToDevice, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    HDC, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos, GetWindowRect,
    LoadCursorW, PeekMessageW, PostMessageW, RegisterClassExW, SetLayeredWindowAttributes,
    SetWindowDisplayAffinity, SetWindowPos, ShowWindow, TranslateMessage, CS_HREDRAW, CS_VREDRAW,
    HMENU, IDC_ARROW, LWA_ALPHA, MA_NOACTIVATE, MSG, PM_REMOVE, SWP_NOACTIVATE, SWP_NOZORDER,
    SW_SHOWNOACTIVATE, WDA_EXCLUDEFROMCAPTURE, WM_DESTROY, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_PAINT, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use super::bar::{self, BarHit, BAR_H, BAR_W};
use super::BarEnd;
use crate::error::{Error, Result};

const WM_OPENATAT_KEY: u32 = 0x8000 + 43;
const CLASS: windows::core::PCWSTR = w!("OpenAtatRecordBar");

static BAR_HWND: AtomicIsize = AtomicIsize::new(0);

struct LiveBar {
    started: Instant,
    last_secs: u64,
    drag: Option<(i32, i32, i32, i32)>,
    end: Option<BarEnd>,
}

static LIVE: Mutex<Option<LiveBar>> = Mutex::new(None);

pub fn bar_wants_keys() -> bool {
    BAR_HWND.load(Ordering::SeqCst) != 0
}

pub fn bar_hwnd() -> Option<isize> {
    let v = BAR_HWND.load(Ordering::SeqCst);
    (v != 0).then_some(v)
}

pub fn feed_vk(vk: u32) {
    if let Some(hwnd) = bar_hwnd() {
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

pub fn run() -> Result<BarEnd> {
    crate::windows_runtime::ensure_com();
    *LIVE.lock().unwrap_or_else(|e| e.into_inner()) = Some(LiveBar {
        started: Instant::now(),
        last_secs: 0,
        drag: None,
        end: None,
    });
    let hwnd = create_bar()?;
    BAR_HWND.store(hwnd.0 as isize, Ordering::SeqCst);
    unsafe {
        let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }

    let mut msg = MSG::default();
    loop {
        let ended = {
            let g = LIVE.lock().unwrap_or_else(|e| e.into_inner());
            g.as_ref().and_then(|b| b.end).is_some()
        };
        if ended {
            break;
        }
        unsafe {
            if PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            } else {
                windows::Win32::System::Threading::Sleep(50);
                maybe_repaint(hwnd);
            }
        }
    }

    let end = {
        let mut g = LIVE.lock().unwrap_or_else(|e| e.into_inner());
        g.take().and_then(|b| b.end).unwrap_or(BarEnd::Cancel)
    };
    BAR_HWND.store(0, Ordering::SeqCst);
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
    Ok(end)
}

fn maybe_repaint(hwnd: HWND) {
    let mut g = LIVE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(bar) = g.as_mut() else {
        return;
    };
    let secs = bar.started.elapsed().as_secs();
    if secs != bar.last_secs {
        bar.last_secs = secs;
        drop(g);
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::InvalidateRect(Some(hwnd), None, false);
        }
    }
}

fn create_bar() -> Result<HWND> {
    unsafe {
        let hinstance = GetModuleHandleW(None).map_err(|e| Error::msg(e.to_string()))?;
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        let _ = RegisterClassExW(&wc);
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let x = pt.x - (BAR_W as i32) / 2;
        let y = 16;
        let hwnd = CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
            CLASS,
            w!("OpenAtat Record"),
            WS_POPUP,
            x,
            y,
            BAR_W as i32,
            BAR_H as i32,
            None,
            HMENU::default(),
            Some(hinstance.into()),
            None,
        )
        .map_err(|e| Error::msg(e.to_string()))?;
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 245, LWA_ALPHA);
        let _ = SetWindowPos(
            hwnd,
            Some(windows::Win32::UI::WindowsAndMessaging::HWND_TOPMOST),
            x,
            y,
            BAR_W as i32,
            BAR_H as i32,
            SWP_NOACTIVATE | SWP_NOZORDER,
        );
        Ok(hwnd)
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = (lparam.0 as i32) & 0xffff;
            let y = (lparam.0 as i32) >> 16;
            match bar::hit(x as f64, y as f64) {
                BarHit::Stop => {
                    if let Some(b) = LIVE.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
                        b.end = Some(BarEnd::Stop);
                    }
                }
                BarHit::Drag => {
                    let mut rc = RECT::default();
                    unsafe {
                        let _ = GetWindowRect(hwnd, &mut rc);
                    }
                    if let Some(b) = LIVE.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
                        b.drag = Some((x, y, rc.left, rc.top));
                    }
                }
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let x = (lparam.0 as i32) & 0xffff;
            let y = (lparam.0 as i32) >> 16;
            let mut g = LIVE.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(b) = g.as_mut() {
                if let Some((px, py, ox, oy)) = b.drag {
                    let nx = ox + (x - px);
                    let ny = oy + (y - py);
                    drop(g);
                    unsafe {
                        let _ = SetWindowPos(
                            hwnd,
                            None,
                            nx,
                            ny,
                            BAR_W as i32,
                            BAR_H as i32,
                            SWP_NOACTIVATE | SWP_NOZORDER,
                        );
                    }
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            if let Some(b) = LIVE.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
                b.drag = None;
            }
            LRESULT(0)
        }
        WM_OPENATAT_KEY => {
            if wparam.0 as u32 == u32::from(VK_ESCAPE.0) {
                if let Some(b) = LIVE.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
                    b.end = Some(BarEnd::Cancel);
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => LRESULT(0),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn paint(hwnd: HWND) {
    let secs = LIVE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|b| b.started.elapsed().as_secs())
        .unwrap_or(0);
    let pixels = bar::render(BAR_W, BAR_H, secs);
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc: HDC = BeginPaint(hwnd, &mut ps);
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: BAR_W as i32,
                biHeight: -(BAR_H as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0 as u32,
                ..Default::default()
            },
            ..Default::default()
        };
        let _ = SetDIBitsToDevice(
            hdc,
            0,
            0,
            BAR_W,
            BAR_H,
            0,
            0,
            0,
            BAR_H,
            pixels.as_ptr() as *const _,
            &info,
            DIB_RGB_COLORS,
        );
        let _ = EndPaint(hwnd, &ps);
    }
}
