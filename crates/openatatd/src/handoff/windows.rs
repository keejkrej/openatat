//! `CreateProcessW` handoff. Prompt is argv / a file, never `cmd.exe /c`.
//!
//! Prefers Windows Terminal (`wt.exe -d <scratch> -- <cli>`). cwd is the
//! scratch directory on the process (`lpCurrentDirectory`). Never splices
//! the prompt into a shell string.

use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};

use super::HandoffPlan;
use crate::agent;
use crate::error::{Error, Result};

/// CREATE_NEW_PROCESS_GROUP | CREATE_NEW_CONSOLE | DETACHED_PROCESS bits we
/// actually want: a new console so the terminal is visible, no `cmd /c`.
const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
const CREATE_UNICODE_ENVIRONMENT: u32 = 0x0000_0400;

pub fn spawn(plan: &HandoffPlan) -> Result<()> {
    if plan.uses_shell_string() {
        return Err(Error::msg(
            "handoff refused to spawn a shell string (prompt is data)",
        ));
    }
    if argv_looks_like_cmd_c(&plan.argv) {
        return Err(Error::msg(
            "handoff refused cmd.exe /c or powershell -Command (prompt is data)",
        ));
    }
    spawn_create_process(plan)
}

pub fn argv_looks_like_cmd_c(argv: &[String]) -> bool {
    argv.windows(2).any(|w| {
        let prog = std::path::Path::new(&w[0])
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&w[0])
            .to_ascii_lowercase();
        let flag = w[1].to_ascii_lowercase();
        matches!(prog.as_str(), "cmd" | "cmd.exe") && matches!(flag.as_str(), "/c" | "-c")
            || matches!(prog.as_str(), "powershell" | "powershell.exe" | "pwsh" | "pwsh.exe")
                && (flag == "-command" || flag == "-c" || flag == "/c")
    })
}

fn spawn_create_process(plan: &HandoffPlan) -> Result<()> {
    let mut cmd = Command::new(plan.program());
    cmd.args(plan.args())
        .current_dir(&plan.cwd)
        .env_clear()
        .envs(agent::filtered_env())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NEW_CONSOLE | CREATE_UNICODE_ENVIRONMENT);
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| Error::msg(format!("CreateProcessW {}: {e}", plan.program())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_cmd_c_and_powershell_command() {
        assert!(argv_looks_like_cmd_c(&[
            "cmd.exe".into(),
            "/c".into(),
            "echo pwned".into()
        ]));
        assert!(argv_looks_like_cmd_c(&[
            "powershell.exe".into(),
            "-Command".into(),
            "echo pwned".into()
        ]));
        assert!(!argv_looks_like_cmd_c(&[
            "wt.exe".into(),
            "-d".into(),
            "C:\\\\scratch".into(),
            "--".into(),
            "claude".into(),
            "hello".into()
        ]));
    }
}
