//! `WS_EX_NOACTIVATE` region picker. Full-monitor dim + rubber-band.
//!
//! Never call `SetForegroundWindow`. Esc arrives from the process-local
//! hook. Overlay HWND is excluded from the later still
//! (`WDA_EXCLUDEFROMCAPTURE`).

use std::mem::size_of;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Mutex;

use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, SetDIBitsToDevice, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    HDC, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos,
    LoadCursorW, PeekMessageW, PostMessageW, RegisterClassExW, SetLayeredWindowAttributes,
    SetWindowDisplayAffinity, SetWindowPos, ShowWindow, TranslateMessage, CS_HREDRAW, CS_VREDRAW,
    HMENU, IDC_CROSS, LWA_ALPHA, MA_NOACTIVATE, MSG, PM_REMOVE, SWP_NOACTIVATE, SW_SHOWNOACTIVATE,
    WDA_EXCLUDEFROMCAPTURE, WM_DESTROY, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE,
    WM_MOUSEMOVE, WM_PAINT, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

use super::draw::{self, Rect};
use super::PickedRegion;
use crate::error::{Error, Result};

const WM_OPENATAT_KEY: u32 = 0x8000 + 42;
const CLASS: windows::core::PCWSTR = w!("OpenAtatPicker");

static PICKER_HWND: AtomicIsize = AtomicIsize::new(0);

struct LivePicker {
    press: Option<(i32, i32)>,
    current: Option<(i32, i32)>,
    width: u32,
    height: u32,
    origin_x: i32,
    origin_y: i32,
    end: Option<PickerEnd>,
}

#[derive(Clone)]
enum PickerEnd {
    Cancel,
    Region(PickedRegion),
}

static LIVE: Mutex<Option<LivePicker>> = Mutex::new(None);

pub fn picker_wants_keys() -> bool {
    PICKER_HWND.load(Ordering::SeqCst) != 0
}

pub fn feed_vk(vk: u32) {
    if let Some(hwnd) = picker_hwnd() {
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

fn picker_hwnd() -> Option<isize> {
    let v = PICKER_HWND.load(Ordering::SeqCst);
    (v != 0).then_some(v)
}

pub fn pick_region() -> Result<Option<PickedRegion>> {
    crate::windows_runtime::ensure_com();
    let (ox, oy, w, h) = monitor_under_cursor();
    let hwnd = create_picker(ox, oy, w, h)?;
    PICKER_HWND.store(hwnd.0 as isize, Ordering::SeqCst);
    *LIVE.lock().unwrap_or_else(|e| e.into_inner()) = Some(LivePicker {
        press: None,
        current: None,
        width: w as u32,
        height: h as u32,
        origin_x: ox,
        origin_y: oy,
        end: None,
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
            g.as_ref().and_then(|l| l.end.clone()).is_some()
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
                windows::Win32::System::Threading::Sleep(10);
            }
        }
    }

    let end = {
        let mut g = LIVE.lock().unwrap_or_else(|e| e.into_inner());
        g.take().and_then(|l| l.end)
    };
    PICKER_HWND.store(0, Ordering::SeqCst);
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
    match end {
        Some(PickerEnd::Region(r)) => Ok(Some(r)),
        _ => Ok(None),
    }
}

fn create_picker(x: i32, y: i32, w: i32, h: i32) -> Result<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None).map_err(|e| Error::msg(format!("GetModuleHandleW: {e}")))?;
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_CROSS).unwrap_or_default(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        let _ = RegisterClassExW(&class);
        let ex = WS_EX_NOACTIVATE | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED;
        let hwnd = CreateWindowExW(
            ex,
            CLASS,
            w!("OpenAtat"),
            WS_POPUP,
            x,
            y,
            w,
            h,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .map_err(|e| Error::msg(format!("CreateWindowExW: {e}")))?;
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 220, LWA_ALPHA);
        let _ = SetWindowPos(
            hwnd,
            Some(windows::Win32::UI::WindowsAndMessaging::HWND_TOPMOST),
            x,
            y,
            w,
            h,
            SWP_NOACTIVATE,
        );
        Ok(hwnd)
    }
}

