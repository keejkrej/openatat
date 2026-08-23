//! Provider picker + PATH probe. Mirrors `openatatd::agent::providers` so this
//! crate does not depend on the daemon (and its GPU-free Wayland stack).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Values the Settings picker offers. `dummy` is a daemon fallback, not a pick.
pub const PROVIDER_CHOICES: &[&str] = &[
    "auto", "claude", "codex", "grok", "cursor", "pi", "hermes", "opencode", "custom",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderSpec {
    pub id: &'static str,
    pub bins: &'static [&'static str],
    pub argv: &'static [&'static str],
}

/// Product order and documented default argv (prompt is data, not a shell line).
pub const REGISTRY: &[ProviderSpec] = &[
    ProviderSpec {
        id: "claude",
        bins: &["claude"],
        argv: &["claude", "--print", "--permission-mode", "plan", "{prompt}"],
    },
    ProviderSpec {
        id: "codex",
        bins: &["codex"],
        argv: &[
            "codex",
            "exec",
            "--sandbox",
            "read-only",
            "--ephemeral",
            "-",
        ],
    },
    ProviderSpec {
        id: "grok",
        bins: &["grok"],
        argv: &[
            "grok",
            "--sandbox",
            "read-only",
            "--prompt-file",
            "{prompt_file}",
        ],
    },
    ProviderSpec {
        id: "cursor",
        bins: &["cursor-agent", "agent"],
        argv: &[
            "cursor-agent",
            "--print",
            "--mode",
            "ask",
            "--trust",
            "{prompt}",
        ],
    },
    ProviderSpec {
        id: "pi",
        bins: &["pi"],
        argv: &["pi", "--print", "{prompt}"],
    },
    ProviderSpec {
        id: "hermes",
        bins: &["hermes"],
        argv: &["hermes", "-z", "{prompt}"],
    },
    ProviderSpec {
        id: "opencode",
        bins: &["opencode"],
        argv: &["opencode", "run", "{prompt}"],
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedBinary {
    pub name: String,
    pub path: PathBuf,
}

pub fn spec(id: &str) -> Option<&'static ProviderSpec> {
    REGISTRY.iter().find(|s| s.id == id)
}

pub fn default_argv(provider: &str) -> Vec<String> {
    spec(provider)
        .map(|s| s.argv.iter().map(|a| (*a).to_string()).collect())
        .unwrap_or_default()
}

pub fn normalize_provider(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "auto" => "auto".into(),
        "claude" | "claude-code" => "claude".into(),
        "codex" => "codex".into(),
        "grok" | "xai" => "grok".into(),
        "cursor" | "cursor-agent" | "agent" => "cursor".into(),
        "pi" => "pi".into(),
        "hermes" => "hermes".into(),
        "opencode" => "opencode".into(),
        "custom" => "custom".into(),
        other => other.to_string(),
    }
}

pub fn is_known_choice(id: &str) -> bool {
    PROVIDER_CHOICES.contains(&id)
}

/// PATH names we show in Settings: every documented binary, plus whether it exists.
pub fn detect_on_path(path: Option<&OsStr>) -> Vec<(String, Option<PathBuf>)> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for spec in REGISTRY {
        for name in spec.bins {
            if !seen.insert(*name) {
                continue;
            }
            out.push(((*name).to_string(), which(name, path)));
        }
    }
    out
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

    #[test]
    fn choices_match_product() {
        assert_eq!(
            PROVIDER_CHOICES,
            ["auto", "claude", "codex", "grok", "cursor", "pi", "hermes", "opencode", "custom"]
        );
    }

    #[test]
    fn defaults_are_argv_lists_not_shell_lines() {
        for spec in REGISTRY {
            assert!(
                spec.argv
                    .iter()
                    .all(|a| !a.contains('|') && !a.contains(';')),
                "{} looks like a shell line",
                spec.id
            );
            assert!(!spec.argv.is_empty());
        }
    }

    #[test]
    fn detect_finds_sh() {
        let found = detect_on_path(std::env::var_os("PATH").as_deref());
        assert!(found.iter().any(|(n, _)| n == "claude"));
        let sh = which("sh", std::env::var_os("PATH").as_deref());
        assert!(sh.is_some(), "sh should be on PATH in CI");
    }
}
