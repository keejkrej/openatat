//! Local JSONL history. Only `id`, `timestamp`, `entry`, `prompt`.
//! Never store or display screenshots or agent output.

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use openatat_ipc::HistoryRecord;

pub fn load_newest_first(path: &Path) -> Vec<HistoryRecord> {
    let Ok(file) = fs::File::open(path) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for line in BufReader::new(file).lines() {
        let Ok(line) = line else {
            continue;
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(rec) = serde_json::from_str::<HistoryRecord>(line) {
            rows.push(rec);
        }
    }
    rows.reverse();
    rows
}

pub fn search<'a>(rows: &'a [HistoryRecord], query: &str) -> Vec<&'a HistoryRecord> {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return rows.iter().collect();
    }
    rows.iter()
        .filter(|r| {
            r.prompt.to_ascii_lowercase().contains(&q)
                || r.id.to_ascii_lowercase().contains(&q)
                || r.timestamp.to_ascii_lowercase().contains(&q)
                || entry_label(r).contains(&q)
        })
        .collect()
}

/// Reuse copies/emits the prompt only. Responses were never stored.
pub fn reuse_prompt(rec: &HistoryRecord) -> String {
    rec.prompt.clone()
}

pub fn clear(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut f = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    f.write_all(b"").map_err(|e| e.to_string())?;
    Ok(())
}

/// Human-readable stamp. File stores unix seconds (see `openatatd::history`).
pub fn format_timestamp(raw: &str) -> String {
    let Ok(secs) = raw.parse::<u64>() else {
        return raw.to_string();
    };
    const SECS_PER_DAY: u64 = 86_400;
    let days = secs / SECS_PER_DAY;
    let tod = secs % SECS_PER_DAY;
    let (y, m, d) = civil_from_days(days as i64);
    let hh = tod / 3600;
    let mm = (tod % 3600) / 60;
    let ss = tod % 60;
    format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02} UTC")
}

fn civil_from_days(z: i64) -> (i32, u32, u32) {
    // Howard Hinnant civil_from_days (proleptic Gregorian).
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

pub fn entry_label(rec: &HistoryRecord) -> &'static str {
    match rec.entry {
        openatat_ipc::EntryPoint::TextField => "text-field",
        openatat_ipc::EntryPoint::Demo => "demo",
        openatat_ipc::EntryPoint::FileManager => "file-manager",
        openatat_ipc::EntryPoint::Orb => "orb",
        openatat_ipc::EntryPoint::Handoff => "handoff",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openatat_ipc::EntryPoint;
    use std::path::PathBuf;

    fn tmp_hist() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "openatat-ui-hist-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("history.jsonl")
    }

    fn rec(id: &str, ts: &str, prompt: &str) -> HistoryRecord {
        HistoryRecord {
            id: id.into(),
            timestamp: ts.into(),
            entry: EntryPoint::Demo,
            prompt: prompt.into(),
        }
    }

    #[test]
    fn newest_first_and_search() {
        let path = tmp_hist();
        let mut f = fs::File::create(&path).unwrap();
        for (i, p) in [("1", "alpha"), ("2", "bravo friendlier"), ("3", "charlie")].iter() {
            let r = rec(i, i, p);
            writeln!(f, "{}", serde_json::to_string(&r).unwrap()).unwrap();
        }
        let rows = load_newest_first(&path);
        assert_eq!(
            rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["3", "2", "1"]
        );
        let hits = search(&rows, "friend");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].prompt, "bravo friendlier");
        assert_eq!(reuse_prompt(hits[0]), "bravo friendlier");
        clear(&path).unwrap();
        assert!(load_newest_first(&path).is_empty());
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(path.parent().unwrap());
    }

    #[test]
    fn skips_bad_lines_and_never_has_response_fields() {
        let path = tmp_hist();
        fs::write(
            &path,
            r#"not json
{"id":"a","timestamp":"1","entry":"demo","prompt":"hi"}
{"id":"b","timestamp":"2","entry":"demo","prompt":"there","response":"NO"}
"#,
        )
        .unwrap();
        let rows = load_newest_first(&path);
        // Extra keys are ignored by serde by default; both valid objects load.
        assert_eq!(rows.len(), 2);
        for r in &rows {
            let v = serde_json::to_value(r).unwrap();
            let obj = v.as_object().unwrap();
            assert_eq!(obj.len(), 4);
            assert!(!obj.contains_key("response"));
        }
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(path.parent().unwrap());
    }

    #[test]
    fn formats_unix_stamp() {
        assert_eq!(format_timestamp("0"), "1970-01-01 00:00:00 UTC");
        assert_eq!(format_timestamp("not-a-number"), "not-a-number");
    }
}
