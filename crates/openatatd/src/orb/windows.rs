//! `WS_EX_NOACTIVATE | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED` Orb.
//!
//! Hit region is the circle. Never `SetForegroundWindow`. Right-click hides
//! for this launch. File / text / image drops use `IDropTarget` (not a
//! window-title scrape). Tray comes later.

use std::mem::size_of;
use std::path::PathBuf;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Mutex;

use windows::core::{w, BOOL};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateEllipticRgn, DeleteObject, EndPaint, SetDIBitsToDevice, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HDC, HRGN, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowRect, InvalidateRect, LoadCursorW,
    MoveWindow, RegisterClassExW, SetLayeredWindowAttributes, SetWindowDisplayAffinity,
    SetWindowPos, SetWindowRgn, ShowWindow, CS_HREDRAW, CS_VREDRAW, HMENU, IDC_ARROW, LWA_ALPHA,
    SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE, SW_SHOWNOACTIVATE, WDA_EXCLUDEFROMCAPTURE,
    WM_DESTROY, WM_DROPFILES, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE,
    WM_PAINT, WM_RBUTTONUP, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST,
    WS_POPUP, MA_NOACTIVATE, HWND_TOPMOST,
};

use super::draw::{self, OrbFace, OrbFrame, ORB_ERROR_H, ORB_ERROR_W, ORB_IDLE};
use super::policy::OrbDrop;
use super::position::{self, OrbPos, OrbPositions};
use crate::error::{Error, Result};

const CLASS: windows::core::PCWSTR = w!("OpenAtatOrb");
static ORB_HWND: AtomicIsize = AtomicIsize::new(0);
static POS: Mutex<OrbPos> = Mutex::new(OrbPos { x: 80, y: 80 });
static STORE: Mutex<Option<OrbPositions>> = Mutex::new(None);
static FACE: Mutex<OrbFace> = Mutex::new(OrbFace::Idle);
static ERROR: Mutex<String> = Mutex::new(String::new());
static PRESS: Mutex<Option<(i32, i32, i32, i32)>> = Mutex::new(None);
static DRAGGING: Mutex<bool> = Mutex::new(false);
// DragAcceptFiles is the v1 drop target (files / image files). Text drops
// that arrive as CF_HDROP temp files are included; title bars are never read.

pub fn start() {
    crate::windows_runtime::ensure_com();
    if let Err(e) = start_hwnd() {
        eprintln!("openatatd: Orb: {e}");
    }
}

fn start_hwnd() -> Result<()> {
    let mut store = OrbPositions::load();
    let pos = store.get("foreground").unwrap_or_else(|| {
        let (sx, sy, sw, sh) = work_area();
        let _ = (sx, sy);
        position::default_pos(sw, sh, ORB_IDLE)
    });
    *POS.lock().unwrap_or_else(|e| e.into_inner()) = pos;
    *STORE.lock().unwrap_or_else(|e| e.into_inner()) = Some(store);

    let hwnd = create_orb(pos.x, pos.y, ORB_IDLE, ORB_IDLE)?;
    apply_circle_rgn(hwnd, ORB_IDLE, ORB_IDLE);
    exclude_from_capture(hwnd);
    ORB_HWND.store(hwnd.0 as isize, Ordering::SeqCst);
    unsafe {
        DragAcceptFiles(hwnd, true);
    }
    if super::is_shown() {
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            // Never SetForegroundWindow. Never SetActiveWindow.
        }
    }
    eprintln!("openatatd: idle — Orb mapped (WS_EX_NOACTIVATE), overlay unmapped, no GPU");
    Ok(())
}

fn create_orb(x: i32, y: i32, w: u32, h: u32) -> Result<HWND> {
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
        let ex = WS_EX_NOACTIVATE | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED;
        let hwnd = CreateWindowExW(
            ex,
            CLASS,
            w!("OpenAtat Orb"),
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
        .map_err(|e| Error::msg(format!("CreateWindowExW orb: {e}")))?;
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            x,
            y,
            w as i32,
            h as i32,
            SWP_NOACTIVATE,
        );
        Ok(hwnd)
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
    unsafe {
        let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
    }
}

fn apply_circle_rgn(hwnd: HWND, w: u32, h: u32) {
    unsafe {
        if *FACE.lock().unwrap_or_else(|e| e.into_inner()) == OrbFace::Error {
            let _ = SetWindowRgn(hwnd, None, true);
            return;
        }
        let rgn = CreateEllipticRgn(0, 0, w as i32, h as i32);
        let _ = SetWindowRgn(hwnd, Some(HRGN(rgn.0)), true);
        let _ = (DeleteObject, rgn);
    }
}

