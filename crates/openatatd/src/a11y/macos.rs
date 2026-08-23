//! Real AXUIElement path. Re-probe secure input / AXSecureTextField every call.

use std::ffi::c_void;
use std::ptr;
use std::sync::mpsc::Sender;

use accessibility_sys::{
    kAXErrorSuccess, kAXFocusedUIElementAttribute, kAXIdentifierAttribute, kAXPositionAttribute,
    kAXRoleAttribute, kAXSelectedTextAttribute, kAXSelectedTextRangeAttribute, kAXSubroleAttribute,
    kAXValueAttribute, kAXValueTypeCFRange, kAXValueTypeCGPoint, AXUIElementCopyAttributeValue,
    AXUIElementCreateSystemWide, AXUIElementGetPid, AXUIElementRef, AXUIElementSetAttributeValue,
    AXValueGetValue, AXValueRef,
};
use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
use core_foundation::string::{CFString, CFStringRef};

use super::macos_policy::{
    encode_element_id, interpret_ax_field, interpret_ax_selection, is_browser_bundle, role_is_secure,
};
use super::{MouseUpHit, Rect, SelectionProbe};
use crate::error::{Error, Result};
use crate::trigger::macos_policy::{strip_trailing_trigger, strip_trigger_before_caret};
use crate::trigger::FieldKind;

struct AxElem {
    raw: AXUIElementRef,
    owned: bool,
}

impl AxElem {
    fn system_wide() -> Option<Self> {
        let raw = unsafe { AXUIElementCreateSystemWide() };
        if raw.is_null() {
            None
        } else {
            Some(Self { raw, owned: true })
        }
    }

    fn copy_attr(&self, name: &str) -> Option<CFTypeRef> {
        let key = CFString::new(name);
        let mut out: CFTypeRef = ptr::null();
        let err = unsafe {
            AXUIElementCopyAttributeValue(
                self.raw,
                key.as_concrete_TypeRef(),
                &mut out as *mut _ as *mut *const c_void,
            )
        };
        if err != kAXErrorSuccess || out.is_null() {
            None
        } else {
            Some(out)
        }
    }

    fn string_attr(&self, name: &str) -> Option<String> {
        let v = self.copy_attr(name)?;
        let s = unsafe { cf_string(v) };
        unsafe { CFRelease(v) };
        s
    }

    fn elem_attr(&self, name: &str) -> Option<AxElem> {
        let v = self.copy_attr(name)?;
        Some(AxElem {
            raw: v as AXUIElementRef,
            owned: true,
        })
    }

    fn pid(&self) -> Option<i32> {
        let mut pid = 0;
        let err = unsafe { AXUIElementGetPid(self.raw, &mut pid) };
        (err == kAXErrorSuccess).then_some(pid)
    }

    fn set_string(&self, name: &str, value: &str) -> bool {
        let key = CFString::new(name);
        let val = CFString::new(value);
        let err = unsafe {
            AXUIElementSetAttributeValue(self.raw, key.as_concrete_TypeRef(), val.as_CFTypeRef())
        };
        err == kAXErrorSuccess
    }
}

impl Drop for AxElem {
    fn drop(&mut self) {
        if self.owned && !self.raw.is_null() {
            unsafe { CFRelease(self.raw as CFTypeRef) };
        }
    }
}

unsafe fn cf_string(v: CFTypeRef) -> Option<String> {
    if v.is_null() {
        return None;
    }
    let s = CFString::wrap_under_get_rule(v as CFStringRef);
    Some(s.to_string())
}

fn focused_element() -> Option<AxElem> {
    let sys = AxElem::system_wide()?;
    sys.elem_attr(kAXFocusedUIElementAttribute)
}

fn role_of(elem: &AxElem) -> String {
    elem.string_attr(kAXRoleAttribute).unwrap_or_default()
}

fn subrole_of(elem: &AxElem) -> Option<String> {
    elem.string_attr(kAXSubroleAttribute)
}

pub fn probe_field_kind() -> FieldKind {
    let secure_input = crate::trigger::macos::is_secure_event_input();
    let ax = focused_element()
        .map(|el| interpret_ax_field(&role_of(&el), subrole_of(&el).as_deref()))
        .unwrap_or(FieldKind::Other);
    if secure_input || ax == FieldKind::Secure {
        FieldKind::Secure
    } else {
        ax
    }
}

pub fn probe_selection() -> SelectionProbe {
    if crate::trigger::macos::is_secure_event_input() {
        return SelectionProbe::Secure;
    }
    let Some(el) = focused_element() else {
        return SelectionProbe::Unavailable;
    };
    let role = role_of(&el);
    let sub = subrole_of(&el);
    if role_is_secure(&role, sub.as_deref()) {
        return SelectionProbe::Secure;
    }
    let selected = el.string_attr(kAXSelectedTextAttribute);
    let range = selected_range(&el);
    let bounds = selected_bounds(&el);
    interpret_ax_selection(&role, sub.as_deref(), selected.as_deref(), range, bounds)
}

fn selected_range(el: &AxElem) -> Option<(i32, i32)> {
    let v = el.copy_attr(kAXSelectedTextRangeAttribute)?;
    let range = cf_range(v as AXValueRef);
    unsafe { CFRelease(v) };
    range
}

