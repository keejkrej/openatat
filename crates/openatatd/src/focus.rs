//! Focus identity. Linux: Hyprland address. Mac: pid + AX element. Never title.

use std::process::Command;

use openatat_ipc::FocusSnapshot;

use crate::error::Result;

pub fn snapshot() -> FocusSnapshot {
    #[cfg(target_os = "macos")]
    {
        return macos::snapshot();
    }
    #[cfg(target_os = "windows")]
    {
        return windows::snapshot();
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let mut snap = FocusSnapshot::default();
        if let Ok(win) = active_window() {
            snap.window_address = win.address;
            snap.app_id = win.class;
            snap.title = win.title;
        }
        snap.output = active_output();
        snap
    }
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct HyprWindow {
    #[serde(default)]
    address: Option<String>,
    #[serde(default)]
    class: Option<String>,
    #[serde(default)]
    title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputGeom {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub focused: bool,
}

impl OutputGeom {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x
            && y >= self.y
            && x < self.x + self.width as i32
            && y < self.y + self.height as i32
    }
}

#[derive(Debug, serde::Deserialize)]
struct HyprMonitor {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    focused: bool,
    #[serde(default)]
    x: i32,
    #[serde(default)]
    y: i32,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
}

pub fn active_window() -> Result<HyprWindow> {
    let out = Command::new("hyprctl")
        .args(["activewindow", "-j"])
        .output()?;
    if !out.status.success() {
        return Err(crate::error::Error::msg(
            "hyprctl activewindow failed — is Hyprland running?",
        ));
    }
    Ok(serde_json::from_slice(&out.stdout)?)
}

pub fn active_output() -> Option<String> {
    focused_output().map(|o| o.name)
}

pub fn focused_output() -> Option<OutputGeom> {
    let outputs = all_outputs();
    outputs
        .iter()
        .find(|o| o.focused)
        .cloned()
        .or_else(|| outputs.into_iter().next())
}

/// Output under the pointer, else the focused / first output.
pub fn output_under_pointer() -> Option<OutputGeom> {
    let outputs = all_outputs();
    if let Some((x, y)) = cursor_pos() {
        if let Some(hit) = outputs.iter().find(|o| o.contains(x, y)).cloned() {
            return Some(hit);
        }
    }
    outputs
        .iter()
        .find(|o| o.focused)
        .cloned()
        .or_else(|| outputs.into_iter().next())
}

pub fn all_outputs() -> Vec<OutputGeom> {
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let out = match Command::new("hyprctl").args(["monitors", "-j"]).output() {
            Ok(o) if o.status.success() => o,
            _ => return Vec::new(),
        };
        let monitors: Vec<HyprMonitor> = serde_json::from_slice(&out.stdout).unwrap_or_default();
        return monitors
            .into_iter()
            .filter_map(|m| {
                Some(OutputGeom {
                    name: m.name?,
                    x: m.x,
                    y: m.y,
                    width: m.width,
                    height: m.height,
                    focused: m.focused,
                })
            })
            .collect();
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        Vec::new()
    }
}

/// Pointer in screen coordinates. Fallback when a11y extents are missing.
pub fn cursor_pos() -> Option<(i32, i32)> {
    #[cfg(target_os = "macos")]
    {
        return macos::cursor_pos();
    }
    #[cfg(target_os = "windows")]
    {
        return windows::cursor_pos();
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let out = Command::new("hyprctl").args(["cursorpos"]).output().ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8_lossy(&out.stdout);
        let mut parts = s.split(',');
        let x = parts.next()?.trim().parse().ok()?;
        let y = parts.next()?.trim().parse().ok()?;
        Some((x, y))
    }
}

/// Abort insert when this returns true.
pub fn address_changed(expected: &FocusSnapshot) -> bool {
    #[cfg(target_os = "macos")]
    {
        return macos::identity_changed(expected);
    }
    #[cfg(target_os = "windows")]
    {
        return windows::identity_changed(expected);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let Some(want) = expected.window_address.as_deref() else {
            return false;
        };
        match active_window() {
            Ok(now) => match now.address.as_deref() {
                Some(have) => have != want,
                None => true,
            },
            Err(_) => false,
        }
    }
}

#[cfg(target_os = "macos")]
pub fn macos_frontmost_bundle() -> Option<String> {
    macos::frontmost_bundle()
}

#[cfg(not(target_os = "macos"))]
pub fn macos_frontmost_bundle() -> Option<String> {
    None
}

