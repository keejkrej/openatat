//! Handoff to a real terminal session. Owned by `openatatd`, not gpui.
//!
//! Super+Return / the overlay Handoff action opens the user's terminal in an
//! OpenAtat scratch cwd and starts the same BYO CLI as an *interactive*
//! session (no `--print` / plan-mode / one-shot flags). The prompt is data:
//! one argv element, or a file the template names. Never spliced into `sh -c`.
//!
//! Finder tiles (Mac Automation) are attachments only. cwd is always scratch —
//! never guessed from a window title.

mod terminals;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

pub use terminals::{macos_bundle_path, TerminalKind, DETECT_ORDER, DETECT_ORDER_MAC};

use std::path::{Path, PathBuf};

use crate::agent::{
    self, write_attachments, Attachment, CommandTemplate, ProviderKind, ResolveContext,
};
use crate::error::{Error, Result};
use terminals::resolve_terminal;

/// What we will exec. Tests inspect this without opening a window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffPlan {
    pub terminal: TerminalKind,
    pub terminal_bin: PathBuf,
    /// Full argv for the terminal process. Never a shell line.
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub prompt: String,
    pub prompt_file: PathBuf,
    pub inner: Vec<String>,
}

impl HandoffPlan {
    pub fn program(&self) -> &str {
        self.argv.first().map(String::as_str).unwrap_or("")
    }

    pub fn args(&self) -> &[String] {
        self.argv.get(1..).unwrap_or(&[])
    }

    /// True if any argv element is a shell `-c`/`sh` wrapper. Handoff forbids that.
    pub fn uses_shell_string(&self) -> bool {
        argv_looks_like_shell(&self.argv)
    }
}

pub struct Handoff<'a> {
    pub prompt: &'a str,
    pub attachments: &'a [Attachment],
    pub resolve: Option<ResolveContext>,
    pub copy_text: fn(&str) -> Result<()>,
    /// Injected spawn (tests). `None` uses the platform launcher.
    pub spawn: Option<fn(&HandoffPlan) -> Result<()>>,
}

impl<'a> Handoff<'a> {
    pub fn prompt(prompt: &'a str) -> Self {
        Self {
            prompt,
            attachments: &[],
            resolve: None,
            copy_text: crate::clipboard::copy_text,
            spawn: None,
        }
    }
}

/// Resolve + write scratch + spawn. On failure, copy the prompt first.
pub fn run(handoff: &Handoff<'_>) -> Result<HandoffPlan> {
    match run_inner(handoff) {
        Ok(plan) => Ok(plan),
        Err(e) => {
            if let Err(copy_e) = (handoff.copy_text)(handoff.prompt) {
                eprintln!("openatatd: failed to copy prompt after handoff error ({copy_e})");
            }
            Err(e)
        }
    }
}

fn run_inner(handoff: &Handoff<'_>) -> Result<HandoffPlan> {
    let plan = prepare(handoff)?;
    let spawn = handoff.spawn.unwrap_or(platform_spawn);
    spawn(&plan)?;
    Ok(plan)
}

/// Build the argv + scratch workspace. Does not spawn.
pub fn prepare(handoff: &Handoff<'_>) -> Result<HandoffPlan> {
    let owned = handoff
        .resolve
        .clone()
        .unwrap_or_else(ResolveContext::from_process);
    let resolved = agent::resolve_interactive(&owned);
    let scratch = agent::create_scratch()?;
    let prompt = write_attachments(handoff.prompt, handoff.attachments, &scratch)?;
    let prompt_file = scratch.join("prompt.txt");
    std::fs::write(&prompt_file, prompt.as_bytes())?;

    let inner = inner_cli(&resolved.template, resolved.kind, &prompt, &prompt_file)?;
    let (kind, terminal_bin) = resolve_terminal(owned.config.handoff_terminal(), owned.path.as_deref())
        .ok_or_else(|| {
            Error::msg(
                "no terminal on PATH (tried ghostty, kitty, iterm, alacritty, wezterm, Terminal.app, foot, gnome-terminal, xterm)",
            )
        })?;

    let argv = kind.wrap(&terminal_bin, &scratch, &inner);
    if argv_looks_like_shell(&argv) {
        return Err(Error::msg(
            "handoff refused to build a shell string (prompt is data)",
        ));
    }
    if argv.iter().any(|a| a.contains("sh -c") || a.contains("bash -c")) {
        return Err(Error::msg("handoff refused a shell -c wrapper"));
    }

    Ok(HandoffPlan {
        terminal: kind,
        terminal_bin,
        argv,
        cwd: scratch,
        prompt,
        prompt_file,
        inner,
    })
}