fn monitor_under_cursor() -> (i32, i32, i32, i32) {
    unsafe {
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let mon = windows::Win32::Graphics::Gdi::MonitorFromPoint(
            pt,
            windows::Win32::Graphics::Gdi::MONITOR_DEFAULTTONEAREST,
        );
        let mut info = windows::Win32::Graphics::Gdi::MONITORINFO {
            cbSize: size_of::<windows::Win32::Graphics::Gdi::MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = windows::Win32::Graphics::Gdi::GetMonitorInfoW(mon, &mut info);
        let r = info.rcMonitor;
        (r.left, r.top, r.right - r.left, r.bottom - r.top)
    }
}

fn exclude_from_capture(hwnd: HWND) {
    unsafe {
        let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
    }
}

fn paint_now(hwnd: HWND) {
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
        let _ = windows::Win32::Graphics::Gdi::UpdateWindow(hwnd);
    }
}

fn selection_of(live: &LivePicker) -> Option<Rect> {
    let (x0, y0) = live.press?;
    let (x1, y1) = live.current?;
    let (x, y, w, h) = super::super::policy::normalize_rect(x0, y0, x1, y1)?;
    Some(Rect { x, y, w, h })
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_PAINT => {
            paint_hwnd(hwnd);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = (lparam.0 as u32 & 0xffff) as i32;
            let y = ((lparam.0 as u32 >> 16) & 0xffff) as i32;
            with_live(|live| {
                live.press = Some((x, y));
                live.current = Some((x, y));
            });
            paint_now(hwnd);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let x = (lparam.0 as u32 & 0xffff) as i32;
            let y = ((lparam.0 as u32 >> 16) & 0xffff) as i32;
            let dragging = {
                let g = LIVE.lock().unwrap_or_else(|e| e.into_inner());
                g.as_ref().and_then(|l| l.press).is_some()
            };
            if dragging {
                with_live(|live| live.current = Some((x, y)));
                paint_now(hwnd);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let x = (lparam.0 as u32 & 0xffff) as i32;
            let y = ((lparam.0 as u32 >> 16) & 0xffff) as i32;
            with_live(|live| {
                live.current = Some((x, y));
                live.end = match selection_of(live) {
                    Some(r) => Some(PickerEnd::Region(PickedRegion {
                        // Surface-local. WGC still is that monitor; CPU crop.
                        x: r.x,
                        y: r.y,
                        width: r.w,
                        height: r.h,
                        output: Some("foreground".into()),
                        surface_w: live.width,
                        surface_h: live.height,
                    })),
                    None => Some(PickerEnd::Cancel),
                };
            });
            LRESULT(0)
        }
        m if m == WM_OPENATAT_KEY => {
            let vk = wparam.0 as u32;
            if vk == u32::from(VK_ESCAPE.0) {
                with_live(|live| live.end = Some(PickerEnd::Cancel));
            }
            LRESULT(0)
        }
        WM_DESTROY => LRESULT(0),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn with_live(f: impl FnOnce(&mut LivePicker)) {
    if let Ok(mut g) = LIVE.lock() {
        if let Some(live) = g.as_mut() {
            f(live);
        }
    }
}

fn paint_hwnd(hwnd: HWND) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        let pixels = {
            let g = LIVE.lock().unwrap_or_else(|e| e.into_inner());
            g.as_ref().map(|live| {
                (
                    live.width,
                    live.height,
                    draw::render(live.width, live.height, selection_of(live)),
                )
            })
        };
        if let Some((w, h, bgra)) = pixels {
            blit_bgra(hdc, w, h, &bgra);
        }
        let _ = EndPaint(hwnd, &ps);
        let _ = (HMENU::default(), RECT::default());
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
        let _ = SetDIBitsToDevice(
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
