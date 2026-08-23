//! C14 clipboard-shelf policy. Display-free so Linux CI can lock the contract.
//!
//! Passwords never enter the shelf. Recording-off drops new items and keeps
//! existing ones. Consecutive identical copies are ignored. The shelf is not
//! `history.jsonl`. Nothing here logs clip contents.

use crate::trigger::FieldKind;

/// Soft cap. Oldest rows fall off the end of the local file.
pub const MAX_ITEMS: usize = 100;

/// Skip images larger than this. Optional and cheap, not a screenshot archive.
pub const MAX_IMAGE_BYTES: usize = 512 * 1024;

/// One offer the watcher saw. `Debug` redacts payloads.
#[derive(Clone, PartialEq, Eq)]
pub struct Incoming {
    pub plain: String,
    pub html: Option<String>,
    pub rtf: Option<String>,
    pub image: Option<Vec<u8>>,
}

impl Incoming {
    pub fn is_empty(&self) -> bool {
        self.plain.trim().is_empty()
            && self.html.as_deref().map(str::trim).unwrap_or("").is_empty()
            && self.rtf.as_deref().map(str::trim).unwrap_or("").is_empty()
            && self.image.as_ref().map(|b| b.is_empty()).unwrap_or(true)
    }

    pub fn fingerprint(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        self.plain.hash(&mut h);
        self.html.hash(&mut h);
        self.rtf.hash(&mut h);
        match &self.image {
            Some(b) => {
                b.len().hash(&mut h);
                if let Some(head) = b.get(..32) {
                    head.hash(&mut h);
                }
            }
            None => 0u8.hash(&mut h),
        }
        h.finish()
    }
}

impl std::fmt::Debug for Incoming {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Incoming")
            .field("chars", &self.plain.chars().count())
            .field("has_html", &self.html.is_some())
            .field("has_rtf", &self.rtf.is_some())
            .field("image_bytes", &self.image.as_ref().map(|b| b.len()))
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordDecision {
    Store,
    SkipSecure,
    SkipRecordingOff,
    SkipDuplicate,
    SkipEmpty,
}

/// Re-probe `field` on every copy. Never cache a password role.
pub fn decide_record(
    recording: bool,
    field: FieldKind,
    incoming: &Incoming,
    last: Option<&Incoming>,
) -> RecordDecision {
    if field == FieldKind::Secure {
        return RecordDecision::SkipSecure;
    }
    if !recording {
        return RecordDecision::SkipRecordingOff;
    }
    if incoming.is_empty() {
        return RecordDecision::SkipEmpty;
    }
    if last.is_some_and(|prev| same_content(prev, incoming)) {
        return RecordDecision::SkipDuplicate;
    }
    RecordDecision::Store
}

pub fn same_content(a: &Incoming, b: &Incoming) -> bool {
    a.plain == b.plain && a.html == b.html && a.rtf == b.rtf && a.image == b.image
}

/// Case-insensitive substring. Empty query matches everything.
pub fn matches_query(plain: &str, query: &str) -> bool {
    let q = query.trim();
    if q.is_empty() {
        return true;
    }
    plain.to_lowercase().contains(&q.to_lowercase())
}

pub fn preview_line(plain: &str, has_image: bool) -> String {
    if plain.trim().is_empty() && has_image {
        return "[image]".into();
    }
    let one: String = plain
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .take(56)
        .collect();
    if plain.chars().count() > 56 {
        format!("{one}…")
    } else {
        one
    }
}

/// Which OS the in-process shortcut (if any) belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShelfOs {
    Linux,
    Mac,
    Windows,
}

/// Linux never installs a compositor bind. Mac/Win use a clipboard chord,
/// not a `@@` summon. Screenshot keys (`3`/`4`/`5`) are never the shelf.
pub fn is_shelf_shortcut(
    os: ShelfOs,
    meta: bool,
    shift: bool,
    ctrl: bool,
    alt: bool,
    win: bool,
    key: char,
) -> bool {
    if is_screenshot_key(key) {
        return false;
    }
    match os {
        ShelfOs::Linux => false,
        ShelfOs::Mac => {
            meta && shift && !ctrl && !alt && !win && key.eq_ignore_ascii_case(&'v')
        }
        ShelfOs::Windows => {
            win && shift && !ctrl && !alt && key.eq_ignore_ascii_case(&'v')
        }
    }
}

