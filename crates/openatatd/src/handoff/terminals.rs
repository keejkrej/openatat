//! Terminal detection and argv builders. Flags are from each emulator's docs.
//!
//! Never wrap the inner CLI in `sh -c`. The prompt stays an argv element or
//! a file path.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::agent;

/// Omarchy / Hyprland preference order.
pub const DETECT_ORDER: &[TerminalKind] = &[
    TerminalKind::Ghostty,
    TerminalKind::Kitty,
    TerminalKind::Alacritty,
    TerminalKind::Wezterm,
    TerminalKind::Foot,
    TerminalKind::GnomeTerminal,
    TerminalKind::Xterm,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalKind {
    Ghostty,
    Kitty,
    Alacritty,
    Wezterm,
    Foot,
    GnomeTerminal,
    Xterm,
}

impl TerminalKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ghostty => "ghostty",
            Self::Kitty => "kitty",
            Self::Alacritty => "alacritty",
            Self::Wezterm => "wezterm",
            Self::Foot => "foot",
            Self::GnomeTerminal => "gnome-terminal",
            Self::Xterm => "xterm",
        }
    }

    pub fn bin_names(self) -> &'static [&'static str] {
        match self {
            Self::Ghostty => &["ghostty"],
            Self::Kitty => &["kitty"],
            Self::Alacritty => &["alacritty"],
            Self::Wezterm => &["wezterm"],
            Self::Foot => &["foot"],
            Self::GnomeTerminal => &["gnome-terminal"],
            Self::Xterm => &["xterm"],
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "ghostty" => Some(Self::Ghostty),
            "kitty" => Some(Self::Kitty),
            "alacritty" => Some(Self::Alacritty),
            "wezterm" | "wezterm-gui" => Some(Self::Wezterm),
            "foot" | "footclient" => Some(Self::Foot),
            "gnome-terminal" | "gnome-console" | "kgx" => Some(Self::GnomeTerminal),
            "xterm" => Some(Self::Xterm),
            _ => None,
        }
    }

    /// Build `terminal … cwd-flags … [cli]`. No shell. `cwd` must be absolute.
    pub fn wrap(self, terminal_bin: &Path, cwd: &Path, cli: &[String]) -> Vec<String> {
        let bin = terminal_bin.to_string_lossy().into_owned();
        let cwd = cwd.to_string_lossy().into_owned();
        match self {
            // ghostty(1): `--working-directory=DIR`, `-e` takes the rest as the command.
            // https://man.archlinux.org/man/ghostty.1
            Self::Ghostty => {
                let mut v = vec![bin, format!("--working-directory={cwd}")];
                if !cli.is_empty() {
                    v.push("-e".into());
                    v.extend(cli.iter().cloned());
                }
                v
            }
            // kitty: `--directory` / `-d`. Remaining args are the program.
            // https://sw.kovidgoyal.net/kitty/invocation/
            Self::Kitty => {
                let mut v = vec![bin, "--directory".into(), cwd];
                v.extend(cli.iter().cloned());
                v
            }
            // alacritty: `--working-directory DIR`, `-e` / `--command`.
            Self::Alacritty => {
                let mut v = vec![bin, "--working-directory".into(), cwd];
                if !cli.is_empty() {
                    v.push("-e".into());
                    v.extend(cli.iter().cloned());
                }
                v
            }
            // wezterm start --cwd DIR -- cmd args
            // https://wezterm.org/config/launch.html
            Self::Wezterm => {
                let mut v = vec![bin, "start".into(), "--cwd".into(), cwd];
                if !cli.is_empty() {
                    v.push("--".into());
                    v.extend(cli.iter().cloned());
                }
                v
            }
            // foot(1): `-D` / `--working-directory=DIR`. Trailing args are the command.
            Self::Foot => {
                let mut v = vec![bin, "-D".into(), cwd];
                v.extend(cli.iter().cloned());
                v
            }
            // gnome-terminal --working-directory=DIR -- cmd
            Self::GnomeTerminal => {
                let mut v = vec![bin, format!("--working-directory={cwd}")];
                if !cli.is_empty() {
                    v.push("--".into());
                    v.extend(cli.iter().cloned());
                }
                v
            }
            // xterm has no cwd flag; the spawn sets current_dir. `-e` takes the rest.
            Self::Xterm => {
                let mut v = vec![bin];
                if !cli.is_empty() {
                    v.push("-e".into());
                    v.extend(cli.iter().cloned());
                }
                v
            }
        }
    }
}

pub fn resolve_terminal(
    configured: Option<&str>,
    path: Option<&OsStr>,
) -> Option<(TerminalKind, PathBuf)> {
    if let Some(name) = configured {
        if let Some(found) = which_named(name, path) {
            return Some(found);
        }
        eprintln!(
            "openatatd: handoff.terminal = `{name}` is not on PATH; trying Omarchy order"
        );
    }
    which_terminal(None, path)
}

pub fn which_terminal(
    configured: Option<&str>,
    path: Option<&OsStr>,
) -> Option<(TerminalKind, PathBuf)> {
    if let Some(name) = configured {
        return which_named(name, path);
    }
    for kind in DETECT_ORDER {
        if let Some(bin) = first_bin(kind.bin_names(), path) {
            return Some((*kind, bin));
        }
    }
    None
}

fn which_named(name: &str, path: Option<&OsStr>) -> Option<(TerminalKind, PathBuf)> {
    let kind = TerminalKind::parse(name).or_else(|| {
        Path::new(name)
            .file_name()
            .and_then(|s| s.to_str())
            .and_then(TerminalKind::parse)
    })?;
    let names: Vec<&str> = if name.contains('/') {
        vec![name]
    } else {
        kind.bin_names().to_vec()
    };
    first_bin(&names, path).map(|bin| (kind, bin))
}

fn first_bin(names: &[&str], path: Option<&OsStr>) -> Option<PathBuf> {
    for n in names {
        if let Some(p) = agent::which(n, path) {
            return Some(p);
        }
    }
    None
}