#[repr(C)]
struct CfRange {
    location: isize,
    length: isize,
}

fn cf_range(value: AXValueRef) -> Option<(i32, i32)> {
    if value.is_null() {
        return None;
    }
    let mut range = CfRange {
        location: 0,
        length: 0,
    };
    let ok = unsafe {
        AXValueGetValue(
            value,
            kAXValueTypeCFRange,
            &mut range as *mut _ as *mut c_void,
        )
    };
    if ok {
        Some((range.location as i32, (range.location + range.length) as i32))
    } else {
        None
    }
}

fn selected_bounds(_el: &AxElem) -> Option<Rect> {
    None
}

pub fn focused_identity() -> Option<(i32, String)> {
    let el = focused_element()?;
    let pid = el.pid()?;
    let role = role_of(&el);
    let ident = el.string_attr(kAXIdentifierAttribute);
    let pos = ax_point(&el);
    Some((pid, encode_element_id(pid, &role, ident.as_deref(), pos)))
}

fn ax_point(el: &AxElem) -> Option<(i32, i32)> {
    let v = el.copy_attr(kAXPositionAttribute)?;
    #[repr(C)]
    struct CGPoint {
        x: f64,
        y: f64,
    }
    let mut p = CGPoint { x: 0.0, y: 0.0 };
    let ok = unsafe {
        AXValueGetValue(
            v as AXValueRef,
            kAXValueTypeCGPoint,
            &mut p as *mut _ as *mut c_void,
        )
    };
    unsafe { CFRelease(v) };
    ok.then_some((p.x as i32, p.y as i32))
}

pub fn macos_has_marked_text() -> bool {
    let Some(el) = focused_element() else {
        return false;
    };
    el.string_attr("AXMarkedText")
        .map(|s| !s.is_empty())
        .unwrap_or(false)
}

pub fn macos_swallow_trigger() -> Result<()> {
    if probe_field_kind() == FieldKind::Secure {
        return Err(Error::msg("refuse to swallow @@ in a secure field"));
    }
    let Some(el) = focused_element() else {
        return Err(Error::msg("no focused AX element to swallow @@"));
    };
    let Some(value) = el.string_attr(kAXValueAttribute) else {
        return Err(Error::msg("AXValue unavailable; overlay still opens"));
    };
    let caret = selected_range(&el).map(|(_, end)| end as usize);
    let stripped = if let Some(c) = caret {
        strip_trigger_before_caret(&value, c).or_else(|| strip_trailing_trigger(&value))
    } else {
        strip_trailing_trigger(&value)
    };
    let Some(new_value) = stripped else {
        return Err(Error::msg("focused value has no trailing @@"));
    };
    if !el.set_string(kAXValueAttribute, &new_value) {
        return Err(Error::msg("AX set-value failed"));
    }
    Ok(())
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
    let Some(el) = focused_element() else {
        return Ok(false);
    };
    let bundle = crate::focus::macos_frontmost_bundle().unwrap_or_default();
    if is_browser_bundle(&bundle) {
        return paste_cmd_v();
    }
    if let Some(current) = el.string_attr(kAXValueAttribute) {
        let new_value = if let Some((start, end)) = range {
            replace_utf8_range(&current, start, end, text)
        } else if let Some((start, end)) = selected_range(&el) {
            if start != end {
                replace_utf8_range(&current, start, end, text)
            } else {
                insert_utf8_at(&current, start, text)
            }
        } else {
            format!("{current}{text}")
        };
        if el.set_string(kAXValueAttribute, &new_value) {
            return Ok(true);
        }
    }
    // Browsers / Electron often have no settable AXValue. Paste once.
    paste_cmd_v()
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

fn insert_utf8_at(current: &str, at: i32, text: &str) -> String {
    replace_utf8_range(current, at, at, text)
}

fn paste_cmd_v() -> Result<bool> {
    post_cmd_v();
    Ok(true)
}

fn post_cmd_v() {
    unsafe {
        let src = CGEventSourceCreate(1);
        if src.is_null() {
            return;
        }
        let down = CGEventCreateKeyboardEvent(src, 0x09, true);
        let up = CGEventCreateKeyboardEvent(src, 0x09, false);
        if !down.is_null() {
            CGEventSetFlags(down, 0x100_000);
            CGEventPost(0, down);
            CFRelease(down);
        }
        if !up.is_null() {
            CGEventSetFlags(up, 0x100_000);
            CGEventPost(0, up);
            CFRelease(up);
        }
        CFRelease(src);
    }
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventSourceCreate(state: i32) -> *mut c_void;
    fn CGEventCreateKeyboardEvent(src: *mut c_void, keycode: u16, down: bool) -> *mut c_void;
    fn CGEventSetFlags(event: *mut c_void, flags: u64);
    fn CGEventPost(tap: u32, event: *mut c_void);
}

/// Mouse-up only. Shift+arrow never arrives here.
pub fn spawn_mouse_up_watcher(tx: Sender<MouseUpHit>) {
    crate::macos_runtime::install_mouse_up_on_main(tx);
}
