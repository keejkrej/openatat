//! Real UI Automation path. Re-probe `IsPassword` / `ES_PASSWORD` every call.

use std::sync::mpsc::Sender;

use windows::core::Interface;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER};
use windows::Win32::System::Com::SAFEARRAY;
use windows::Win32::System::Ole::SafeArrayDestroy;
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern,
    IUIAutomationTextRange, IUIAutomationValuePattern, UIA_DocumentControlTypeId,
    UIA_EditControlTypeId, UIA_TextPatternId, UIA_ValuePatternId,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_CONTROL, VK_V,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetParent, GetWindowLongW, GetWindowTextW,
    GetWindowThreadProcessId, GWL_STYLE,
};

const ES_PASSWORD: i32 = 0x0020;
const UIA_COMBOBOX: i32 = 50003;
const UIA_TEXT: i32 = 50020;
const UIA_PANE: i32 = 50033;

use super::windows_policy::{
    encode_win_identity, interpret_uia_field, interpret_uia_selection, is_browser_process,
    UiaControl, UiaFieldSnap,
};
use super::{MouseUpHit, Rect, SelectionProbe};
use crate::error::{Error, Result};
use crate::trigger::windows_policy::{strip_trailing_trigger, strip_trigger_before_caret};
use crate::trigger::FieldKind;

fn automation() -> Result<IUIAutomation> {
    unsafe {
        CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| Error::msg(format!("CUIAutomation: {e}")))
    }
}

fn focused_element() -> Result<IUIAutomationElement> {
    let auto = automation()?;
    unsafe {
        auto.GetFocusedElement()
            .map_err(|e| Error::msg(format!("UIA GetFocusedElement: {e}")))
    }
}

fn is_password_of(el: &IUIAutomationElement) -> bool {
    unsafe { el.CurrentIsPassword().map(|b| b.as_bool()).unwrap_or(false) }
}

fn control_of(el: &IUIAutomationElement) -> UiaControl {
    let id = unsafe { el.CurrentControlType().map(|c| c.0).unwrap_or(0) };
    if id == UIA_EditControlTypeId.0 {
        UiaControl::Edit
    } else if id == UIA_DocumentControlTypeId.0 {
        UiaControl::Document
    } else if id == UIA_COMBOBOX {
        UiaControl::ComboBox
    } else if id == UIA_TEXT {
        UiaControl::Text
    } else if id == UIA_PANE {
        UiaControl::Pane
    } else {
        UiaControl::Other
    }
}

fn class_of(el: &IUIAutomationElement) -> String {
    let from_uia = unsafe {
        el.CurrentClassName()
            .ok()
            .map(|s| s.to_string())
            .unwrap_or_default()
    };
    if !from_uia.is_empty() {
        return from_uia;
    }
    hwnd_class(foreground_hwnd())
}

fn snap_of(el: &IUIAutomationElement) -> UiaFieldSnap {
    let class_name = class_of(el);
    UiaFieldSnap {
        is_password: is_password_of(el),
        es_password: windows_es_password(),
        control_type: control_of(el),
        has_value_pattern: value_pattern(el).is_some(),
        has_text_pattern: text_pattern(el).is_some(),
        class_name,
    }
}

fn value_pattern(el: &IUIAutomationElement) -> Option<IUIAutomationValuePattern> {
    unsafe {
        let unk = el.GetCurrentPattern(UIA_ValuePatternId).ok()?;
        unk.cast().ok()
    }
}

fn text_pattern(el: &IUIAutomationElement) -> Option<IUIAutomationTextPattern> {
    unsafe {
        let unk = el.GetCurrentPattern(UIA_TextPatternId).ok()?;
        unk.cast().ok()
    }
}

pub fn probe_field_kind() -> FieldKind {
    match focused_element() {
        Ok(el) => interpret_uia_field(&snap_of(&el)),
        Err(_) => {
            if windows_es_password() {
                FieldKind::Secure
            } else {
                FieldKind::Other
            }
        }
    }
}

pub fn probe_selection() -> SelectionProbe {
    if windows_password_flags() != (false, false) {
        return SelectionProbe::Secure;
    }
    let Ok(el) = focused_element() else {
        return SelectionProbe::Unavailable;
    };
    let snap = snap_of(&el);
    if snap.is_password || snap.es_password {
        return SelectionProbe::Secure;
    }
    let (selected, range, bounds) = text_selection(&el);
    interpret_uia_selection(&snap, selected.as_deref(), range, bounds)
}