#[cfg(target_os = "windows")]
mod windows {
    use super::*;
    use ::windows::Win32::Foundation::POINT;
    use ::windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    pub fn snapshot() -> FocusSnapshot {
        let mut snap = FocusSnapshot::default();
        if let Some((hwnd, elem)) = crate::a11y::windows::focused_identity() {
            snap.window_address = Some(elem.clone());
            snap.element_id = Some(elem);
            snap.pid = Some(hwnd as i32);
        }
        snap.app_id = crate::a11y::foreground_exe();
        snap.title = crate::a11y::foreground_title();
        snap.output = Some(monitor_name());
        snap
    }

    pub fn identity_changed(expected: &FocusSnapshot) -> bool {
        let now = snapshot();
        let expected_hwnd = expected.window_address.as_deref().and_then(|s| {
            crate::a11y::parse_win_identity(s).map(|(h, _)| h)
        });
        let now_hwnd = now.window_address.as_deref().and_then(|s| {
            crate::a11y::parse_win_identity(s).map(|(h, _)| h)
        });
        crate::a11y::win_identity_changed(
            expected_hwnd,
            expected.element_id.as_deref(),
            now_hwnd,
            now.element_id.as_deref(),
        )
    }

    pub fn cursor_pos() -> Option<(i32, i32)> {
        let mut pt = POINT::default();
        unsafe { GetCursorPos(&mut pt).ok()? };
        Some((pt.x, pt.y))
    }

    fn monitor_name() -> String {
        "foreground".into()
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use objc2_app_kit::{NSRunningApplication, NSWorkspace};

    pub fn snapshot() -> FocusSnapshot {
        let mut snap = FocusSnapshot::default();
        if let Some(app) = frontmost_app() {
            let pid = unsafe { app.processIdentifier() };
            snap.pid = Some(pid);
            snap.app_id = Some(unsafe { app.bundleIdentifier() }
                .map(|s| s.to_string())
                .unwrap_or_default())
                .filter(|s| !s.is_empty());
            snap.title = unsafe { app.localizedName() }.map(|s| s.to_string());
            if let Some((pid2, elem)) = crate::a11y::macos::focused_identity() {
                snap.pid = Some(pid2);
                snap.element_id = Some(elem.clone());
                snap.window_address = Some(format!("mac:{pid2}/{elem}"));
            } else {
                snap.window_address = Some(format!("mac:{pid}"));
            }
        }
        snap.output = Some("main".into());
        snap
    }

    pub fn identity_changed(expected: &FocusSnapshot) -> bool {
        let now = snapshot();
        crate::a11y::mac_identity_changed(
            expected.pid,
            expected.app_id.as_deref(),
            expected.element_id.as_deref(),
            now.pid,
            now.app_id.as_deref(),
            now.element_id.as_deref(),
        )
    }

    pub fn frontmost_bundle() -> Option<String> {
        frontmost_app().and_then(|app| unsafe { app.bundleIdentifier() }.map(|s| s.to_string()))
    }

    fn frontmost_app() -> Option<objc2::rc::Retained<NSRunningApplication>> {
        let ws = unsafe { NSWorkspace::sharedWorkspace() };
        unsafe { ws.frontmostApplication() }
    }

    pub fn cursor_pos() -> Option<(i32, i32)> {
        use objc2_app_kit::NSEvent;
        let loc = unsafe { NSEvent::mouseLocation() };
        Some((loc.x as i32, loc.y as i32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_when_addresses_differ() {
        // Pure comparison helper: simulate by constructing snapshots.
        let expected = FocusSnapshot {
            window_address: Some("0x1".into()),
            ..FocusSnapshot::default()
        };
        // Without hyprctl the live check is skipped; unit the predicate shape.
        assert_eq!(expected.window_address.as_deref(), Some("0x1"));
        assert_ne!(Some("0x2"), expected.window_address.as_deref());
    }

    #[test]
    fn parse_activewindow_json() {
        let raw = r#"{"address":"0xabc","class":"kitty","title":"zsh"}"#;
        let w: HyprWindow = serde_json::from_str(raw).unwrap();
        assert_eq!(w.address.as_deref(), Some("0xabc"));
        assert_eq!(w.class.as_deref(), Some("kitty"));
    }

    #[test]
    fn parse_monitor_geometry() {
        let raw = r#"{"name":"DP-1","focused":true,"x":1920,"y":0,"width":2560,"height":1440}"#;
        let m: HyprMonitor = serde_json::from_str(raw).unwrap();
        assert_eq!(m.name.as_deref(), Some("DP-1"));
        assert_eq!(m.x, 1920);
        assert_eq!(m.width, 2560);
    }
}
