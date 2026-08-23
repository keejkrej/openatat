//! Shared protocol between `openatatd` (always-on applet) and `openatat-ui`
//! (on-demand gpui-ce). Also used by `fcitx5-openatat` (and a future IBus adapter).
//!
//! Transport for P0 is a newline-delimited JSON unix socket. Nothing here
//! talks to a network.

use serde::{Deserialize, Serialize};

/// Default relative path under `$XDG_RUNTIME_DIR`.
pub const TRIGGER_SOCKET_NAME: &str = "openatat/trigger.sock";

/// Presence file the Omarchy Quickshell bar chip watches (same runtime dir).
pub const STATUS_FILE_NAME: &str = "openatat/status.json";

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
    /// macOS frontmost pid. Unused on Linux.
    #[serde(default)]
    pub pid: Option<i32>,
    /// macOS focused AX element identity. Unused on Linux.
    #[serde(default)]
    pub element_id: Option<String>,
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
    /// Super+Return / Handoff opened the user's terminal.
    Handoff,
}

/// One history row. Exactly these four fields. Never responses or captures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryRecord {
    pub id: String,
    pub timestamp: String,
    pub entry: EntryPoint,
    pub prompt: String,
}

/// Which `openatat-ui` surface the daemon should spawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum UiPage {
    #[default]
    Settings,
    History,
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
    /// Spawn `openatat-ui` (Settings / History). Not a global hotkey.
    OpenUi {
        #[serde(default)]
        page: UiPage,
    },
    /// Dev: probe the focused AT-SPI selection as a mouse-up. Not a hotkey.
    SelectionProbe,
    /// Bar-chip presence. Does not summon the overlay or take the session lock.
    Status,
    Ping,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum DaemonReply {
    Ok,
    /// Presence: daemon up, no overlay / agent session.
    Idle,
    Busy,
    Error {
        message: String,
    },
}

impl DaemonRequest {
    pub fn encode(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub fn decode(line: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(line.trim())
    }
}

impl DaemonReply {
    pub fn encode(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub fn decode(line: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(line.trim())
    }

    /// Chip / `status.json` view: idle | busy | error (never `ok`).
    pub fn as_presence(&self) -> Self {
        match self {
            Self::Ok => Self::Idle,
            other => other.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_snapshot_old_json_without_mac_fields() {
        let v: FocusSnapshot = serde_json::from_str(r#"{"window_address":"0x1"}"#).unwrap();
        assert_eq!(v.window_address.as_deref(), Some("0x1"));
        assert_eq!(v.pid, None);
        assert_eq!(v.element_id, None);
    }

    #[test]
    fn trigger_roundtrip() {
        let req = DaemonRequest::Trigger {
            source: TriggerSource::Ime,
            focus: FocusSnapshot {
                window_address: Some("0xabc".into()),
                app_id: Some("kitty".into()),
                title: None,
                output: Some("DP-1".into()),
                ..FocusSnapshot::default()
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

    #[test]
    fn open_ui_roundtrip() {
        let req = DaemonRequest::OpenUi {
            page: UiPage::History,
        };
        let line = req.encode().unwrap();
        assert_eq!(DaemonRequest::decode(&line).unwrap(), req);
        assert!(line.contains("open-ui"));
        assert!(line.contains("history"));
    }

    #[test]
    fn handoff_entry_roundtrip() {
        let rec = HistoryRecord {
            id: "2".into(),
            timestamp: "2026-08-23T00:00:00Z".into(),
            entry: EntryPoint::Handoff,
            prompt: "continue in the terminal".into(),
        };
        let v = serde_json::to_value(&rec).unwrap();
        assert_eq!(v["entry"], "handoff");
        assert_eq!(v.as_object().unwrap().len(), 4);
    }

    #[test]
    fn selection_probe_roundtrip() {
        let line = DaemonRequest::SelectionProbe.encode().unwrap();
        assert_eq!(
            DaemonRequest::decode(&line).unwrap(),
            DaemonRequest::SelectionProbe
        );
        assert!(line.contains("selection-probe"));
    }

    #[test]
    fn status_request_roundtrip() {
        let line = DaemonRequest::Status.encode().unwrap();
        assert_eq!(DaemonRequest::decode(&line).unwrap(), DaemonRequest::Status);
        assert_eq!(line, r#"{"cmd":"status"}"#);
        let ping = DaemonRequest::Ping.encode().unwrap();
        assert_eq!(DaemonRequest::decode(&ping).unwrap(), DaemonRequest::Ping);
        assert_eq!(ping, r#"{"cmd":"ping"}"#);
    }

    #[test]
    fn presence_reply_is_idle_busy_error() {
        assert_eq!(DaemonReply::Idle.encode().unwrap(), r#"{"status":"idle"}"#);
        assert_eq!(DaemonReply::Busy.encode().unwrap(), r#"{"status":"busy"}"#);
        let err = DaemonReply::Error {
            message: "agent failed".into(),
        };
        let line = err.encode().unwrap();
        assert_eq!(DaemonReply::decode(&line).unwrap(), err);
        assert!(line.contains("\"status\":\"error\""));
        assert_eq!(DaemonReply::Ok.as_presence(), DaemonReply::Idle);
        assert_eq!(
            DaemonReply::decode(r#"{"status":"idle"}"#).unwrap(),
            DaemonReply::Idle
        );
    }
}
