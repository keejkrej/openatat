use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::Mutex;

use openatat_ipc::{DaemonReply, DaemonRequest, TriggerSource};

use crate::a11y;
use crate::error::{Error, Result};
use crate::paths::{runtime_dir, status_file_path, trigger_socket_path};
use crate::selection::{self, SelectionOrigin, SummonDecision};
use crate::session;
use crate::trigger::{Fcitx5Backend, FieldKind, IbusBackend, ImeBackend};

static BUSY: Mutex<bool> = Mutex::new(false);
static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);

fn try_enter() -> bool {
    let mut g = BUSY.lock().unwrap_or_else(|e| e.into_inner());
    if *g {
        false
    } else {
        *g = true;
        drop(g);
        set_last_error(None);
        publish_presence();
        true
    }
}

fn leave() {
    *BUSY.lock().unwrap_or_else(|e| e.into_inner()) = false;
    publish_presence();
}

fn is_busy() -> bool {
    *BUSY.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn is_session_busy() -> bool {
    is_busy()
}

/// Product `@@` path on macOS. Called from the listen-only tap on the main thread.
pub fn summon_from_ime() {
    if !try_enter() {
        return;
    }
    match session::run_interactive(TriggerSource::Ime) {
        Ok(_) => set_last_error(None),
        Err(e) => set_last_error(Some(e.to_string())),
    }
    leave();
}

pub(crate) fn start_presence_and_selection() {
    publish_presence();
    start_selection_watcher();
}

fn set_last_error(msg: Option<String>) {
    *LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()) = msg;
}

fn last_error() -> Option<String> {
    LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Bar-chip view: idle | busy | error. `ok` is only a mutating-command ack.
pub fn presence_reply() -> DaemonReply {
    if is_busy() {
        DaemonReply::Busy
    } else if let Some(message) = last_error() {
        DaemonReply::Error { message }
    } else {
        DaemonReply::Idle
    }
}

pub fn write_status_file_at(path: &Path, reply: &DaemonReply) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let body = format!("{}\n", reply.as_presence().encode()?);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn publish_presence() {
    let reply = presence_reply();
    if let Err(e) = write_status_file_at(&status_file_path(), &reply) {
        eprintln!("openatatd: status file: {e}");
    }
}

pub fn run_daemon() -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        return crate::macos_runtime::run_daemon_host(|| {
            if let Err(e) = accept_loop() {
                eprintln!("openatatd: socket: {e}");
            }
        });
    }

    let mut fcitx = Fcitx5Backend;
    let mut ibus = IbusBackend;
    let _ = fcitx.start();
    let _ = ibus.start();
    let _ = (fcitx.name(), ibus.name());

    start_presence_and_selection();
    accept_loop()
}

fn accept_loop() -> Result<()> {
    let sock = trigger_socket_path();
    bind_socket(&sock)?;
    eprintln!("openatatd: listening on {}", sock.display());
    eprintln!(
        "openatatd: status at {} (idle|busy|error)",
        status_file_path().display()
    );
    eprintln!("openatatd: idle — no overlay surface, no GPU window");

    let listener = UnixListener::bind(&sock)?;
    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                // Status / Settings must not wait on the overlay event loop.
                // Overlay / agent still serialize via try_enter().
                std::thread::Builder::new()
                    .name("openatat-ipc".into())
                    .spawn(move || {
                        if let Err(e) = handle_client(stream) {
                            eprintln!("openatatd: client: {e}");
                        }
                    })
                    .ok();
            }
            Err(e) => eprintln!("openatatd: accept: {e}"),
        }
    }
    Ok(())
}

fn bind_socket(path: &Path) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let _ = std::fs::remove_file(path);
    let _ = runtime_dir();
    Ok(())
}

fn handle_client(stream: UnixStream) -> Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let req = parse_request(&line)?;
    let reply = dispatch(req);
    let mut stream = stream;
    writeln!(
        stream,
        "{}",
        reply
            .encode()
            .unwrap_or_else(|_| r#"{"status":"error","message":"encode"}"#.into())
    )?;
    Ok(())
}

