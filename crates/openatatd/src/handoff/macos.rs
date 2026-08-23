//! NSWorkspace handoff. Prompt is argv / a file, never a shell string.
//!
//! Prefers `NSWorkspace.openApplication` with `OpenConfiguration`
//! (`arguments`, `currentDirectoryURL`). Falls back to `Command` with cwd
//! set on the process. `open -a` is not used for the prompt.

use std::path::Path;
use std::process::{Command, Stdio};

use objc2_app_kit::{NSWorkspace, NSWorkspaceOpenConfiguration};
use objc2_foundation::{NSArray, NSString, NSURL};

use super::HandoffPlan;
use crate::agent;
use crate::error::{Error, Result};

pub fn spawn(plan: &HandoffPlan) -> Result<()> {
    if plan.uses_shell_string() {
        return Err(Error::msg(
            "handoff refused to spawn a shell string (prompt is data)",
        ));
    }
    match spawn_nsworkspace(plan) {
        Ok(()) => Ok(()),
        Err(ns) => match spawn_direct(plan) {
            Ok(()) => {
                eprintln!("openatatd: NSWorkspace open failed ({ns}); spawned argv directly");
                Ok(())
            }
            Err(direct) => Err(Error::msg(format!(
                "failed to launch terminal `{}` ({ns}; exec: {direct})",
                plan.program()
            ))),
        },
    }
}

fn spawn_nsworkspace(plan: &HandoffPlan) -> Result<()> {
    let ws = unsafe { NSWorkspace::sharedWorkspace() };
    let url = app_url(plan).ok_or_else(|| Error::msg("no .app URL for terminal"))?;
    let cfg = NSWorkspaceOpenConfiguration::configuration();
    unsafe {
        cfg.setActivates(true);
        let cwd = NSURL::fileURLWithPath(&NSString::from_str(&plan.cwd.to_string_lossy()));
        cfg.setCurrentDirectoryURL(Some(&cwd));
        if plan.argv.len() > 1 {
            let args: Vec<_> = plan
                .args()
                .iter()
                .map(|a| NSString::from_str(a))
                .collect();
            cfg.setArguments(&NSArray::from_retained_slice(&args));
        }
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let block = block2::RcBlock::new(move |_app: *mut objc2_app_kit::NSRunningApplication, err: *mut objc2_foundation::NSError| {
        if err.is_null() {
            let _ = tx.send(Ok(()));
        } else {
            let msg = unsafe { err.as_ref() }
                .map(|e| e.localizedDescription().to_string())
                .unwrap_or_else(|| "NSWorkspace open failed".into());
            let _ = tx.send(Err(msg));
        }
    });
    unsafe {
        ws.openApplicationAtURL_configuration_completionHandler(&url, &cfg, Some(&block));
    }
    match rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(Error::msg(e)),
        Err(_) => Err(Error::msg("NSWorkspace open timed out")),
    }
}

fn app_url(plan: &HandoffPlan) -> Option<objc2::rc::Retained<NSURL>> {
    let bin = plan.terminal_bin.as_path();
    let app = ancestor_app(bin).or_else(|| macos_bundle_url(plan.terminal.as_str()))?;
    Some(NSURL::fileURLWithPath(&NSString::from_str(&app.to_string_lossy())))
}

fn ancestor_app(bin: &Path) -> Option<std::path::PathBuf> {
    let mut cur = bin;
    loop {
        if cur.extension().and_then(|e| e.to_str()) == Some("app") {
            return Some(cur.to_path_buf());
        }
        cur = cur.parent()?;
    }
}

fn macos_bundle_url(kind: &str) -> Option<std::path::PathBuf> {
    super::terminals::macos_bundle_path(kind).map(std::path::PathBuf::from)
}

fn spawn_direct(plan: &HandoffPlan) -> Result<()> {
    let mut cmd = Command::new(plan.program());
    cmd.args(plan.args())
        .current_dir(&plan.cwd)
        .env_clear()
        .envs(agent::filtered_env())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| Error::msg(format!("exec {}: {e}", plan.program())))
}
