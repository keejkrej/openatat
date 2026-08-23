//! Display-free C2 / C4 policy. Linux CI locks the contract without a GPU.
//!
//! Area and display stills become one tile on the existing `@@` overlay.
//! The session constructor is Orb-click (no auto C1) plus the captured PNG.
//! Esc on the picker cancels without opening `@@`. The daemon never writes a
//! Hyprland bind and never steals OS screenshot keys.

use openatat_ipc::CaptureKind;

/// Which OS the in-process capture chord (if any) belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureOs {
    Linux,
    Mac,
    Windows,
}

/// Linux never installs a compositor bind. Mac may listen for `⌘⇧3` / `⌘⇧4`
/// only when Input Monitoring is already granted. Windows uses `Win+Shift+3/4`
/// on the existing process-local hook. Not a `@@` summon.
pub fn is_capture_shortcut(
    os: CaptureOs,
    kind: CaptureKind,
    meta: bool,
    shift: bool,
    ctrl: bool,
    alt: bool,
    win: bool,
    key: char,
) -> bool {
    let want = match kind {
        CaptureKind::Display => '3',
        CaptureKind::Area => '4',
    };
    if !key.eq_ignore_ascii_case(&want) {
        return false;
    }
    match os {
        CaptureOs::Linux => false,
        CaptureOs::Mac => meta && shift && !ctrl && !alt && !win,
        CaptureOs::Windows => win && shift && !ctrl && !alt,
    }
}

pub fn capture_kind_for_key(key: char) -> Option<CaptureKind> {
    match key {
        '3' => Some(CaptureKind::Display),
        '4' => Some(CaptureKind::Area),
        _ => None,
    }
}

/// The daemon must not write a Hyprland / compositor bind for capture.
pub fn daemon_installs_compositor_bind() -> bool {
    false
}

/// C2/C4 open the overlay through the Orb-click constructor. Typed `@@`
/// still auto-attaches C1 via [`crate::session::Session::begin`].
pub fn explicit_still_auto_attaches_c1() -> bool {
    false
}

/// Minimum rubber-band in surface pixels. A click without a drag cancels.
pub fn min_region_px() -> u32 {
    2
}

/// Normalize a rubber-band so width/height are positive.
pub fn normalize_rect(x0: i32, y0: i32, x1: i32, y1: i32) -> Option<(i32, i32, u32, u32)> {
    let x = x0.min(x1);
    let y = y0.min(y1);
    let w = x0.abs_diff(x1);
    let h = y0.abs_diff(y1);
    if w < min_region_px() || h < min_region_px() {
        return None;
    }
    Some((x, y, w, h))
}

/// Map a surface-local rect onto an output still (scale / letterbox).
pub fn map_surface_rect_to_image(
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    surface_w: u32,
    surface_h: u32,
    image_w: u32,
    image_h: u32,
) -> Option<(u32, u32, u32, u32)> {
    if surface_w == 0 || surface_h == 0 || image_w == 0 || image_h == 0 {
        return None;
    }
    let sx = image_w as f64 / surface_w as f64;
    let sy = image_h as f64 / surface_h as f64;
    let ix = ((x as f64) * sx).round().max(0.0) as u32;
    let iy = ((y as f64) * sy).round().max(0.0) as u32;
    let iw = ((w as f64) * sx).round().max(1.0) as u32;
    let ih = ((h as f64) * sy).round().max(1.0) as u32;
    let ix = ix.min(image_w.saturating_sub(1));
    let iy = iy.min(image_h.saturating_sub(1));
    let iw = iw.min(image_w.saturating_sub(ix)).max(1);
    let ih = ih.min(image_h.saturating_sub(iy)).max(1);
    Some((ix, iy, iw, ih))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_never_claims_an_in_process_chord() {
        assert!(!is_capture_shortcut(
            CaptureOs::Linux,
            CaptureKind::Area,
            true,
            true,
            false,
            false,
            true,
            '4'
        ));
        assert!(!is_capture_shortcut(
            CaptureOs::Linux,
            CaptureKind::Display,
            true,
            true,
            false,
            false,
            true,
            '3'
        ));
        assert!(!daemon_installs_compositor_bind());
        assert!(!explicit_still_auto_attaches_c1());
    }

    #[test]
    fn mac_and_win_chords_match_atat_without_summoning() {
        assert!(is_capture_shortcut(
            CaptureOs::Mac,
            CaptureKind::Display,
            true,
            true,
            false,
            false,
            false,
            '3'
        ));
        assert!(is_capture_shortcut(
            CaptureOs::Mac,
            CaptureKind::Area,
            true,
            true,
            false,
            false,
            false,
            '4'
        ));
        assert!(is_capture_shortcut(
            CaptureOs::Windows,
            CaptureKind::Display,
            false,
            true,
            false,
            false,
            true,
            '3'
        ));
        assert!(is_capture_shortcut(
            CaptureOs::Windows,
            CaptureKind::Area,
            false,
            true,
            false,
            false,
            true,
            '4'
        ));
        assert!(!is_capture_shortcut(
            CaptureOs::Mac,
            CaptureKind::Area,
            true,
            true,
            false,
            false,
            false,
            'v'
        ));
        assert!(!is_capture_shortcut(
            CaptureOs::Windows,
            CaptureKind::Display,
            false,
            true,
            false,
            false,
            true,
            's'
        ));
        assert_eq!(capture_kind_for_key('3'), Some(CaptureKind::Display));
        assert_eq!(capture_kind_for_key('4'), Some(CaptureKind::Area));
        assert_eq!(capture_kind_for_key('5'), None);
    }

    #[test]
    fn rubber_band_rejects_clicks_and_normalizes() {
        assert_eq!(normalize_rect(10, 10, 10, 10), None);
        assert_eq!(normalize_rect(40, 50, 10, 20), Some((10, 20, 30, 30)));
    }

    #[test]
    fn surface_rect_maps_onto_still() {
        let mapped = map_surface_rect_to_image(10, 20, 30, 40, 100, 100, 200, 200).unwrap();
        assert_eq!(mapped, (20, 40, 60, 80));
    }
}
