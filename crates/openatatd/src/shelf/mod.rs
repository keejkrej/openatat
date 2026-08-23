//! C14 clipboard history shelf. Native applet, same nonactivating class as `@@`.
//!
//! Watch is always-on in `openatatd` (CPU poll). The shelf surface is unmapped
//! at idle (keyboard none). Open via `--shelf` / `{"cmd":"shelf"}` (Linux),
//! `⌘⇧V` when Input Monitoring is granted (macOS), or `Win+Shift+V` on the
//! process-local hook (Windows). Not a `@@` summon. Not gpui.

pub(crate) mod policy;
mod store;
mod watch;

pub use policy::{
    decide_record, default_recording_on, is_shelf_shortcut, matches_query, preview_line,
    recording_from_toml, Incoming, RecordDecision, ShelfOs, MAX_ITEMS,
};
pub use store::{filtered, Clip};
pub use watch::spawn_watcher;

use crate::error::Result;
use crate::focus;
use crate::insert::{self, InsertOutcome};
use crate::overlay::{self, OverlayEnd};
use crate::session::{self, Session, SessionEnd};
use crate::trigger::FieldKind;
use openatat_ipc::EntryPoint;

/// In-memory shelf list the overlay controller drives.
#[derive(Clone)]
pub struct ShelfView {
    pub query: String,
    pub selected: usize,
    pub items: Vec<Clip>,
    pub recording: bool,
}

impl ShelfView {
    pub fn load() -> Self {
        Self {
            query: String::new(),
            selected: 0,
            items: store::load(),
            recording: crate::agent::AgentConfig::load().clipboard_shelf_enabled(),
        }
    }

    pub fn visible(&self) -> Vec<&Clip> {
        self.items
            .iter()
            .filter(|c| matches_query(&c.plain, &self.query))
            .collect()
    }

    pub fn selected_clip(&self) -> Option<&Clip> {
        self.visible().get(self.selected).copied()
    }

    pub fn clamp_selected(&mut self) {
        let n = self.visible().len();
        if n == 0 {
            self.selected = 0;
        } else if self.selected >= n {
            self.selected = n - 1;
        }
    }

    pub fn move_sel(&mut self, delta: isize) {
        let n = self.visible().len() as isize;
        if n == 0 {
            self.selected = 0;
            return;
        }
        let next = (self.selected as isize + delta).clamp(0, n - 1);
        self.selected = next as usize;
    }

    pub fn preview_rows(&self) -> Vec<String> {
        self.visible()
            .into_iter()
            .map(|c| preview_line(&c.plain, c.has_image()))
            .collect()
    }
}

pub fn count() -> usize {
    store::count()
}

pub fn record_if_allowed(field: FieldKind, incoming: Incoming) -> RecordDecision {
    let recording = crate::agent::AgentConfig::load().clipboard_shelf_enabled();
    let last = store::load().first().map(|c| c.incoming());
    let d = decide_record(recording, field, &incoming, last.as_ref());
    if d == RecordDecision::Store {
        let _ = store::push(&incoming);
    }
    d
}

pub fn paste_clip(clip: &Clip, expected: &openatat_ipc::FocusSnapshot) -> Result<InsertOutcome> {
    insert::paste_formatted(
        &clip.plain,
        clip.html.as_deref(),
        clip.rtf.as_deref(),
        clip.incoming().image.as_deref(),
        expected,
    )
}

pub fn run_shelf() -> Result<SessionEnd> {
    let mut session = Session::begin_shelf();
    match overlay::run_shelf(&mut session) {
        Ok(OverlayEnd::Cancelled) => Ok(SessionEnd::Cancelled),
        Ok(OverlayEnd::Copied) | Ok(OverlayEnd::Pasted) => Ok(SessionEnd::CopiedOnly),
        Ok(OverlayEnd::Handoff) => Ok(SessionEnd::HandedOff),
        Ok(OverlayEnd::Tab) => Ok(SessionEnd::CopiedOnly),
        Ok(OverlayEnd::ShelfAsk) => {
            let mut next = Session::begin_orb_click();
            next.dropped_text = session.dropped_text;
            next.focus = session.focus;
            session::run_overlay_from_shelf(next)
        }
        Err(e) if e.is_wayland_connect() => {
            eprintln!(
                "openatatd: shelf overlay unavailable ({e}); {} items on disk (contents not logged)",
                count()
            );
            Ok(SessionEnd::Cancelled)
        }
        Err(e) => Err(e),
    }
}

pub fn run_headless_shelf() -> Result<SessionEnd> {
    let n = count();
    eprintln!("openatatd: shelf ({n} items, contents not logged)");
    let _ = (EntryPoint::Orb, focus::snapshot());
    Ok(SessionEnd::CopiedOnly)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shelf_sources_forbid_gpui_iced_and_set_foreground() {
        for (name, src) in [
            ("mod.rs", include_str!("mod.rs")),
            ("policy.rs", include_str!("policy.rs")),
            ("store.rs", include_str!("store.rs")),
            ("watch.rs", include_str!("watch.rs")),
        ] {
            let gpui = ["gpui", "::"].concat();
            let iced = ["iced", "::"].concat();
            let set_fg = ["SetForegroundWindow", "("].concat();
            let hypr_kw = ["hyprctl ", "keyword bind"].concat();
            let hypr_ds = ["hyprctl ", "dispatch bind"].concat();
            assert!(!src.contains(&gpui), "{name} must not use gpui");
            assert!(!src.contains(&iced), "{name} must not use iced");
            assert!(!src.contains(&set_fg), "{name} must never activate");
            assert!(
                !src.contains(&hypr_kw) && !src.contains(&hypr_ds),
                "{name} must not install a compositor bind"
            );
        }
    }

    #[test]
    fn history_jsonl_schema_is_unchanged() {
        let rec = openatat_ipc::HistoryRecord {
            id: "1".into(),
            timestamp: "0".into(),
            entry: openatat_ipc::EntryPoint::Demo,
            prompt: "x".into(),
        };
        let v = serde_json::to_value(&rec).unwrap();
        let obj = v.as_object().unwrap();
        assert_eq!(obj.len(), 4);
        assert!(obj.contains_key("id"));
        assert!(obj.contains_key("timestamp"));
        assert!(obj.contains_key("entry"));
        assert!(obj.contains_key("prompt"));
        assert!(!obj.contains_key("plain"));
        assert!(!obj.contains_key("html"));
    }

    #[test]
    fn view_search_and_selection() {
        let mut view = ShelfView {
            query: String::new(),
            selected: 0,
            recording: true,
            items: vec![
                Clip {
                    id: "a".into(),
                    timestamp: "1".into(),
                    plain: "alpha".into(),
                    html: None,
                    rtf: None,
                    image_path: None,
                },
                Clip {
                    id: "b".into(),
                    timestamp: "2".into(),
                    plain: "bravo".into(),
                    html: None,
                    rtf: None,
                    image_path: None,
                },
            ],
        };
        assert_eq!(view.visible().len(), 2);
        view.query = "br".into();
        view.clamp_selected();
        assert_eq!(view.visible().len(), 1);
        assert_eq!(view.selected_clip().unwrap().plain, "bravo");
        view.query.clear();
        view.selected = 1;
        view.move_sel(-1);
        assert_eq!(view.selected, 0);
    }
}
