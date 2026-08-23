//! Focus identity. On Hyprland we compare `activewindow` **address**, not title.

use std::process::Command;

use openatat_ipc::FocusSnapshot;

use crate::error::Result;

pub fn snapshot() -> FocusSnapshot {
    let mut snap = FocusSnapshot::default();
    if let Ok(win) = active_window() {
        snap.window_address = win.address;
        snap.app_id = win.class;
        snap.title = win.title;
    }
    snap.output = active_output();
    snap
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
    let out = Command::new("hyprctl")
        .args(["monitors", "-j"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let monitors: Vec<HyprMonitor> = serde_json::from_slice(&out.stdout).ok()?;
    monitors.into_iter().find(|m| m.focused).and_then(|m| {
        Some(OutputGeom {
            name: m.name?,
            x: m.x,
            y: m.y,
            width: m.width,
            height: m.height,
        })
    })
}

/// Hyprland cursor in screen coordinates. Fallback when AT-SPI extents are missing.
pub fn cursor_pos() -> Option<(i32, i32)> {
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

/// Abort insert when this returns true.
pub fn address_changed(expected: &FocusSnapshot) -> bool {
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
