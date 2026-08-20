use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use openatat_ipc::{EntryPoint, HistoryRecord};
use uuid::Uuid;

use crate::error::Result;
use crate::paths::{data_dir, history_path};

pub fn append_prompt(entry: EntryPoint, prompt: &str) -> Result<HistoryRecord> {
    append_at(&history_path(), entry, prompt)
}

pub fn append_at(path: &Path, entry: EntryPoint, prompt: &str) -> Result<HistoryRecord> {
    let rec = HistoryRecord {
        id: Uuid::new_v4().to_string(),
        timestamp: utc_stamp(),
        entry,
        prompt: prompt.to_string(),
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(f, "{}", serde_json::to_string(&rec)?)?;
    Ok(rec)
}

pub fn default_history_file() -> PathBuf {
    let _ = fs::create_dir_all(data_dir());
    history_path()
}

fn utc_stamp() -> String {
    // Avoid a chrono/time dependency for four fields of local history.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{now}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_four_field_jsonl() {
        let dir = std::env::temp_dir().join(format!("openatat-hist-{}", Uuid::new_v4()));
        let path = dir.join("history.jsonl");
        let rec = append_at(&path, EntryPoint::Demo, "hello").unwrap();
        let line = std::fs::read_to_string(&path).unwrap();
        let parsed: HistoryRecord = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(parsed.id, rec.id);
        assert_eq!(parsed.prompt, "hello");
        assert_eq!(parsed.entry, EntryPoint::Demo);
        let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(v.as_object().unwrap().len(), 4);
        let _ = std::fs::remove_dir_all(dir);
    }
}
