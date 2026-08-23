//! Always-on clipboard watch. CPU only. Never logs clip contents.
//!
//! Linux reads `wlr-data-control` / `ext-data-control` through
//! `wl-clipboard-rs`. macOS uses `NSPasteboard` changeCount. Windows polls
//! the clipboard sequence number (no extra activating HWND).

use std::thread;
use std::time::Duration;

use super::policy::{decide_record, Incoming, RecordDecision};
use super::store;
use crate::a11y;
use crate::agent::AgentConfig;
use crate::trigger::FieldKind;

const POLL_MS: u64 = 400;

pub fn spawn_watcher() {
    if thread::Builder::new()
        .name("openatat-clip".into())
        .spawn(watch_loop)
        .is_err()
    {
        eprintln!("openatatd: clipboard watch thread failed to start");
        return;
    }
    eprintln!("openatatd: clipboard watch started (CPU poll; contents never logged)");
}

fn watch_loop() {
    let mut last_fp: Option<u64> = None;
    let mut last_kept: Option<Incoming> = None;
    loop {
        thread::sleep(Duration::from_millis(POLL_MS));
        let Some(incoming) = crate::clipboard::read_offer() else {
            continue;
        };
        let fp = incoming.fingerprint();
        if last_fp == Some(fp) {
            continue;
        }
        last_fp = Some(fp);

        let field = a11y::probe_field_kind();
        let recording = AgentConfig::load().clipboard_shelf_enabled();
        match decide_record(recording, field, &incoming, last_kept.as_ref()) {
            RecordDecision::Store => {
                if store::push(&incoming).is_ok() {
                    last_kept = Some(incoming);
                }
            }
            RecordDecision::SkipSecure => {
                // Drop the payload. Do not keep a password in `last_kept`.
                last_kept = None;
            }
            RecordDecision::SkipDuplicate => {
                last_kept = Some(incoming);
            }
            RecordDecision::SkipRecordingOff | RecordDecision::SkipEmpty => {}
        }
    }
}

/// One-shot apply used by tests. Never prints clip text.
pub fn apply_copy(
    recording: bool,
    field: FieldKind,
    incoming: Incoming,
    last: Option<&Incoming>,
    push: impl FnOnce(&Incoming),
) -> RecordDecision {
    let d = decide_record(recording, field, &incoming, last);
    if d == RecordDecision::Store {
        push(&incoming);
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shelf::policy::Incoming;
    use std::sync::Mutex;

    #[test]
    fn apply_copy_password_does_not_push() {
        let pushed = Mutex::new(None);
        let incoming = Incoming {
            plain: "hunter2".into(),
            html: None,
            rtf: None,
            image: None,
        };
        let d = apply_copy(true, FieldKind::Secure, incoming, None, |c| {
            *pushed.lock().unwrap() = Some(c.plain.clone());
        });
        assert_eq!(d, RecordDecision::SkipSecure);
        assert!(pushed.lock().unwrap().is_none());
    }

    #[test]
    fn apply_copy_recording_off_does_not_push() {
        let mut hit = false;
        let incoming = Incoming {
            plain: "keep-me-out".into(),
            html: None,
            rtf: None,
            image: None,
        };
        let d = apply_copy(false, FieldKind::AcceptsText, incoming, None, |_| {
            hit = true;
        });
        assert_eq!(d, RecordDecision::SkipRecordingOff);
        assert!(!hit);
    }

    #[test]
    fn watch_source_is_not_a_compositor_bind() {
        let src = include_str!("watch.rs");
        assert!(!src.contains(&["hyprctl ", "keyword bind"].concat()));
        assert!(!src.contains(&["hyprctl ", "dispatch bind"].concat()));
        assert!(!src.contains(&["SetForegroundWindow", "("].concat()));
        assert!(!src.contains(&["gpui", "::"].concat()));
        assert!(!src.contains(&["iced", "::"].concat()));
    }
}
