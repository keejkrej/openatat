//! Native nonactivating region picker (C2). Not gpui. Not iced.
//!
//! Linux: layer-shell Overlay, dimmed, rubber-band, OnDemand only while
//! picking. macOS: NSPanel nonactivating. Windows: `WS_EX_NOACTIVATE`.
//! Esc cancels without opening `@@`. Portal / slurp / grim -g are not the
//! product picker.

use crate::error::Result;
use crate::focus::OutputGeom;

pub(crate) mod draw;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickedRegion {
    /// Layout / screen coordinates (grim `-g`, SCK crop, WGC crop).
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Output the rubber-band lived on, when known.
    pub output: Option<String>,
    /// Surface size the rect was drawn in (for scale mapping).
    pub surface_w: u32,
    pub surface_h: u32,
}

impl PickedRegion {
    pub fn as_grim_geometry(&self) -> String {
        format!("{},{} {}x{}", self.x, self.y, self.width, self.height)
    }
}

/// Native picker. `Ok(None)` is Esc / empty drag — do not open `@@`.
pub fn pick_region() -> Result<Option<PickedRegion>> {
    #[cfg(target_os = "linux")]
    {
        return linux::pick_region();
    }
    #[cfg(target_os = "macos")]
    {
        return macos::pick_region();
    }
    #[cfg(target_os = "windows")]
    {
        return windows::pick_region();
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Err(crate::error::Error::msg("area picker unsupported on this OS"))
    }
}

/// Documented fallback when the native picker cannot map (like `--demo`
/// for trigger). `slurp` becomes the front app — not the product path.
#[cfg(target_os = "linux")]
pub fn slurp_fallback() -> Result<Option<PickedRegion>> {
    use std::process::Command;
    let out = Command::new("slurp")
        .args(["-f", "%x,%y %wx%h"])
        .output()
        .map_err(|e| {
            crate::error::Error::msg(format!(
                "native area picker could not map and slurp is missing ({e}). \
                 On Omarchy add a Hyprland bind to `openatatd --capture area` \
                 and run under Hyprland."
            ))
        })?;
    if !out.status.success() {
        return Ok(None);
    }
    parse_slurp_geometry(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(not(target_os = "linux"))]
pub fn slurp_fallback() -> Result<Option<PickedRegion>> {
    Err(crate::error::Error::msg(
        "slurp fallback is Linux-only; use the native picker or --capture",
    ))
}

pub fn parse_slurp_geometry(s: &str) -> Result<Option<PickedRegion>> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    // "x,y wxh"
    let (pos, size) = s
        .split_once(' ')
        .ok_or_else(|| crate::error::Error::msg("slurp geometry is not `x,y wxh`"))?;
    let (x, y) = pos
        .split_once(',')
        .ok_or_else(|| crate::error::Error::msg("slurp geometry is not `x,y wxh`"))?;
    let (w, h) = size
        .split_once('x')
        .ok_or_else(|| crate::error::Error::msg("slurp geometry is not `x,y wxh`"))?;
    let x: i32 = x.parse().map_err(|_| crate::error::Error::msg("slurp x"))?;
    let y: i32 = y.parse().map_err(|_| crate::error::Error::msg("slurp y"))?;
    let width: u32 = w.parse().map_err(|_| crate::error::Error::msg("slurp w"))?;
    let height: u32 = h.parse().map_err(|_| crate::error::Error::msg("slurp h"))?;
    if width < super::policy::min_region_px() || height < super::policy::min_region_px() {
        return Ok(None);
    }
    Ok(Some(PickedRegion {
        x,
        y,
        width,
        height,
        output: None,
        surface_w: width,
        surface_h: height,
    }))
}

pub fn output_for_picker() -> Option<OutputGeom> {
    crate::focus::output_under_pointer().or_else(crate::focus::focused_output)
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_picker_wants_keys() -> bool {
    windows::picker_wants_keys()
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_picker_hwnd() -> Option<isize> {
    windows::picker_hwnd_pub()
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_feed_picker_vk(vk: u32) {
    windows::feed_vk(vk)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slurp_geometry_parses() {
        let r = parse_slurp_geometry("100,200 30x40").unwrap().unwrap();
        assert_eq!((r.x, r.y, r.width, r.height), (100, 200, 30, 40));
        assert_eq!(r.as_grim_geometry(), "100,200 30x40");
        assert!(parse_slurp_geometry("10,10 1x1").unwrap().is_none());
        assert!(parse_slurp_geometry("").unwrap().is_none());
    }

    #[test]
    fn picker_sources_forbid_gpui_iced_and_activation() {
        let files = [
            ("mod.rs", include_str!("mod.rs")),
            ("draw.rs", include_str!("draw.rs")),
            ("linux.rs", include_str!("linux.rs")),
            ("macos.rs", include_str!("macos.rs")),
            ("windows.rs", include_str!("windows.rs")),
        ];
        for (name, src) in files {
            let gpui = ["gpui", "::"].concat();
            let iced = ["iced", "::"].concat();
            let set_fg = ["SetForegroundWindow", "("].concat();
            assert!(!src.contains(&gpui), "{name} must not use gpui");
            assert!(!src.contains(&iced), "{name} must not use iced");
            assert!(!src.contains(&set_fg), "{name} must never activate");
            let picker = ["GraphicsCapture", "Picker"].concat();
            assert!(
                !src.contains(&picker),
                "{name} must not use the system capture picker"
            );
            let portal = ["xdg-desktop", "-portal"].concat();
            assert!(
                !src.contains(&portal),
                "{name} must not use the portal Screenshot chooser"
            );
            let stub = ["ScreenCapture", "Frame"].concat();
            assert!(
                !src.contains(&stub),
                "{name} must not use the gpui capture stub"
            );
        }
    }

    #[test]
    fn product_picker_is_not_slurp_or_grim_g() {
        let linux = include_str!("linux.rs");
        assert!(
            !linux.contains("Command::new(\"slurp\")"),
            "slurp is a documented fallback, not the product picker"
        );
        assert!(
            !linux.contains("Command::new(\"grim\")") && !linux.contains("\"-g\""),
            "grim -g is the crop backend, not the picker"
        );
    }
}
