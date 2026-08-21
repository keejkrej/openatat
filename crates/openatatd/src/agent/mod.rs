//! BYO CLI runner. No bundled model.
//!
//! Prompt is passed as data (argv, stdin, or a temp file the template names).
//! Never spliced into a shell string. Launch always uses an OpenAtat scratch
//! cwd (file-manager tiles are not implemented). On failure the prompt is
//! copied to the clipboard before the error is returned.

mod config;
mod providers;
mod refine;
mod template;

pub use config::AgentConfig;
pub use providers::{ProviderKind, REGISTRY};
pub use refine::RefineSession;
pub use template::{CommandTemplate, PromptPass, RenderedCommand};

use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::{Error, Result};
use crate::paths::cache_dir;

use providers::{dummy_template, spec, template_for};
use template::PROMPT_FILE;

/// Screenshot (or later tiles) written into the scratch workspace.
#[derive(Debug, Clone)]
pub enum Attachment {
    Still { png: Vec<u8> },
}

#[derive(Debug, Clone)]
pub struct ResolveContext {
    pub path: Option<OsString>,
    pub openatat_agent: Option<String>,
    pub config: AgentConfig,
}

impl ResolveContext {
    pub fn from_process() -> Self {
        Self {
            path: std::env::var_os("PATH"),
            openatat_agent: std::env::var("OPENATAT_AGENT")
                .ok()
                .filter(|s| !s.is_empty()),
            config: AgentConfig::load(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Resolved {
    pub kind: ProviderKind,
    pub template: CommandTemplate,
    pub dummy_fallback: bool,
}

impl Resolved {
    pub fn display(&self) -> String {
        self.template.display()
    }
}

pub struct Launch<'a> {
    pub prompt: &'a str,
    pub attachments: &'a [Attachment],
    pub resolve: Option<ResolveContext>,
    pub copy_text: fn(&str) -> Result<()>,
}

impl<'a> Launch<'a> {
    pub fn prompt(prompt: &'a str) -> Self {
        Self {
            prompt,
            attachments: &[],
            resolve: None,
            copy_text: crate::clipboard::copy_text,
        }
    }
}

/// Resolve + run. On failure, copy the prompt first so it is never lost.
pub fn run(prompt: &str) -> Result<String> {
    run_launch(&Launch::prompt(prompt))
}

/// P0 name kept as a thin alias so older call sites still compile.
pub fn run_dummy(prompt: &str) -> Result<String> {
    run(prompt)
}

pub fn resolve_selected() -> Resolved {
    resolve(&ResolveContext::from_process())
}

pub fn resolve(ctx: &ResolveContext) -> Resolved {
    if let Some(bin) = ctx.openatat_agent.as_deref() {
        eprintln!("openatatd: OPENATAT_AGENT escape hatch → `{bin}` (prompt on stdin)");
        return Resolved {
            kind: ProviderKind::Custom,
            template: CommandTemplate::from_bin(bin),
            dummy_fallback: false,
        };
    }

    if let Ok(Some(template)) = ctx.config.argv_override() {
        let kind = ctx.config.requested_kind().unwrap_or(ProviderKind::Custom);
        return Resolved {
            kind,
            template,
            dummy_fallback: false,
        };
    }

    if let Some(kind) = ctx.config.requested_kind() {
        if kind == ProviderKind::Dummy {
            return dummy_resolved();
        }
        if kind == ProviderKind::Custom {
            return dummy_resolved_logged("custom provider has no argv/bin in agent.toml");
        }
        if let Some(s) = spec(kind) {
            if let Some(bin) = first_on_path(s.bins, ctx.path.as_deref()) {
                if let Ok(template) = template_for(s, &bin) {
                    return Resolved {
                        kind,
                        template,
                        dummy_fallback: false,
                    };
                }
            }
            return dummy_resolved_logged(&format!(
                "provider `{}` is set but none of {:?} are on PATH",
                kind.as_str(),
                s.bins
            ));
        }
    }

    for s in providers::REGISTRY {
        if let Some(bin) = first_on_path(s.bins, ctx.path.as_deref()) {
            if let Ok(template) = template_for(s, &bin) {
                eprintln!(
                    "openatatd: using provider `{}` (`{}`)",
                    s.kind.as_str(),
                    bin
                );
                return Resolved {
                    kind: s.kind,
                    template,
                    dummy_fallback: false,
                };
            }
        }
    }

    if let Some(bin) = which("openatat-agent", ctx.path.as_deref()) {
        eprintln!("openatatd: using `openatat-agent` on PATH (prompt on stdin)");
        return Resolved {
            kind: ProviderKind::Custom,
            template: CommandTemplate::from_bin(&bin.to_string_lossy()),
            dummy_fallback: false,
        };
    }

    dummy_resolved_logged("no BYO provider on PATH")
}

fn dummy_resolved() -> Resolved {
    Resolved {
        kind: ProviderKind::Dummy,
        template: dummy_template().expect("dummy template"),
        dummy_fallback: true,
    }
}

fn dummy_resolved_logged(why: &str) -> Resolved {
    eprintln!("openatatd: {why}; using echo dummy (no provider installed)");
    dummy_resolved()
}

pub fn run_launch(launch: &Launch<'_>) -> Result<String> {
    match run_launch_inner(launch) {
        Ok(out) => Ok(out),
        Err(e) => {
            if let Err(copy_e) = (launch.copy_text)(launch.prompt) {
                eprintln!("openatatd: failed to copy prompt after agent error ({copy_e})");
            }
            Err(e)
        }
    }
}

fn run_launch_inner(launch: &Launch<'_>) -> Result<String> {
    let owned = launch.resolve.clone().unwrap_or_else(ResolveContext::from_process);
    let resolved = resolve(&owned);
    let scratch = create_scratch()?;
    let prompt = write_attachments(launch.prompt, launch.attachments, &scratch)?;
    let prompt_file = if resolved.template.pass() == PromptPass::File
        || resolved
            .template
            .argv
            .iter()
            .any(|a| a.contains(PROMPT_FILE))
    {
        let path = scratch.join("prompt.txt");
        std::fs::write(&path, prompt.as_bytes())?;
        Some(path)
    } else {
        None
    };
    let rendered = resolved.template.render(&prompt, prompt_file.as_deref())?;
    spawn_rendered(&rendered, &prompt, &scratch)
}

fn write_attachments(prompt: &str, attachments: &[Attachment], scratch: &Path) -> Result<String> {
    let mut out = prompt.to_string();
    for att in attachments {
        match att {
            Attachment::Still { png } => {
                let path = scratch.join("still.png");
                std::fs::write(&path, png)?;
                out.push_str(
                    "\n\n[OpenAtat] Attached screenshot: still.png (working directory).",
                );
            }
        }
    }
    Ok(out)
}

pub fn create_scratch() -> Result<PathBuf> {
    let dir = cache_dir()
        .join("scratch")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn spawn_rendered(rendered: &RenderedCommand, prompt: &str, scratch: &Path) -> Result<String> {
    // Argv only. Never `sh -c` and never interpolate the prompt into a shell line.
    let mut child = Command::new(&rendered.program);
    child
        .args(&rendered.args)
        .current_dir(scratch)
        .env_clear()
        .envs(filtered_env())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = child.spawn().map_err(|e| {
        Error::msg(format!(
            "failed to launch `{}` ({e})",
            rendered.program
        ))
    })?;
    if let Some(mut stdin) = child.stdin.take() {
        if rendered.pass == PromptPass::Stdin {
            stdin.write_all(prompt.as_bytes())?;
        }
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.trim();
        return Err(Error::msg(if err.is_empty() {
            format!(
                "agent `{}` exited {}",
                rendered.program, out.status
            )
        } else {
            format!(
                "agent `{}` exited {}: {err}",
                rendered.program, out.status
            )
        }));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn filtered_env() -> Vec<(OsString, OsString)> {
    const KEEP: &[&str] = &[
        "PATH",
        "HOME",
        "USER",
        "LOGNAME",
        "SHELL",
        "LANG",
        "LANGUAGE",
        "TZ",
        "TERM",
        "COLORTERM",
        "TMPDIR",
        "TMP",
        "TEMP",
        "SSH_AUTH_SOCK",
        "SSH_AGENT_PID",
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XAUTHORITY",
        "DBUS_SESSION_BUS_ADDRESS",
        "XDG_RUNTIME_DIR",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_CACHE_HOME",
        "XDG_STATE_HOME",
        "XDG_CURRENT_DESKTOP",
    ];
    std::env::vars_os()
        .filter(|(key, _)| keep_env_key(key, KEEP))
        .collect()
}

fn keep_env_key(key: &OsStr, keep: &[&str]) -> bool {
    let Some(name) = key.to_str() else {
        return false;
    };
    if name == "PWD" || name == "OPENATAT_AGENT" {
        return false;
    }
    if keep.contains(&name) || name.starts_with("LC_") || name.starts_with("XDG_") {
        return true;
    }
    let u = name.to_ascii_uppercase();
    u.ends_with("_API_KEY")
        || u.ends_with("_TOKEN")
        || u.ends_with("_AUTH")
        || u.ends_with("_ACCESS_TOKEN")
        || u.ends_with("_SECRET")
        || u.starts_with("ANTHROPIC")
        || u.starts_with("OPENAI")
        || u.starts_with("CURSOR")
        || u.starts_with("XAI")
        || u.starts_with("GROK")
        || u.starts_with("HERMES")
        || u.starts_with("OPENCODE")
        || u.starts_with("OPENROUTER")
        || u.starts_with("CLAUDE")
}

pub fn which(name: &str, path: Option<&OsStr>) -> Option<PathBuf> {
    let p = Path::new(name);
    if p.is_absolute() || name.contains('/') {
        return is_executable(p).then(|| p.to_path_buf());
    }
    let path = path?;
    std::env::split_paths(path).find_map(|dir| {
        let cand = dir.join(name);
        is_executable(&cand).then_some(cand)
    })
}

fn first_on_path(bins: &[&str], path: Option<&OsStr>) -> Option<String> {
    for name in bins {
        if let Some(found) = which(name, path) {
            return Some(found.to_string_lossy().into_owned());
        }
    }
    None
}

fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::config::load_from;
    use std::sync::Mutex;
    use uuid::Uuid;

    static COPY_LOCK: Mutex<Option<String>> = Mutex::new(None);

    fn mock_copy(s: &str) -> Result<()> {
        *COPY_LOCK.lock().unwrap() = Some(s.to_string());
        Ok(())
    }

    fn dummy_ctx() -> ResolveContext {
        ResolveContext {
            path: std::env::var_os("PATH"),
            openatat_agent: None,
            config: AgentConfig {
                provider: Some("dummy".into()),
                ..Default::default()
            },
        }
    }

    fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&path, perms).unwrap();
        }
        path
    }

    #[test]
    fn echo_dummy_prefixes_prompt() {
        let out = run_launch(&Launch {
            prompt: "make this friendlier",
            attachments: &[],
            resolve: Some(dummy_ctx()),
            copy_text: mock_copy,
        })
        .unwrap();
        assert!(out.contains("make this friendlier"), "{out}");
        assert!(out.contains("openatat-dummy"), "{out}");
    }

    #[test]
    fn dummy_fallback_when_path_has_no_provider() {
        let resolved = resolve(&ResolveContext {
            path: Some("/nonexistent-openatat-path".into()),
            openatat_agent: None,
            config: AgentConfig::default(),
        });
        assert!(resolved.dummy_fallback);
        assert_eq!(resolved.kind, ProviderKind::Dummy);
        assert_eq!(resolved.template.argv[0], "echo");
    }

    #[test]
    fn no_shell_interpolation_prompt_is_one_argv() {
        let dir = std::env::temp_dir().join(format!("openatat-argv-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = write_script(
            &dir,
            "dump-args",
            "#!/bin/sh\nprintf '%s\\n' \"$1\"\n",
        );
        let prompt = "hello \"quoted\"\n$(whoami); echo pwned";
        let ctx = ResolveContext {
            path: std::env::var_os("PATH"),
            openatat_agent: None,
            config: AgentConfig {
                argv: Some(vec![
                    script.to_string_lossy().into_owned(),
                    "{prompt}".into(),
                ]),
                ..Default::default()
            },
        };
        let out = run_launch(&Launch {
            prompt,
            attachments: &[],
            resolve: Some(ctx),
            copy_text: mock_copy,
        })
        .unwrap();
        assert_eq!(out, prompt);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn openatat_agent_passes_prompt_on_stdin() {
        let dir = std::env::temp_dir().join(format!("openatat-stdin-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = write_script(&dir, "read-stdin", "#!/bin/sh\ncat\n");
        let prompt = "line1\nline2 \"x\"";
        let ctx = ResolveContext {
            path: std::env::var_os("PATH"),
            openatat_agent: Some(script.to_string_lossy().into_owned()),
            config: AgentConfig::default(),
        };
        let out = run_launch(&Launch {
            prompt,
            attachments: &[],
            resolve: Some(ctx),
            copy_text: mock_copy,
        })
        .unwrap();
        assert_eq!(out, prompt);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn launch_uses_scratch_cwd() {
        let dir = std::env::temp_dir().join(format!("openatat-cwd-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = write_script(&dir, "pwd-bin", "#!/bin/sh\npwd\n");
        let ctx = ResolveContext {
            path: std::env::var_os("PATH"),
            openatat_agent: Some(script.to_string_lossy().into_owned()),
            config: AgentConfig::default(),
        };
        let out = run_launch(&Launch {
            prompt: "ignored",
            attachments: &[],
            resolve: Some(ctx),
            copy_text: mock_copy,
        })
        .unwrap();
        assert!(
            out.contains("openatat") && out.contains("scratch"),
            "cwd should be an OpenAtat scratch dir, got {out}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn fail_copies_prompt() {
        *COPY_LOCK.lock().unwrap() = None;
        let dir = std::env::temp_dir().join(format!("openatat-fail-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = write_script(&dir, "boom", "#!/bin/sh\nexit 2\n");
        let prompt = "do not lose this\nsecond line";
        let ctx = ResolveContext {
            path: std::env::var_os("PATH"),
            openatat_agent: Some(script.to_string_lossy().into_owned()),
            config: AgentConfig::default(),
        };
        let err = run_launch(&Launch {
            prompt,
            attachments: &[],
            resolve: Some(ctx),
            copy_text: mock_copy,
        })
        .unwrap_err();
        assert!(err.to_string().contains("exited"), "{err}");
        assert_eq!(COPY_LOCK.lock().unwrap().as_deref(), Some(prompt));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn config_file_selects_dummy() {
        let dir = std::env::temp_dir().join(format!("openatat-toml-{}", Uuid::new_v4()));
        let cfg_dir = dir.join("openatat");
        std::fs::create_dir_all(&cfg_dir).unwrap();
        let path = cfg_dir.join("agent.toml");
        std::fs::write(&path, "provider = \"dummy\"\n").unwrap();
        let cfg = load_from(&path).unwrap();
        let resolved = resolve(&ResolveContext {
            path: Some("/nonexistent".into()),
            openatat_agent: None,
            config: cfg,
        });
        assert_eq!(resolved.kind, ProviderKind::Dummy);
        let _ = std::fs::remove_dir_all(dir);
    }
}
