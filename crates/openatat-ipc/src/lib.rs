//! Shared protocol between `openatatd` (always-on applet) and `openatat-ui`
//! (on-demand gpui-ce). Also used by `fcitx5-openatat` (and a future IBus adapter).
//!
//! Transport for P0 is a newline-delimited JSON unix socket. Nothing here
//! talks to a network.

use serde::{Deserialize, Serialize};

/// Default relative path under `$XDG_RUNTIME_DIR`.
pub const TRIGGER_SOCKET_NAME: &str = "openatat/trigger.sock";

/// How the overlay was summoned. A global hotkey is not a valid source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TriggerSource {
    /// Product path: Fcitx5 / IBus committed-text filter.
    Ime,
    /// Dev-only Wayland / test path. Not the product.
    Demo,
}

/// Where the user was when they typed `@@` (or fired the demo trigger).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FocusSnapshot {
    /// Hyprland `activewindow` address (`0x…`). Compared again at insert time.
    pub window_address: Option<String>,
    pub app_id: Option<String>,
    pub title: Option<String>,
    /// Output name (`DP-1`, …) used for grim `-o`.
    pub output: Option<String>,
}

/// Entry point recorded in local history. P0 only uses `text-field` and `demo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EntryPoint {
    TextField,
    Demo,
    /// Finder / Nautilus — not implemented (Nautilus has no selection D-Bus API).
    FileManager,
    /// Orb click — P2.
    Orb,
}

/// One history row. Exactly these four fields. Never responses or captures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryRecord {
    pub id: String,
    pub timestamp: String,
    pub entry: EntryPoint,
    pub prompt: String,
}

/// Messages the IME addon / `openatatd trigger` send to the applet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum DaemonRequest {
    Trigger {
        source: TriggerSource,
        #[serde(default)]
        focus: FocusSnapshot,
    },
    /// Dev helper: feed committed text without a compositor seat.
    CommitText {
        text: String,
    },
    Ping,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum DaemonReply {
    Ok,
    Busy,
    Error { message: String },
}

impl DaemonRequest {
    pub fn encode(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub fn decode(line: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(line.trim())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_roundtrip() {
        let req = DaemonRequest::Trigger {
            source: TriggerSource::Ime,
            focus: FocusSnapshot {
                window_address: Some("0xabc".into()),
                app_id: Some("kitty".into()),
                title: None,
                output: Some("DP-1".into()),
            },
        };
        let line = req.encode().unwrap();
        assert_eq!(DaemonRequest::decode(&line).unwrap(), req);
    }

    #[test]
    fn history_has_only_four_fields() {
        let rec = HistoryRecord {
            id: "1".into(),
            timestamp: "2026-08-20T00:00:00Z".into(),
            entry: EntryPoint::TextField,
            prompt: "make this friendlier".into(),
        };
        let v = serde_json::to_value(&rec).unwrap();
        let obj = v.as_object().unwrap();
        assert_eq!(obj.len(), 4);
        assert!(obj.contains_key("id"));
        assert!(obj.contains_key("timestamp"));
        assert!(obj.contains_key("entry"));
        assert!(obj.contains_key("prompt"));
    }
}
