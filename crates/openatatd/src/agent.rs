//! P0 dummy CLI. No bundled model. BYO agent is P1 (`OPENATAT_AGENT` / PATH).

use std::io::Write;
use std::process::{Command, Stdio};

use crate::error::{Error, Result};

pub fn run_dummy(prompt: &str) -> Result<String> {
    if let Ok(agent) = std::env::var("OPENATAT_AGENT") {
        return run_agent_cmd(&agent, prompt);
    }
    if which("openatat-agent") {
        return run_agent_cmd("openatat-agent", prompt);
    }
    let out = Command::new("echo")
        .arg(format!("openatat-dummy: {prompt}"))
        .output()?;
    if !out.status.success() {
        return Err(Error::msg("echo dummy agent failed"));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn run_agent_cmd(cmd: &str, prompt: &str) -> Result<String> {
    // Pass the prompt as data on stdin, never interpolated into a shell string.
    let mut child = Command::new(cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(prompt.as_bytes())?;
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        return Err(Error::msg(format!("agent `{cmd}` exited {}", out.status)));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn which(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| {
            std::env::split_paths(&p).any(|dir| {
                let cand = dir.join(name);
                cand.is_file()
            })
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echo_dummy_prefixes_prompt() {
        std::env::remove_var("OPENATAT_AGENT");
        let out = run_dummy("make this friendlier").unwrap();
        assert!(out.contains("make this friendlier"), "{out}");
        assert!(out.contains("openatat-dummy"), "{out}");
    }
}