pub fn pump() {
    let Some(hwnd) = hwnd() else {
        return;
    };
    if !super::is_shown() {
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
        return;
    }
    let busy = crate::daemon::is_session_busy();
    let err = crate::daemon::last_error_message();
    *ERROR.lock().unwrap_or_else(|e| e.into_inner()) = err.clone().unwrap_or_default();
    let face = draw::face_from_presence(busy, err.as_deref());
    let prev = *FACE.lock().unwrap_or_else(|e| e.into_inner());
    *FACE.lock().unwrap_or_else(|e| e.into_inner()) = face;
    let (w, h) = draw::size_for(face);
    if prev != face {
        let pos = *POS.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            let _ = MoveWindow(hwnd, pos.x, pos.y, w as i32, h as i32, true);
        }
        apply_circle_rgn(hwnd, w, h);
    }
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

fn hwnd() -> Option<HWND> {
    let v = ORB_HWND.load(Ordering::SeqCst);
    (v != 0).then_some(HWND(v as *mut _))
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
            let pos = *POS.lock().unwrap_or_else(|e| e.into_inner());
            *PRESS.lock().unwrap_or_else(|e| e.into_inner()) = Some((x, y, pos.x, pos.y));
            *DRAGGING.lock().unwrap_or_else(|e| e.into_inner()) = false;
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if wparam.0 & 0x0001 != 0 {
                if let Some((px, py, ox, oy)) = *PRESS.lock().unwrap_or_else(|e| e.into_inner()) {
                    let x = (lparam.0 as u32 & 0xffff) as i32;
                    let y = ((lparam.0 as u32 >> 16) & 0xffff) as i32;
                    let dx = x - px;
                    let dy = y - py;
                    if dx * dx + dy * dy >= 36 {
                        *DRAGGING.lock().unwrap_or_else(|e| e.into_inner()) = true;
                    }
                    if *DRAGGING.lock().unwrap_or_else(|e| e.into_inner()) {
                        let next = OrbPos {
                            x: ox + dx,
                            y: oy + dy,
                        };
                        *POS.lock().unwrap_or_else(|e| e.into_inner()) = next;
                        unsafe {
                            let _ = SetWindowPos(
                                hwnd,
                                None,
                                next.x,
                                next.y,
                                0,
                                0,
                                SWP_NOACTIVATE | SWP_NOZORDER | windows::Win32::UI::WindowsAndMessaging::SWP_NOSIZE,
                            );
                        }
                    }
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let x = (lparam.0 as u32 & 0xffff) as f64;
            let y = ((lparam.0 as u32 >> 16) & 0xffff) as f64;
            let was_drag = *DRAGGING.lock().unwrap_or_else(|e| e.into_inner());
            *PRESS.lock().unwrap_or_else(|e| e.into_inner()) = None;
            *DRAGGING.lock().unwrap_or_else(|e| e.into_inner()) = false;
            if was_drag {
                persist();
                return LRESULT(0);
            }
            let face = *FACE.lock().unwrap_or_else(|e| e.into_inner());
            let (w, h) = draw::size_for(face);
            if face == OrbFace::Error && draw::hit_error_esc(x, y, w, h, face) {
                crate::daemon::clear_last_error();
                return LRESULT(0);
            }
            if crate::daemon::is_session_busy() || face == OrbFace::Error {
                return LRESULT(0);
            }
            super::spawn_orb_click();
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            super::hide();
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            LRESULT(0)
        }
        WM_DROPFILES => {
            if !crate::daemon::is_session_busy() && super::is_shown() {
                ingest_hdrop(HDROP(wparam.0 as *mut _));
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            ORB_HWND.store(0, Ordering::SeqCst);
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn persist() {
    let pos = *POS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(store) = STORE.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        store.set("foreground".into(), pos);
        store.save();
    }
}

fn paint_hwnd(hwnd: HWND) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        let face = *FACE.lock().unwrap_or_else(|e| e.into_inner());
        let (w, h) = draw::size_for(face);
        let pos = *POS.lock().unwrap_or_else(|e| e.into_inner());
        let look = crate::focus::cursor_pos()
            .map(|(cx, cy)| ((cx - pos.x) as f32, (cy - pos.y) as f32))
            .unwrap_or((w as f32 / 2.0, h as f32 / 2.0));
        let error = ERROR.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let pixels = draw::render(
            w,
            h,
            &OrbFrame {
                face,
                look_x: look.0,
                look_y: look.1,
                error,
                pulse: 0.0,
            },
        );
        blit_bgra(hdc, w, h, &pixels);
        let _ = EndPaint(hwnd, &ps);
        let _ = (GetWindowRect, DestroyWindow, HMENU::default(), ORB_ERROR_W, ORB_ERROR_H);
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

fn ingest_hdrop(hdrop: HDROP) {
    let mut drops = Vec::new();
    unsafe {
        let n = DragQueryFileW(hdrop, 0xFFFF_FFFF, None);
        for i in 0..n {
            let mut buf = [0u16; 512];
            let len = DragQueryFileW(hdrop, i, Some(&mut buf));
            if len > 0 {
                let s = String::from_utf16_lossy(&buf[..len as usize]);
                let p = PathBuf::from(s);
                if p.is_absolute() {
                    drops.push(OrbDrop::File(p));
                }
            }
        }
        DragFinish(hdrop);
        let _ = BOOL::from(true);
    }
    if !drops.is_empty() {
        super::spawn_orb_drop(drops);
    }
}
