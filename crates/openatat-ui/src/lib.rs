//! On-demand Settings / History process.
//!
//! Business logic lives here and is tested without a display. The gpui-ce
//! window is behind the `gpui` feature so `cargo test --workspace` does not
//! pull a GPU stack into applet tests.

pub mod history;
pub mod paths;
pub mod permissions;
pub mod providers;
pub mod settings;

#[cfg(feature = "gpui")]
mod field;
#[cfg(feature = "gpui")]
pub mod ui;

use openatat_ipc::UiPage;

pub const HELP: &str = "\
openatat-ui — Settings + History (gpui-ce, on demand)

USAGE:
  openatat-ui                 Settings (default)
  openatat-ui --settings      Settings + permissions copy
  openatat-ui --history       History browser
  openatat-ui --help

This process is spawned on demand and quits when the last window closes.
It is not the @@ overlay or the Orb. Overlay / Orb / trigger / insert / capture stay in openatatd.
Activating this window is fine.

Build the GPU window with: cargo build -p openatat-ui --features gpui
`cargo test --workspace` does not enable that feature.

Linux gpui-ce needs headers: libxkbcommon-dev libwayland-dev libvulkan-dev
(and usually libfontconfig-dev). Runtime: Vulkan + Wayland or X11.

openatatd --settings / --history (or {\"cmd\":\"open-ui\"} on trigger.sock)
spawns this binary.
";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cli {
    pub page: UiPage,
    pub help: bool,
}

impl Cli {
    pub fn parse(args: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        let mut cli = Cli {
            page: UiPage::Settings,
            help: false,
        };
        for arg in args {
            match arg.as_ref() {
                "--help" | "-h" => cli.help = true,
                "--settings" | "settings" => cli.page = UiPage::Settings,
                "--history" | "history" => cli.page = UiPage::History,
                "--permissions" | "permissions" => cli.page = UiPage::Settings,
                other => {
                    if other.starts_with('-') {
                        eprintln!("openatat-ui: unknown flag {other} (see --help)");
                    }
                }
            }
        }
        cli
    }
}

/// Copy/emit helper used by Reuse. Callers write this to the clipboard.
pub fn emit_reuse(prompt: &str) -> String {
    prompt.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pages() {
        assert_eq!(Cli::parse(["--history"]).page, UiPage::History);
        assert_eq!(Cli::parse(["--settings"]).page, UiPage::Settings);
        assert!(Cli::parse(["--help"]).help);
        let none: [&str; 0] = [];
        assert_eq!(Cli::parse(none).page, UiPage::Settings);
    }

    #[test]
    fn emit_reuse_is_the_prompt() {
        assert_eq!(emit_reuse("make this friendlier"), "make this friendlier");
    }
}