fn inner_cli(
    template: &CommandTemplate,
    kind: ProviderKind,
    prompt: &str,
    prompt_file: &Path,
) -> Result<Vec<String>> {
    if kind == ProviderKind::Dummy {
        // echo is not an interactive session. Open the terminal in scratch;
        // prompt.txt is already written.
        return Ok(Vec::new());
    }
    let rendered = template.render(prompt, Some(prompt_file))?;
    let mut cli = Vec::with_capacity(1 + rendered.args.len());
    cli.push(rendered.program);
    cli.extend(rendered.args);
    Ok(cli)
}

fn argv_looks_like_shell(argv: &[String]) -> bool {
    argv.windows(2).any(|w| {
        let prog = Path::new(&w[0])
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&w[0]);
        matches!(prog, "sh" | "bash" | "zsh" | "fish" | "dash") && w[1] == "-c"
    })
}

fn platform_spawn(plan: &HandoffPlan) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        return linux::spawn(plan);
    }
    #[cfg(target_os = "macos")]
    {
        return macos::spawn(plan);
    }
    #[cfg(target_os = "windows")]
    {
        return windows::spawn(plan);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = plan;
        Err(Error::msg("handoff unsupported on this OS"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentConfig;
    use crate::handoff::terminals::which_terminal;
    use std::ffi::OsString;
    use std::sync::{Mutex, MutexGuard};
    use uuid::Uuid;

    static COPY_LOCK: Mutex<Option<String>> = Mutex::new(None);

    struct CacheHomeGuard {
        old: Option<OsString>,
        _lock: MutexGuard<'static, ()>,
    }

    impl CacheHomeGuard {
        fn set(dir: &Path) -> Self {
            let lock = crate::paths::xdg_test_lock();
            let old = std::env::var_os("XDG_CACHE_HOME");
            std::env::set_var("XDG_CACHE_HOME", dir.join("cache"));
            Self { old, _lock: lock }
        }
    }

    impl Drop for CacheHomeGuard {
        fn drop(&mut self) {
            match &self.old {
                Some(v) => std::env::set_var("XDG_CACHE_HOME", v),
                None => std::env::remove_var("XDG_CACHE_HOME"),
            }
        }
    }

    fn mock_copy(s: &str) -> Result<()> {
        *COPY_LOCK.lock().unwrap() = Some(s.to_string());
        Ok(())
    }

    fn fail_spawn(_: &HandoffPlan) -> Result<()> {
        Err(Error::msg("spawn failed (test)"))
    }

    fn ok_spawn(_: &HandoffPlan) -> Result<()> {
        Ok(())
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

    fn ctx_with_path(dir: &Path, terminal: Option<&str>) -> ResolveContext {
        ResolveContext {
            path: Some(dir.as_os_str().to_os_string()),
            openatat_agent: None,
            config: AgentConfig {
                provider: Some("dummy".into()),
                handoff: terminal.map(|t| crate::agent::HandoffSection {
                    terminal: Some(t.into()),
                }),
                ..Default::default()
            },
        }
    }

    fn seed_terminals(dir: &Path, names: &[&str]) {
        for n in names {
            write_script(dir, n, "#!/bin/sh\nexit 0\n");
        }
    }

    #[test]
    fn prompt_is_not_in_a_shell_string() {
        let dir = std::env::temp_dir().join(format!("openatat-ho-sh-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        seed_terminals(&dir, &["ghostty"]);
        let _env = CacheHomeGuard::set(&dir);
        let prompt = "hello \"quoted\"\n$(whoami); echo pwned && true";
        let plan = prepare(&Handoff {
            prompt,
            attachments: &[],
            resolve: Some(ctx_with_path(&dir, None)),
            copy_text: mock_copy,
            spawn: Some(ok_spawn),
        })
        .unwrap();
        assert!(!plan.uses_shell_string(), "{:?}", plan.argv);
        assert!(
            !plan.argv.iter().any(|a| a.contains("sh -c") || a.contains("bash -c")),
            "{:?}",
            plan.argv
        );
        // Dummy opens a shell in scratch; prompt lives in the file, not argv.
        assert!(!plan.argv.iter().any(|a| a.contains("$(whoami)")));
        assert!(!plan.argv.iter().any(|a| a.contains("echo pwned")));
        let on_disk = std::fs::read_to_string(&plan.prompt_file).unwrap();
        assert_eq!(on_disk, prompt);
        assert_eq!(plan.prompt, prompt);
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(&plan.cwd);
    }

    #[test]
    fn prompt_as_argv_stays_one_element() {
        let dir = std::env::temp_dir().join(format!("openatat-ho-arg-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        seed_terminals(&dir, &["kitty"]);
        write_script(&dir, "claude", "#!/bin/sh\nexit 0\n");
        let _env = CacheHomeGuard::set(&dir);
        let prompt = "fix \"this\"\n$(reboot)";
        let ctx = ResolveContext {
            path: Some(dir.as_os_str().to_os_string()),
            openatat_agent: None,
            config: AgentConfig {
                provider: Some("claude".into()),
                handoff: Some(crate::agent::HandoffSection {
                    terminal: Some("kitty".into()),
                }),
                ..Default::default()
            },
        };
        let plan = prepare(&Handoff {
            prompt,
            attachments: &[],
            resolve: Some(ctx),
            copy_text: mock_copy,
            spawn: Some(ok_spawn),
        })
        .unwrap();
        assert!(!plan.uses_shell_string());
        assert_eq!(plan.inner.last().map(String::as_str), Some(prompt));
        assert_eq!(
            plan.inner.iter().filter(|a| a.contains("$(reboot)")).count(),
            1
        );
        assert!(!plan.argv.iter().any(|a| a.contains("sh -c")));
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(&plan.cwd);
    }

    #[test]
    fn scratch_cwd_is_under_openatat_cache() {
        let dir = std::env::temp_dir().join(format!("openatat-ho-cwd-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        seed_terminals(&dir, &["foot"]);
        let cache = dir.join("cache");
        let _env = CacheHomeGuard::set(&dir);
        let plan = prepare(&Handoff {
            prompt: "go",
            attachments: &[],
            resolve: Some(ctx_with_path(&dir, Some("foot"))),
            copy_text: mock_copy,
            spawn: Some(ok_spawn),
        })
        .unwrap();
        let cwd = plan.cwd.to_string_lossy();
        assert!(
            cwd.contains("openatat") && cwd.contains("scratch"),
            "cwd should be ~/.cache/openatat/scratch/<id>, got {cwd}"
        );
        assert!(plan.cwd.starts_with(cache.join("openatat/scratch")));
        assert!(plan.cwd.is_dir());
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(&plan.cwd);
    }

    #[test]
    fn fail_copies_prompt() {
        *COPY_LOCK.lock().unwrap() = None;
        let dir = std::env::temp_dir().join(format!("openatat-ho-fail-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        seed_terminals(&dir, &["xterm"]);
        let _env = CacheHomeGuard::set(&dir);
        let prompt = "do not lose this\nsecond line";
        let err = run(&Handoff {
            prompt,
            attachments: &[],
            resolve: Some(ctx_with_path(&dir, Some("xterm"))),
            copy_text: mock_copy,
            spawn: Some(fail_spawn),
        })
        .unwrap_err();
        assert!(err.to_string().contains("spawn failed"), "{err}");
        assert_eq!(COPY_LOCK.lock().unwrap().as_deref(), Some(prompt));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn configurable_terminal_is_used() {
        let dir = std::env::temp_dir().join(format!("openatat-ho-term-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        // Both exist; config must win over Omarchy default (ghostty first).
        seed_terminals(&dir, &["ghostty", "kitty", "alacritty"]);
        let _env = CacheHomeGuard::set(&dir);
        let ctx = ctx_with_path(&dir, Some("kitty"));
        let plan = prepare(&Handoff {
            prompt: "hi",
            attachments: &[],
            resolve: Some(ctx),
            copy_text: mock_copy,
            spawn: Some(ok_spawn),
        })
        .unwrap();
        assert_eq!(plan.terminal, TerminalKind::Kitty);
        assert!(
            plan.terminal_bin.ends_with("kitty"),
            "{:?}",
            plan.terminal_bin
        );
        assert_eq!(plan.argv[0], plan.terminal_bin.to_string_lossy());
        assert!(plan.argv.iter().any(|a| a == "--directory"));
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(&plan.cwd);
    }

    #[test]
    fn omarchy_order_picks_first_on_path() {
        let dir = std::env::temp_dir().join(format!("openatat-ho-ord-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        seed_terminals(&dir, &["wezterm", "xterm"]);
        let found = which_terminal(None, Some(dir.as_os_str())).unwrap();
        assert_eq!(found.0, TerminalKind::Wezterm);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn wrap_flags_match_docs() {
        let cwd = Path::new("/tmp/openatat-scratch");
        let cli = vec!["claude".into(), "do it".into()];
        assert_eq!(
            TerminalKind::Ghostty.wrap(Path::new("/bin/ghostty"), cwd, &cli)[1],
            "--working-directory=/tmp/openatat-scratch"
        );
        let kitty = TerminalKind::Kitty.wrap(Path::new("/bin/kitty"), cwd, &cli);
        assert_eq!(kitty[1], "--directory");
        assert_eq!(kitty[2], "/tmp/openatat-scratch");
        let ala = TerminalKind::Alacritty.wrap(Path::new("/bin/alacritty"), cwd, &cli);
        assert_eq!(&ala[1..4], ["--working-directory", "/tmp/openatat-scratch", "-e"]);
        let wez = TerminalKind::Wezterm.wrap(Path::new("/bin/wezterm"), cwd, &cli);
        assert_eq!(&wez[1..5], ["start", "--cwd", "/tmp/openatat-scratch", "--"]);
        let foot = TerminalKind::Foot.wrap(Path::new("/bin/foot"), cwd, &cli);
        assert_eq!(&foot[1..3], ["-D", "/tmp/openatat-scratch"]);
        let gnome = TerminalKind::GnomeTerminal.wrap(Path::new("/bin/gnome-terminal"), cwd, &cli);
        assert_eq!(gnome[1], "--working-directory=/tmp/openatat-scratch");
        assert_eq!(gnome[2], "--");
        let xterm = TerminalKind::Xterm.wrap(Path::new("/bin/xterm"), cwd, &cli);
        assert_eq!(xterm[1], "-e");
        assert!(!xterm.iter().any(|a| a.contains("sh")));
        let iterm = TerminalKind::Iterm.wrap(Path::new("/Applications/iTerm.app/Contents/MacOS/iTerm2"), cwd, &cli);
        assert_eq!(iterm[0].contains("iTerm"), true);
        assert!(!iterm.iter().any(|a| a.contains("sh -c")));
        assert_eq!(iterm.last().map(String::as_str), Some("do it"));
        let term = TerminalKind::TerminalApp.wrap(Path::new("/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal"), cwd, &cli);
        assert_eq!(term.len(), 1, "Terminal.app wrap must not splice the prompt: {term:?}");
    }

    #[test]
    fn macos_bundle_paths_are_absolute_apps() {
        assert_eq!(
            super::terminals::macos_bundle_path("ghostty"),
            Some("/Applications/Ghostty.app")
        );
        assert_eq!(
            super::terminals::macos_bundle_path("iterm"),
            Some("/Applications/iTerm.app")
        );
        assert_eq!(
            super::terminals::macos_bundle_path("terminal"),
            Some("/System/Applications/Utilities/Terminal.app")
        );
        assert!(super::terminals::macos_bundle_path("foot").is_none());
    }

    #[test]
    fn mac_detect_order_ends_with_terminal_app() {
        assert!(super::terminals::DETECT_ORDER_MAC.contains(&TerminalKind::Iterm));
        assert_eq!(
            super::terminals::DETECT_ORDER_MAC.last(),
            Some(&TerminalKind::TerminalApp)
        );
        assert!(!super::terminals::DETECT_ORDER.contains(&TerminalKind::TerminalApp));
    }
}