fn text_selection(el: &IUIAutomationElement) -> (Option<String>, Option<(i32, i32)>, Option<Rect>) {
    let Some(tp) = text_pattern(el) else {
        return (None, None, None);
    };
    unsafe {
        let Ok(ranges) = tp.GetSelection() else {
            return (None, None, None);
        };
        let n = ranges.Length().unwrap_or(0);
        if n <= 0 {
            return (Some(String::new()), Some((0, 0)), None);
        }
        let Ok(range) = ranges.GetElement(0) else {
            return (None, None, None);
        };
        let text = range.GetText(-1).ok().map(|s| s.to_string());
        let bounds = range_bounds(&range);
        // Offsets are not always exposed; policy treats missing range as
        // 0..chars so a non-empty TextPattern selection still summons.
        (text, None, bounds)
    }
}

fn range_bounds(range: &IUIAutomationTextRange) -> Option<Rect> {
    unsafe {
        let arr = range.GetBoundingRectangles().ok()?;
        let auto = automation().ok()?;
        let mut ptr: *mut RECT = std::ptr::null_mut();
        let n = auto.SafeArrayToRectNativeArray(arr, &mut ptr).ok()?;
        let rect = if !ptr.is_null() && n > 0 {
            let r = *ptr;
            Some(Rect {
                x: r.left,
                y: r.top,
                width: (r.right - r.left).max(0),
                height: (r.bottom - r.top).max(0),
            })
        } else {
            None
        };
        if !ptr.is_null() {
            CoTaskMemFree(Some(ptr.cast()));
        }
        let _ = SafeArrayDestroy(arr);
        rect
    }
}

pub fn focused_identity() -> Option<(isize, String)> {
    let hwnd = foreground_hwnd().0 as isize;
    if hwnd == 0 {
        return None;
    }
    let rid = focused_element()
        .ok()
        .and_then(|el| runtime_id(&el))
        .unwrap_or_default();
    Some((hwnd, encode_win_identity(hwnd, &rid)))
}

fn runtime_id(el: &IUIAutomationElement) -> Option<Vec<i32>> {
    unsafe {
        let arr = el.GetRuntimeId().ok()?;
        let auto = automation().ok()?;
        let mut ptr: *mut i32 = std::ptr::null_mut();
        let n = auto.IntSafeArrayToNativeArray(arr, &mut ptr).ok()?;
        let out = if !ptr.is_null() && n > 0 {
            Some(std::slice::from_raw_parts(ptr, n as usize).to_vec())
        } else {
            None
        };
        if !ptr.is_null() {
            CoTaskMemFree(Some(ptr.cast()));
        }
        let _ = SafeArrayDestroy(arr);
        let _ = arr as *mut SAFEARRAY;
        out
    }
}

pub fn windows_password_flags() -> (bool, bool) {
    let uia = focused_element()
        .ok()
        .map(|el| is_password_of(&el))
        .unwrap_or(false);
    (uia, windows_es_password())
}

pub fn windows_es_password() -> bool {
    let hwnd = focused_edit_hwnd();
    if hwnd.0.is_null() {
        return false;
    }
    unsafe {
        let style = GetWindowLongW(hwnd, GWL_STYLE);
        style & ES_PASSWORD != 0
    }
}

fn focused_edit_hwnd() -> HWND {
    unsafe {
        let focus = GetFocus();
        if !focus.0.is_null() {
            return focus;
        }
        GetForegroundWindow()
    }
}

fn foreground_hwnd() -> HWND {
    unsafe { GetForegroundWindow() }
}

fn hwnd_class(hwnd: HWND) -> String {
    if hwnd.0.is_null() {
        return String::new();
    }
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    if n <= 0 {
        String::new()
    } else {
        String::from_utf16_lossy(&buf[..n as usize])
    }
}

pub fn foreground_exe() -> Option<String> {
    let hwnd = foreground_hwnd();
    if hwnd.0.is_null() {
        return None;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == 0 {
        return None;
    }
    query_process_image(pid)
}

fn query_process_image(pid: u32) -> Option<String> {
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = windows::Win32::Foundation::CloseHandle(handle);
        if ok.is_err() {
            return None;
        }
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }
}

