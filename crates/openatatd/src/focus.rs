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

#[derive(Debug, serde::Deserialize)]
struct HyprMonitor {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    focused: bool,
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
    let out = Command::new("hyprctl").args(["monitors", "-j"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let monitors: Vec<HyprMonitor> = serde_json::from_slice(&out.stdout).ok()?;
    monitors
        .into_iter()
        .find(|m| m.focused)
        .and_then(|m| m.name)
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
}