pub fn is_screenshot_key(key: char) -> bool {
    matches!(key, '3' | '4' | '5')
}

/// The daemon must not write a Hyprland / compositor bind for the shelf.
pub fn daemon_installs_compositor_bind() -> bool {
    false
}

/// Default when `[clipboard] shelf` is omitted.
pub fn default_recording_on() -> bool {
    true
}

pub fn recording_from_toml(shelf: Option<bool>) -> bool {
    shelf.unwrap_or_else(default_recording_on)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(plain: &str) -> Incoming {
        Incoming {
            plain: plain.into(),
            html: None,
            rtf: None,
            image: None,
        }
    }

    #[test]
    fn password_field_never_stores_even_if_text_is_present() {
        let incoming = text("hunter2");
        assert_eq!(
            decide_record(true, FieldKind::Secure, &incoming, None),
            RecordDecision::SkipSecure
        );
        assert_eq!(
            decide_record(true, FieldKind::Secure, &incoming, Some(&incoming)),
            RecordDecision::SkipSecure
        );
    }

    #[test]
    fn normal_field_stores_when_recording_is_on() {
        let incoming = text("hello from a normal field");
        assert_eq!(
            decide_record(true, FieldKind::AcceptsText, &incoming, None),
            RecordDecision::Store
        );
    }

    #[test]
    fn recording_off_drops_new_items() {
        let incoming = text("should not land");
        assert_eq!(
            decide_record(false, FieldKind::AcceptsText, &incoming, None),
            RecordDecision::SkipRecordingOff
        );
    }

    #[test]
    fn consecutive_identical_copies_are_deduped() {
        let a = text("same");
        let b = text("same");
        assert_eq!(
            decide_record(true, FieldKind::AcceptsText, &b, Some(&a)),
            RecordDecision::SkipDuplicate
        );
        let c = text("other");
        assert_eq!(
            decide_record(true, FieldKind::AcceptsText, &c, Some(&a)),
            RecordDecision::Store
        );
    }

    #[test]
    fn empty_offer_is_skipped() {
        assert_eq!(
            decide_record(true, FieldKind::AcceptsText, &text("   "), None),
            RecordDecision::SkipEmpty
        );
    }

    #[test]
    fn search_is_case_insensitive() {
        assert!(matches_query("Hello World", "world"));
        assert!(!matches_query("Hello World", "xyz"));
        assert!(matches_query("anything", ""));
    }

    #[test]
    fn preview_redacts_newlines_and_truncates() {
        let line = preview_line("one\ntwo", false);
        assert!(!line.contains('\n'), "{line}");
        assert!(preview_line("", true).contains("image"));
    }

    #[test]
    fn debug_does_not_print_payload() {
        let incoming = text("SECRET_CLIP_PAYLOAD");
        let dbg = format!("{incoming:?}");
        assert!(!dbg.contains("SECRET_CLIP_PAYLOAD"), "{dbg}");
        assert!(dbg.contains("chars"));
    }

    #[test]
    fn mac_and_win_chords_are_not_summon_and_not_screenshots() {
        assert!(is_shelf_shortcut(
            ShelfOs::Mac,
            true,
            true,
            false,
            false,
            false,
            'v'
        ));
        assert!(is_shelf_shortcut(
            ShelfOs::Windows,
            false,
            true,
            false,
            false,
            true,
            'V'
        ));
        assert!(!is_shelf_shortcut(
            ShelfOs::Linux,
            true,
            true,
            false,
            false,
            true,
            'v'
        ));
        assert!(!is_shelf_shortcut(
            ShelfOs::Mac,
            true,
            true,
            false,
            false,
            false,
            '3'
        ));
        assert!(!is_shelf_shortcut(
            ShelfOs::Mac,
            true,
            true,
            false,
            false,
            false,
            '4'
        ));
        assert!(!is_shelf_shortcut(
            ShelfOs::Mac,
            true,
            true,
            false,
            false,
            false,
            '5'
        ));
        assert!(is_screenshot_key('3'));
        assert!(!daemon_installs_compositor_bind());
    }

    #[test]
    fn recording_defaults_on() {
        assert!(default_recording_on());
        assert!(recording_from_toml(None));
        assert!(!recording_from_toml(Some(false)));
        assert!(recording_from_toml(Some(true)));
    }
}