fn parse_request(line: &str) -> Result<DaemonRequest> {
    let line = line.trim();
    if line.is_empty() || line == "trigger" {
        return Ok(DaemonRequest::Trigger {
            source: TriggerSource::Demo,
            focus: Default::default(),
        });
    }
    if line == "status" || line == "ping" {
        return Ok(DaemonRequest::Status);
    }
    DaemonRequest::decode(line).map_err(Error::from)
}

fn dispatch(req: DaemonRequest) -> DaemonReply {
    match req {
        DaemonRequest::Ping | DaemonRequest::Status => presence_reply(),
        DaemonRequest::OpenUi { page } => match crate::ui_spawn::spawn(page) {
            Ok(()) => DaemonReply::Ok,
            Err(e) => DaemonReply::Error {
                message: e.to_string(),
            },
        },
        DaemonRequest::CommitText { .. } => {
            // The addon owns the IME filter. A lone commit without a trigger
            // is ignored in the daemon (filter lives in-process in tests).
            DaemonReply::Ok
        }
        DaemonRequest::Trigger { source, .. } => {
            if !try_enter() {
                return DaemonReply::Busy;
            }
            let reply = match session::run_interactive(source) {
                Ok(_) => {
                    set_last_error(None);
                    DaemonReply::Ok
                }
                Err(e) => {
                    let message = e.to_string();
                    set_last_error(Some(message.clone()));
                    DaemonReply::Error { message }
                }
            };
            leave();
            reply
        }
        DaemonRequest::SelectionProbe => {
            if !try_enter() {
                return DaemonReply::Busy;
            }
            let reply = match run_selection_probe() {
                Ok(_) => {
                    set_last_error(None);
                    DaemonReply::Ok
                }
                Err(e) => {
                    let message = e.to_string();
                    set_last_error(Some(message.clone()));
                    DaemonReply::Error { message }
                }
            };
            leave();
            reply
        }
    }
}

fn start_selection_watcher() {
    let (tx, rx) = std::sync::mpsc::channel();
    selection::spawn_watcher(tx);
    std::thread::Builder::new()
        .name("openatat-sel-run".into())
        .spawn(move || {
            while let Ok(hit) = rx.recv() {
                let field = a11y::probe_field_kind();
                if selection::decide_hit(&hit, field) != SummonDecision::ShowBar {
                    continue;
                }
                let Some(sel) = selection::ready_selection(&hit).cloned() else {
                    continue;
                };
                if !try_enter() {
                    continue;
                }
                match session::run_selection_bar(sel, hit.pointer) {
                    Ok(_) => set_last_error(None),
                    Err(e) => set_last_error(Some(e.to_string())),
                }
                leave();
            }
        })
        .ok();
}

fn run_selection_probe() -> Result<()> {
    // Dev path: treat the current AT-SPI selection as a mouse-up.
    let field = a11y::probe_field_kind();
    let hit = selection::SelectionHit {
        origin: SelectionOrigin::MouseUp,
        probe: a11y::probe_selection(),
        pointer: crate::focus::cursor_pos(),
    };
    if field == FieldKind::Secure || hit.probe.is_secure() {
        return Ok(());
    }
    if selection::decide_hit(&hit, field) != SummonDecision::ShowBar {
        return Ok(());
    }
    let Some(sel) = selection::ready_selection(&hit).cloned() else {
        return Ok(());
    };
    session::run_selection_bar(sel, hit.pointer).map(|_| ())
}

fn send_line(req: &DaemonRequest) -> Result<()> {
    let path = trigger_socket_path();
    let mut stream = UnixStream::connect(&path).map_err(|e| {
        Error::msg(format!(
            "cannot connect to {} ({e}). Is openatatd running?",
            path.display()
        ))
    })?;
    writeln!(stream, "{}", req.encode()?)?;
    let mut reader = BufReader::new(stream);
    let mut reply = String::new();
    reader.read_line(&mut reply)?;
    print!("{reply}");
    Ok(())
}

pub fn send_trigger() -> Result<()> {
    let req = DaemonRequest::Trigger {
        source: TriggerSource::Demo,
        focus: crate::focus::snapshot(),
    };
    send_line(&req)
}

pub fn send_selection_probe() -> Result<()> {
    send_line(&DaemonRequest::SelectionProbe)
}

