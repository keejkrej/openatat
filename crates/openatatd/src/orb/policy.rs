//! Display-free Orb rules. Linux CI locks these without a compositor.
//!
//! Product (Atat manual / FAQ):
//! - Click opens an empty `@@` input: no auto-screenshot, no attachments.
//! - Typed `@@` still auto-attaches C1.
//! - Hide lasts this launch only and does not disable `@@` / selection / capture.
//! - Drop uses real file/text/image payloads, never a Nautilus / Explorer title.

use std::path::{Path, PathBuf};

use openatat_ipc::EntryPoint;

use crate::agent::Attachment;
use crate::capture::Still;
use crate::session::Session;

/// What the Orb accepted. Paths come from the drop protocol, never a title bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrbDrop {
    File(PathBuf),
    /// Raw image bytes from the drag (PNG). Not a screenshot we took.
    Image { png: Vec<u8> },
    Text(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrbOpenKind {
    Click,
    Drop,
}

/// C1 auto-still is only for typed `@@` / demo / Finder — never Orb click/drop.
pub fn auto_attach_c1(entry: EntryPoint) -> bool {
    !matches!(entry, EntryPoint::Orb)
}

/// Hide only covers the resting Orb. Typed `@@` stays live.
pub fn typed_trigger_allowed(_show_orb: bool) -> bool {
    true
}

/// Selection bar and other explicit entries stay live while the Orb is hidden.
pub fn explicit_entry_allowed(_show_orb: bool) -> bool {
    true
}

/// A stray click must not cancel a running agent. Esc is the only cancel.
pub fn click_while_busy_is_ignored(busy: bool) -> bool {
    busy
}

/// Next launch always starts with Show Orb checked. Hide is not persisted.
pub fn show_orb_on_launch() -> bool {
    true
}

/// Window titles (Nautilus, Explorer, Finder) are never drop sources.
pub fn paths_from_window_title(_title: &str) -> Vec<PathBuf> {
    Vec::new()
}

/// `text/uri-list` from a Wayland / Cocoa / Win32 drop. Relative and
/// non-`file:` lines are dropped. Title-looking strings are not parsed.
pub fn parse_uri_list(raw: &str) -> Vec<PathBuf> {
    raw.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(file_uri_to_path)
        .collect()
}

fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file:")?;
    let rest = rest.strip_prefix("//").unwrap_or(rest);
    let path_part = if rest.starts_with('/') {
        rest
    } else if let Some(p) = rest.strip_prefix("localhost") {
        p
    } else {
        return None;
    };
    let decoded = percent_decode(path_part);
    if decoded.is_empty() {
        return None;
    }
    let p = PathBuf::from(decoded);
    p.is_absolute().then_some(p)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (from_hex(bytes[i + 1]), from_hex(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

pub fn apply_drops(session: &mut Session, drops: Vec<OrbDrop>) {
    for drop in drops {
        match drop {
            OrbDrop::File(path) => {
                if path.is_absolute() {
                    session.dropped_files.push(path);
                }
            }
            OrbDrop::Image { png } => {
                if session.still.is_none() {
                    if let Ok(still) = crate::capture::downscale_long_edge(&png, crate::capture::LONG_EDGE)
                    {
                        session.still = Some(still);
                    } else {
                        session.still = Some(Still {
                            png,
                            width: 0,
                            height: 0,
                        });
                    }
                }
            }
            OrbDrop::Text(text) => {
                if !text.is_empty() {
                    session.dropped_text.push(text);
                }
            }
        }
    }
}

pub fn drop_attachments(session: &Session) -> Vec<Attachment> {
    let mut out = Vec::new();
    for path in &session.dropped_files {
        out.push(Attachment::File { path: path.clone() });
    }
    for text in &session.dropped_text {
        out.push(Attachment::Snippet {
            text: text.clone(),
        });
    }
    out
}

pub fn is_image_path(path: &Path) -> bool {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp") => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Session;
    use openatat_ipc::{FocusSnapshot, TriggerSource};

    fn empty() -> Session {
        Session {
            source: TriggerSource::Demo,
            entry: EntryPoint::Orb,
            focus: FocusSnapshot::default(),
            still: None,
            prompt: String::new(),
            preview: None,
            selection: None,
            placement: None,
            finder_cwd: None,
            finder_files: Vec::new(),
            dropped_files: Vec::new(),
            dropped_text: Vec::new(),
            video: None,
        }
    }

    #[test]
    fn orb_click_skips_c1_still() {
        assert!(!auto_attach_c1(EntryPoint::Orb));
        assert!(auto_attach_c1(EntryPoint::TextField));
        assert!(auto_attach_c1(EntryPoint::Demo));
        let s = Session::begin_orb_click();
        assert_eq!(s.entry, EntryPoint::Orb);
        assert!(s.still.is_none(), "Orb click must not attach a still");
        assert!(s.finder_files.is_empty());
        assert!(s.finder_cwd.is_none());
        assert!(s.dropped_files.is_empty());
        assert!(s.dropped_text.is_empty());
    }

    #[test]
    fn hide_does_not_disable_typed_at_at() {
        assert!(typed_trigger_allowed(true));
        assert!(typed_trigger_allowed(false));
        assert!(explicit_entry_allowed(false));
        assert!(show_orb_on_launch());
    }

    #[test]
    fn busy_click_is_ignored() {
        assert!(click_while_busy_is_ignored(true));
        assert!(!click_while_busy_is_ignored(false));
    }

    #[test]
    fn uri_list_keeps_absolute_file_uris_only() {
        let raw = "\
# comment
file:///tmp/openatat-drop.png
file://localhost/tmp/notes.txt
https://example.com/nope
relative.txt
Nautilus
";
        let paths = parse_uri_list(raw);
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/tmp/openatat-drop.png"),
                PathBuf::from("/tmp/notes.txt")
            ]
        );
    }

    #[test]
    fn nautilus_title_is_never_a_path() {
        assert!(paths_from_window_title("Nautilus").is_empty());
        assert!(paths_from_window_title("/home/me/Projects").is_empty());
        assert!(paths_from_window_title("~/Documents").is_empty());
    }

    #[test]
    fn apply_drops_attaches_files_and_text_not_a_c1_still() {
        let mut s = empty();
        apply_drops(
            &mut s,
            vec![
                OrbDrop::File(PathBuf::from("/tmp/a.rs")),
                OrbDrop::Text("hello from drop".into()),
            ],
        );
        assert_eq!(s.dropped_files, vec![PathBuf::from("/tmp/a.rs")]);
        assert_eq!(s.dropped_text, vec!["hello from drop".to_string()]);
        assert!(s.still.is_none(), "file/text drop must not invent a C1 still");
        let atts = drop_attachments(&s);
        assert_eq!(atts.len(), 2);
    }

    #[test]
    fn percent_encoded_uri() {
        let paths = parse_uri_list("file:///tmp/hello%20world.txt\n");
        assert_eq!(paths, vec![PathBuf::from("/tmp/hello world.txt")]);
    }
}