pub fn windows_swallow_trigger() -> Result<()> {
    if probe_field_kind() == FieldKind::Secure {
        return Err(Error::msg("refuse to swallow @@ in a password field"));
    }
    let el = focused_element()?;
    if let Some(vp) = value_pattern(&el) {
        let current = unsafe { vp.CurrentValue().ok().map(|s| s.to_string()) }.unwrap_or_default();
        if let Some(stripped) = strip_trailing_trigger(&current)
            .or_else(|| strip_trigger_before_caret(&current, current.chars().count()))
        {
            unsafe {
                vp.SetValue(&windows::core::BSTR::from(stripped.as_str()))
                    .map_err(|e| Error::msg(format!("UIA ValuePattern SetValue: {e}")))?;
            }
            return Ok(());
        }
        return Err(Error::msg("focused value has no trailing @@"));
    }
    Err(Error::msg("UIA ValuePattern unavailable; overlay still opens"))
}

pub fn insert_into_focused_field(text: &str) -> Result<bool> {
    write_or_paste(text, None)
}

pub fn replace_range(text: &str, start: i32, end: i32) -> Result<bool> {
    write_or_paste(text, Some((start, end)))
}

fn write_or_paste(text: &str, range: Option<(i32, i32)>) -> Result<bool> {
    if probe_field_kind() == FieldKind::Secure {
        return Ok(false);
    }
    let exe = foreground_exe().unwrap_or_default();
    let class = hwnd_class(foreground_hwnd());
    if is_browser_process(&exe) || is_browser_process(&class) {
        return paste_ctrl_v();
    }
    let Ok(el) = focused_element() else {
        return paste_ctrl_v();
    };
    if let Some(vp) = value_pattern(&el) {
        let current = unsafe { vp.CurrentValue().ok().map(|s| s.to_string()) }.unwrap_or_default();
        let new_value = if let Some((start, end)) = range {
            replace_utf8_range(&current, start, end, text)
        } else {
            format!("{current}{text}")
        };
        if unsafe { vp.SetValue(&windows::core::BSTR::from(new_value.as_str())) }.is_ok() {
            return Ok(true);
        }
    }
    let _ = range;
    paste_ctrl_v()
}

fn replace_utf8_range(current: &str, start: i32, end: i32, text: &str) -> String {
    let chars: Vec<char> = current.chars().collect();
    let s = (start.max(0) as usize).min(chars.len());
    let e = (end.max(0) as usize).min(chars.len()).max(s);
    let mut out: String = chars[..s].iter().collect();
    out.push_str(text);
    out.extend(chars[e..].iter());
    out
}

fn paste_ctrl_v() -> Result<bool> {
    unsafe {
        let mut inputs = [
            key_input(VK_CONTROL, false),
            key_input(VK_V, false),
            key_input(VK_V, true),
            key_input(VK_CONTROL, true),
        ];
        let n = SendInput(&mut inputs, std::mem::size_of::<INPUT>() as i32);
        if n == 0 {
            return Err(Error::msg("SendInput Ctrl+V failed"));
        }
    }
    Ok(true)
}

fn key_input(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Mouse-up only. Shift+arrow never arrives here.
pub fn spawn_mouse_up_watcher(tx: Sender<MouseUpHit>) {
    crate::windows_runtime::install_mouse_up(tx);
}

pub fn foreground_class() -> String {
    hwnd_class(foreground_hwnd())
}

pub fn foreground_title() -> Option<String> {
    let hwnd = foreground_hwnd();
    if hwnd.0.is_null() {
        return None;
    }
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if n <= 0 {
        None
    } else {
        Some(String::from_utf16_lossy(&buf[..n as usize]))
    }
}

pub fn explorer_root_hwnd() -> Option<isize> {
    let mut hwnd = foreground_hwnd();
    if hwnd.0.is_null() {
        return None;
    }
    unsafe {
        loop {
            let Ok(parent) = GetParent(hwnd) else {
                break;
            };
            if parent.0.is_null() {
                break;
            }
            hwnd = parent;
        }
    }
    Some(hwnd.0 as isize)
}
