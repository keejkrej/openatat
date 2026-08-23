//! Linux spawn. Prefer execve of the terminal with cwd + argv.
//! `hyprctl dispatch exec` is last resort and never receives the prompt.

use std::process::{Command, Stdio};

use super::HandoffPlan;
use crate::agent;
use crate::error::{Error, Result};

pub fn spawn(plan: &HandoffPlan) -> Result<()> {
    match spawn_direct(plan) {
        Ok(()) => Ok(()),
        Err(direct) => match spawn_hyprctl(plan) {
            Ok(()) => {
                eprintln!("openatatd: terminal spawn failed ({direct}); used hyprctl dispatch exec");
                Ok(())
            }
            Err(hypr) => Err(Error::msg(format!(
                "failed to launch terminal `{}` ({direct}; hyprctl: {hypr})",
                plan.program()
            ))),
        },
    }
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
        // New process group so a one-shot `--demo` parent exit does not SIGHUP the TTY.
        cmd.process_group(0);
    }
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| Error::msg(format!("exec {}: {e}", plan.program())))
}

/// Hyprland `dispatch exec` is a shell-ish last resort. The prompt must already
/// live in `prompt.txt` — it is never an argument here.
fn spawn_hyprctl(plan: &HandoffPlan) -> Result<()> {
    if plan.argv.iter().any(|a| a == &plan.prompt) {
        return Err(Error::msg(
            "hyprctl exec refused: prompt would enter a shell string",
        ));
    }
    if agent::which("hyprctl", std::env::var_os("PATH").as_deref()).is_none() {
        return Err(Error::msg("hyprctl not on PATH"));
    }
    // argv tokens only. Hyprland joins them as the exec command; no `sh -c`.
    let status = Command::new("hyprctl")
        .arg("dispatch")
        .arg("exec")
        .arg("--")
        .args(&plan.argv)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .map_err(|e| Error::msg(format!("hyprctl: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::msg(format!("hyprctl dispatch exec exited {status}")))
    }
}
