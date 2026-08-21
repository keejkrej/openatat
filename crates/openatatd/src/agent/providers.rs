//! Built-in BYO providers. Binaries are looked up on PATH; templates are argv lists.
//!
//! Flags below were taken from each CLI's own docs. If a conservative
//! read-only / print mode is not documented, the template is stdin-or-prompt
//! plus OpenAtat's scratch cwd only — no invented dangerous defaults.

use super::template::CommandTemplate;
use crate::error::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Claude,
    Codex,
    Grok,
    Cursor,
    Pi,
    Hermes,
    OpenCode,
    Custom,
    Dummy,
}

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Grok => "grok",
            Self::Cursor => "cursor",
            Self::Pi => "pi",
            Self::Hermes => "hermes",
            Self::OpenCode => "opencode",
            Self::Custom => "custom",
            Self::Dummy => "dummy",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "claude" | "claude-code" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "grok" | "xai" => Some(Self::Grok),
            "cursor" | "cursor-agent" | "agent" => Some(Self::Cursor),
            "pi" => Some(Self::Pi),
            "hermes" => Some(Self::Hermes),
            "opencode" => Some(Self::OpenCode),
            "custom" => Some(Self::Custom),
            "dummy" | "echo" => Some(Self::Dummy),
            "auto" => None,
            _ => None,
        }
    }
}

pub struct ProviderSpec {
    pub kind: ProviderKind,
    /// PATH names to try, in order. First executable wins.
    pub bins: &'static [&'static str],
    /// argv[0] is replaced with the resolved binary. Placeholders: `{prompt}`, `{prompt_file}`.
    pub argv: &'static [&'static str],
}

/// Product order: Claude Code, Codex, Grok, Cursor, Pi, Hermes, OpenCode.
pub const REGISTRY: &[ProviderSpec] = &[
    // Claude Code: `claude -p` / `--print`. `plan` reads and explores; it does not edit source.
    // https://code.claude.com/docs/en/cli-reference
    ProviderSpec {
        kind: ProviderKind::Claude,
        bins: &["claude"],
        argv: &[
            "claude",
            "--print",
            "--permission-mode",
            "plan",
            "{prompt}",
        ],
    },
    // Codex: `codex exec`. Default sandbox is read-only; we set it explicitly.
    // `-` reads the prompt from stdin. `--ephemeral` skips session files.
    // https://developers.openai.com/codex/noninteractive
    ProviderSpec {
        kind: ProviderKind::Codex,
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
    // Grok Build does not read piped stdin as the prompt. `--prompt-file` does.
    // `--sandbox read-only` is a documented profile.
    // https://docs.x.ai/build/cli/reference
    ProviderSpec {
        kind: ProviderKind::Grok,
        bins: &["grok"],
        argv: &[
            "grok",
            "--sandbox",
            "read-only",
            "--prompt-file",
            "{prompt_file}",
        ],
    },
    // Cursor CLI entrypoints are `agent` and the `cursor-agent` alias (not the
    // `cursor` editor launcher). `--print` is headless; `--mode ask` is read-only;
    // `--trust` skips the untrusted-workspace prompt on a fresh scratch dir.
    // https://cursor.com/docs/cli/using
    ProviderSpec {
        kind: ProviderKind::Cursor,
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
    // Pi: `pi -p` / `--print`. No documented read-only permission flag we will invent.
    // https://github.com/earendil-works/pi
    ProviderSpec {
        kind: ProviderKind::Pi,
        bins: &["pi"],
        argv: &["pi", "--print", "{prompt}"],
    },
    // Hermes: `-z` is the scripted one-shot (final reply on stdout). Prompt is argv data.
    // https://hermes-agent.nousresearch.com/docs/reference/cli-commands
    ProviderSpec {
        kind: ProviderKind::Hermes,
        bins: &["hermes"],
        argv: &["hermes", "-z", "{prompt}"],
    },
    // OpenCode: `opencode run`. Do not pass `--auto` (auto-approves permissions).
    // https://opencode.ai/docs/cli/
    ProviderSpec {
        kind: ProviderKind::OpenCode,
        bins: &["opencode"],
        argv: &["opencode", "run", "{prompt}"],
    },
];

pub fn spec(kind: ProviderKind) -> Option<&'static ProviderSpec> {
    REGISTRY.iter().find(|s| s.kind == kind)
}

pub fn dummy_template() -> Result<CommandTemplate> {
    CommandTemplate::new(["echo", "openatat-dummy: {prompt}"])
}

pub fn template_for(spec: &ProviderSpec, resolved_bin: &str) -> Result<CommandTemplate> {
    let mut argv: Vec<String> = spec.argv.iter().map(|s| (*s).to_string()).collect();
    if let Some(first) = argv.first_mut() {
        *first = resolved_bin.to_string();
    }
    CommandTemplate::new(argv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_order_matches_spec() {
        let names: Vec<_> = REGISTRY.iter().map(|s| s.kind.as_str()).collect();
        assert_eq!(
            names,
            ["claude", "codex", "grok", "cursor", "pi", "hermes", "opencode"]
        );
    }

    #[test]
    fn templates_are_argv_not_shell_lines() {
        for spec in REGISTRY {
            assert!(
                spec.argv.iter().all(|a| !a.contains('|') && !a.contains(';')),
                "{:?} looks like a shell line",
                spec.kind
            );
            assert!(!spec.argv.is_empty());
        }
    }
}
