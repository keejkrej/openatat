//! C10 live text selection bar. Native applet, same layer-shell class as `@@`.
//!
//! Mouse-up only. Selected text is ephemeral: used for the action, never
//! written to history, never logged. Secure fields are re-probed every time.

use crate::a11y::{self, SelectionProbe, TextSelection};
use crate::error::Result;
use crate::trigger::FieldKind;

mod policy;

pub use policy::{
    decide_summon, history_prompt_for, launch_prompt_for, search_url, BarAction, PromptAction,
    SelectionOrigin, SummonDecision,
};

/// One candidate from the watcher or a test inject. `Debug` redacts text.
#[derive(Clone)]
pub struct SelectionHit {
    pub origin: SelectionOrigin,
    pub probe: SelectionProbe,
    pub pointer: Option<(i32, i32)>,
}

impl std::fmt::Debug for SelectionHit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SelectionHit")
            .field("origin", &self.origin)
            .field("probe", &self.probe)
            .field("pointer", &self.pointer)
            .finish()
    }
}

/// Where to put the compact bar (layer-shell TOP|LEFT margins).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub margin_top: i32,
    pub margin_left: i32,
}

pub const BAR_W: u32 = 520;
pub const BAR_H: u32 = 40;

pub fn placement_for(
    bounds: Option<a11y::Rect>,
    pointer: Option<(i32, i32)>,
    output: Option<crate::focus::OutputGeom>,
) -> Placement {
    let (sx, sy) = if let Some(r) = bounds {
        (r.x, r.y + r.height + 8)
    } else if let Some((x, y)) = pointer {
        (x + 12, y + 16)
    } else {
        return Placement {
            margin_top: 80,
            margin_left: 0,
        };
    };
    if let Some(out) = output {
        let max_x = (out.width as i32 - BAR_W as i32 - 8).max(8);
        let max_y = (out.height as i32 - BAR_H as i32 - 8).max(8);
        let x = (sx - out.x).clamp(8, max_x);
        let y = (sy - out.y).clamp(8, max_y);
        Placement {
            margin_top: y,
            margin_left: x,
        }
    } else {
        Placement {
            margin_top: sy.max(8),
            margin_left: sx.max(8),
        }
    }
}

/// Apply the product gate. Re-probes field kind every call (never cached).
pub fn decide_hit(hit: &SelectionHit, field: FieldKind) -> SummonDecision {
    let text = match &hit.probe {
        SelectionProbe::Secure => return SummonDecision::SkipSecure,
        SelectionProbe::Unavailable => None,
        SelectionProbe::NoSelection => Some(""),
        SelectionProbe::Ready(sel) => Some(sel.text()),
    };
    if matches!(hit.probe, SelectionProbe::Secure) || field == FieldKind::Secure {
        return SummonDecision::SkipSecure;
    }
    decide_summon(hit.origin, field, text)
}

pub fn ready_selection(hit: &SelectionHit) -> Option<&TextSelection> {
    match &hit.probe {
        SelectionProbe::Ready(sel) if !sel.is_empty() => Some(sel),
        _ => None,
    }
}

/// Re-probe password role before using a held selection. Never reads secure text.
pub fn allow_held_selection() -> bool {
    a11y::probe_field_kind() != FieldKind::Secure
}

pub fn copy_selection(sel: &TextSelection) -> Result<()> {
    crate::clipboard::copy_plain_or_html(sel.text(), sel.html())
}

pub fn search_selection(sel: &TextSelection) -> Result<()> {
    open_search(sel.text())
}

pub fn open_search(query: &str) -> Result<()> {
    let url = search_url(query);
    // Do not log `url` — it contains the ephemeral selection.
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(opener)
        .arg(&url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| crate::error::Error::msg(format!("{opener}: {e}")))
}

/// Start the mouse-up watcher. Keyboard selections never arrive here.
pub fn spawn_watcher(tx: std::sync::mpsc::Sender<SelectionHit>) {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let (raw_tx, raw_rx) = std::sync::mpsc::channel();
        a11y::spawn_mouse_up_watcher(raw_tx);
        std::thread::Builder::new()
            .name("openatat-sel-map".into())
            .spawn(move || {
                while let Ok(hit) = raw_rx.recv() {
                    let mapped = SelectionHit {
                        origin: SelectionOrigin::MouseUp,
                        probe: hit.probe,
                        pointer: hit.pointer,
                    };
                    if tx.send(mapped).is_err() {
                        break;
                    }
                }
            })
            .ok();
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = tx;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a11y::Rect;

    #[test]
    fn placement_prefers_extents_then_pointer() {
        let p = placement_for(
            Some(Rect {
                x: 100,
                y: 40,
                width: 80,
                height: 16,
            }),
            Some((0, 0)),
            None,
        );
        assert_eq!(p.margin_left, 100);
        assert_eq!(p.margin_top, 64);
        let p = placement_for(None, Some((20, 30)), None);
        assert_eq!((p.margin_left, p.margin_top), (32, 46));
        let p = placement_for(None, None, None);
        assert_eq!(p.margin_top, 80);
    }

    #[test]
    fn placement_clamps_to_output() {
        let out = crate::focus::OutputGeom {
            name: "DP-1".into(),
            x: 0,
            y: 0,
            width: 200,
            height: 100,
        };
        let p = placement_for(
            Some(Rect {
                x: 180,
                y: 90,
                width: 10,
                height: 10,
            }),
            None,
            Some(out),
        );
        // Output is narrower than the bar; clamp to the 8px inset.
        assert_eq!(p.margin_left, 8);
        assert!(p.margin_top >= 8 && p.margin_top < 100);
    }
}