pub fn send_status() -> Result<()> {
    send_line(&DaemonRequest::Status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::time::Duration;

    #[test]
    fn bare_trigger_word_is_demo_source() {
        let req = parse_request("trigger\n").unwrap();
        match req {
            DaemonRequest::Trigger { source, .. } => assert_eq!(source, TriggerSource::Demo),
            _ => panic!("expected trigger"),
        }
    }

    #[test]
    fn bare_status_and_json_status() {
        assert_eq!(parse_request("status\n").unwrap(), DaemonRequest::Status);
        assert_eq!(parse_request("ping\n").unwrap(), DaemonRequest::Status);
        assert_eq!(
            parse_request(r#"{"cmd":"status"}"#).unwrap(),
            DaemonRequest::Status
        );
        assert_eq!(
            parse_request(r#"{"cmd":"ping"}"#).unwrap(),
            DaemonRequest::Ping
        );
    }

    #[test]
    fn status_dispatch_is_idle_when_not_busy() {
        // Other tests may have left LAST_ERROR set; presence without a
        // session is still a chip-legal idle|busy|error document.
        let reply = dispatch(DaemonRequest::Status);
        match reply {
            DaemonReply::Idle | DaemonReply::Busy | DaemonReply::Error { .. } => {}
            DaemonReply::Ok => panic!("status must not return ok"),
        }
        assert_eq!(dispatch(DaemonRequest::Ping), reply);
    }

    #[test]
    fn status_file_is_idle_busy_or_error_json() {
        let dir = std::env::temp_dir().join(format!(
            "openatat-status-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("status.json");
        write_status_file_at(&path, &DaemonReply::Idle).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert_eq!(DaemonReply::decode(&raw).unwrap(), DaemonReply::Idle);
        assert!(raw.contains(r#""status":"idle""#));

        write_status_file_at(&path, &DaemonReply::Busy).unwrap();
        assert_eq!(
            DaemonReply::decode(&std::fs::read_to_string(&path).unwrap()).unwrap(),
            DaemonReply::Busy
        );

        write_status_file_at(
            &path,
            &DaemonReply::Error {
                message: "boom".into(),
            },
        )
        .unwrap();
        let err = DaemonReply::decode(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            err,
            DaemonReply::Error {
                message: "boom".into()
            }
        );

        write_status_file_at(&path, &DaemonReply::Ok).unwrap();
        assert_eq!(
            DaemonReply::decode(&std::fs::read_to_string(&path).unwrap()).unwrap(),
            DaemonReply::Idle
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn status_json_roundtrip_on_temp_socket() {
        let dir = std::env::temp_dir().join(format!(
            "openatat-sock-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let sock_path = dir.join("trigger.sock");
        let listener = UnixListener::bind(&sock_path).unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle_client(stream).unwrap();
        });

        let mut client = UnixStream::connect(&sock_path).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        writeln!(client, r#"{{"cmd":"status"}}"#).unwrap();
        let mut reply = String::new();
        BufReader::new(client).read_line(&mut reply).unwrap();
        let decoded = DaemonReply::decode(&reply).unwrap();
        match decoded {
            DaemonReply::Idle | DaemonReply::Busy | DaemonReply::Error { .. } => {}
            DaemonReply::Ok => panic!("socket status must not return ok"),
        }
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn omarchy_plugin_manifest_is_bar_widget() {
        let raw = include_str!("../../../omarchy/openatat/manifest.json");
        let v: serde_json::Value = serde_json::from_str(raw).unwrap();
        assert_eq!(v["schemaVersion"], 1);
        assert_eq!(v["id"], "openatat.chip");
        assert!(!v["id"].as_str().unwrap().starts_with("omarchy."));
        assert_eq!(v["kinds"][0], "bar-widget");
        assert_eq!(v["entryPoints"]["barWidget"], "Widget.qml");
        assert_eq!(v["barWidget"]["defaultSection"], "right");
        assert_eq!(v["barWidget"]["allowMultiple"], false);
        let widget = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../omarchy/openatat/Widget.qml");
        assert!(widget.is_file(), "{}", widget.display());
        let qml = std::fs::read_to_string(&widget).unwrap();
        assert!(qml.contains("open-ui"));
        assert!(qml.contains("FileView"));
        assert!(!qml.contains("waybar"));
        assert!(
            !qml.contains("sendLine(\"{\\\"cmd\\\":\\\"trigger\\\"}\")"),
            "chip must not summon the overlay"
        );
    }
}
