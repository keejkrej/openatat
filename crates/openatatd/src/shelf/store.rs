//! Bounded local shelf. Lives next to `history.jsonl` but is a different file.
//!
//! Schema is not `id, timestamp, entry, prompt`. Never write clips into
//! history. Callers must not log [`Clip`] payloads.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::policy::{Incoming, MAX_IMAGE_BYTES, MAX_ITEMS};
use crate::error::Result;
use crate::paths::{clipboard_shelf_dir, clipboard_shelf_path};

/// One stored clip. `Debug` redacts payloads.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clip {
    pub id: String,
    pub timestamp: String,
    pub plain: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtf: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_path: Option<String>,
}

impl Clip {
    pub fn incoming(&self) -> Incoming {
        Incoming {
            plain: self.plain.clone(),
            html: self.html.clone(),
            rtf: self.rtf.clone(),
            image: self
                .image_path
                .as_ref()
                .and_then(|p| fs::read(p).ok())
                .filter(|b| !b.is_empty()),
        }
    }

    pub fn has_image(&self) -> bool {
        self.image_path.is_some()
    }
}

impl std::fmt::Debug for Clip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Clip")
            .field("id", &self.id)
            .field("timestamp", &self.timestamp)
            .field("chars", &self.plain.chars().count())
            .field("has_html", &self.html.is_some())
            .field("has_rtf", &self.rtf.is_some())
            .field("has_image", &self.image_path.is_some())
            .finish()
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct ShelfFile {
    items: Vec<Clip>,
}

pub fn load() -> Vec<Clip> {
    load_at(&clipboard_shelf_path())
}

pub fn load_at(path: &Path) -> Vec<Clip> {
    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
    serde_json::from_str::<ShelfFile>(&raw)
        .map(|f| f.items)
        .unwrap_or_default()
}

pub fn count() -> usize {
    load().len()
}

pub fn push(incoming: &Incoming) -> Result<Clip> {
    push_at(&clipboard_shelf_path(), &clipboard_shelf_dir(), incoming)
}

pub fn push_at(path: &Path, img_dir: &Path, incoming: &Incoming) -> Result<Clip> {
    let mut items = load_at(path);
    let id = Uuid::new_v4().to_string();
    let image_path = write_image(img_dir, &id, incoming.image.as_deref())?;
    let clip = Clip {
        id,
        timestamp: utc_stamp(),
        plain: incoming.plain.clone(),
        html: incoming.html.clone(),
        rtf: incoming.rtf.clone(),
        image_path,
    };
    items.insert(0, clip.clone());
    items.truncate(MAX_ITEMS);
    save_at(path, &items)?;
    Ok(clip)
}

fn write_image(dir: &Path, id: &str, bytes: Option<&[u8]>) -> Result<Option<String>> {
    let Some(bytes) = bytes else {
        return Ok(None);
    };
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return Ok(None);
    }
    fs::create_dir_all(dir)?;
    let path = dir.join(format!("{id}.bin"));
    fs::write(&path, bytes)?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

fn save_at(path: &Path, items: &[Clip]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(&ShelfFile {
        items: items.to_vec(),
    })?;
    fs::write(&tmp, body)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn utc_stamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{now}")
}

/// Newest-first rows that match `query`.
pub fn filtered(items: &[Clip], query: &str) -> Vec<Clip> {
    items
        .iter()
        .filter(|c| super::policy::matches_query(&c.plain, query))
        .cloned()
        .collect()
}

pub fn default_shelf_file() -> PathBuf {
    clipboard_shelf_path()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::history_path;
    use crate::paths::xdg_test_lock;

    #[test]
    fn shelf_file_is_not_history_jsonl() {
        assert_ne!(clipboard_shelf_path(), history_path());
        assert!(clipboard_shelf_path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("clipboard-shelf"));
        assert!(!clipboard_shelf_path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("history"));
    }

    #[test]
    fn push_does_not_touch_history_jsonl() {
        let _g = xdg_test_lock();
        let dir = std::env::temp_dir().join(format!("openatat-shelf-{}", Uuid::new_v4()));
        std::env::set_var("XDG_DATA_HOME", &dir);
        let hist = history_path();
        assert!(!hist.exists());
        let shelf = dir.join("openatat/clipboard-shelf.json");
        let imgs = dir.join("openatat/clipboard-shelf");
        let incoming = Incoming {
            plain: "clip-one".into(),
            html: Some("<b>clip-one</b>".into()),
            rtf: None,
            image: None,
        };
        push_at(&shelf, &imgs, &incoming).unwrap();
        assert!(!hist.exists(), "shelf must not create history.jsonl");
        let items = load_at(&shelf);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].plain, "clip-one");
        let raw = std::fs::read_to_string(&shelf).unwrap();
        assert!(!raw.contains("\"entry\""), "{raw}");
        assert!(!raw.contains("\"prompt\""), "{raw}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bounded_and_newest_first() {
        let dir = std::env::temp_dir().join(format!("openatat-shelf-b-{}", Uuid::new_v4()));
        let path = dir.join("clipboard-shelf.json");
        let imgs = dir.join("imgs");
        for i in 0..(MAX_ITEMS + 5) {
            let incoming = Incoming {
                plain: format!("n{i}"),
                html: None,
                rtf: None,
                image: None,
            };
            push_at(&path, &imgs, &incoming).unwrap();
        }
        let items = load_at(&path);
        assert_eq!(items.len(), MAX_ITEMS);
        assert_eq!(items[0].plain, format!("n{}", MAX_ITEMS + 4));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn debug_redacts_clip_text() {
        let clip = Clip {
            id: "1".into(),
            timestamp: "0".into(),
            plain: "SECRET_SHELF_TEXT".into(),
            html: None,
            rtf: None,
            image_path: None,
        };
        let dbg = format!("{clip:?}");
        assert!(!dbg.contains("SECRET_SHELF_TEXT"), "{dbg}");
    }
}
