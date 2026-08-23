use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::Mutex;

use openatat_ipc::{DaemonReply, DaemonRequest, TriggerSource};

use crate::a11y;
use crate::error::{Error, Result};
use crate::paths::{runtime_dir, trigger_socket_path};
use crate::selection::{self, SelectionOrigin, SummonDecision};
use crate::session;
use crate::trigger::{Fcitx5Backend, FieldKind, IbusBackend, ImeBackend};

static BUSY: Mutex<bool> = Mutex::new(false);

fn try_enter() -> bool {
    let mut g = BUSY.lock().unwrap_or_else(|e| e.into_inner());
    if *g {
        false
    } else {
        *g = true;
        true
    }
}

fn leave() {
    *BUSY.lock().unwrap_or_else(|e| e.into_inner()) = false;
}

pub fn run_daemon() -> Result<()> {
    let mut fcitx = Fcitx5Backend;
    let mut ibus = IbusBackend;
    let _ = fcitx.start();
    let _ = ibus.start();
    let _ = (fcitx.name(), ibus.name());

    let sock = trigger_socket_path();
    bind_socket(&sock)?;
    start_selection_watcher();
    eprintln!("openatatd: listening on {}", sock.display());
    eprintln!("openatatd: idle — no layer surface, no GPU window");

    let listener = UnixListener::bind(&sock)?;
    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                if let Err(e) = handle_client(stream) {
                    eprintln!("openatatd: client: {e}");
                }
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
        serde_json::to_string(&reply)
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
    DaemonRequest::decode(line).map_err(Error::from)
}

fn dispatch(req: DaemonRequest) -> DaemonReply {
    match req {
        DaemonRequest::Ping => DaemonReply::Ok,
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
                Ok(_) => DaemonReply::Ok,
                Err(e) => DaemonReply::Error {
                    message: e.to_string(),
                },
            };
            leave();
            reply
        }
        DaemonRequest::SelectionProbe => {
            if !try_enter() {
                return DaemonReply::Busy;
            }
            let reply = match run_selection_probe() {
                Ok(_) => DaemonReply::Ok,
                Err(e) => DaemonReply::Error {
                    message: e.to_string(),
                },
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
                let _ = session::run_selection_bar(sel, hit.pointer);
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

pub fn send_trigger() -> Result<()> {
    let path = trigger_socket_path();
    let mut stream = UnixStream::connect(&path).map_err(|e| {
        Error::msg(format!(
            "cannot connect to {} ({e}). Is openatatd running?",
            path.display()
        ))
    })?;
    let req = DaemonRequest::Trigger {
        source: TriggerSource::Demo,
        focus: crate::focus::snapshot(),
    };
    writeln!(stream, "{}", req.encode()?)?;
    let mut reader = BufReader::new(stream);
    let mut reply = String::new();
    reader.read_line(&mut reply)?;
    print!("{reply}");
    Ok(())
}

pub fn send_selection_probe() -> Result<()> {
    let path = trigger_socket_path();
    let mut stream = UnixStream::connect(&path).map_err(|e| {
        Error::msg(format!(
            "cannot connect to {} ({e}). Is openatatd running?",
            path.display()
        ))
    })?;
    let req = DaemonRequest::SelectionProbe;
    writeln!(stream, "{}", req.encode()?)?;
    let mut reader = BufReader::new(stream);
    let mut reply = String::new();
    reader.read_line(&mut reply)?;
    print!("{reply}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_trigger_word_is_demo_source() {
        let req = parse_request("trigger\n").unwrap();
        match req {
            DaemonRequest::Trigger { source, .. } => assert_eq!(source, TriggerSource::Demo),
            _ => panic!("expected trigger"),
        }
    }
}
