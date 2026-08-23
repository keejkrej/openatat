//! On-demand Settings / History / Studio process.
//!
//! Business logic lives here and is tested without a display. The gpui-ce
//! window is behind the `gpui` feature so `cargo test --workspace` does not
//! pull a GPU stack into applet tests.

pub mod history;
pub mod paths;
pub mod permissions;
pub mod providers;
pub mod settings;
pub mod studio;

#[cfg(feature = "gpui")]
mod field;
#[cfg(feature = "gpui")]
pub mod studio_ui;
#[cfg(feature = "gpui")]
pub mod ui;

use std::path::PathBuf;

use openatat_ipc::UiPage;

pub const HELP: &str = "\
openatat-ui — Settings + History + Studio (gpui-ce, on demand)

USAGE:
  openatat-ui                 Settings (default)
  openatat-ui --settings      Settings + permissions copy
  openatat-ui --history       History browser
  openatat-ui --studio --image <path>
                              C17 annotation (local PNG/JPEG only)
  openatat-ui --help

This process is spawned on demand and quits when the last window closes.
It is not the @@ overlay. Overlay / Orb / trigger / insert / capture stay in openatatd.
Activating this window is fine.

Build the GPU window with: cargo build -p openatat-ui --features gpui
`cargo test --workspace` does not enable that feature.

Linux gpui-ce needs headers: libxkbcommon-dev libwayland-dev libvulkan-dev
(and usually libfontconfig-dev). Runtime: Vulkan + Wayland or X11.

openatatd --settings / --history / --studio (or {\"cmd\":\"open-ui\"} on trigger.sock)
spawns this binary. Overlay Edit is:
  openatat-ui --features gpui -- --studio --image <still.png>
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cli {
    pub page: UiPage,
    pub help: bool,
    pub image: Option<PathBuf>,
}

impl Cli {
    pub fn parse(args: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        let mut cli = Cli {
            page: UiPage::Settings,
            help: false,
            image: None,
        };
        let mut expect_image = false;
        for arg in args {
            let arg = arg.as_ref();
            if expect_image {
                cli.image = Some(PathBuf::from(arg));
                expect_image = false;
                continue;
            }
            match arg {
                "--help" | "-h" => cli.help = true,
                "--settings" | "settings" => cli.page = UiPage::Settings,
                "--history" | "history" => cli.page = UiPage::History,
                "--permissions" | "permissions" => cli.page = UiPage::Settings,
                "--studio" | "studio" => cli.page = UiPage::Studio,
                "--image" => expect_image = true,
                other if let Some(p) = other.strip_prefix("--image=") => {
                    cli.image = Some(PathBuf::from(p));
                }
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
        assert_eq!(Cli::parse(["--studio"]).page, UiPage::Studio);
        assert!(Cli::parse(["--help"]).help);
        let none: [&str; 0] = [];
        assert_eq!(Cli::parse(none).page, UiPage::Settings);
    }

    #[test]
    fn parse_studio_image() {
        let cli = Cli::parse(["--studio", "--image", "/tmp/shot.png"]);
        assert_eq!(cli.page, UiPage::Studio);
        assert_eq!(
            cli.image.as_deref(),
            Some(std::path::Path::new("/tmp/shot.png"))
        );
        let cli = Cli::parse(["--studio", "--image=/var/still.png"]);
        assert_eq!(
            cli.image.as_deref(),
            Some(std::path::Path::new("/var/still.png"))
        );
    }

    #[test]
    fn emit_reuse_is_the_prompt() {
        assert_eq!(emit_reuse("make this friendlier"), "make this friendlier");
    }

    #[test]
    fn studio_lives_in_this_crate_not_the_applet() {
        assert!(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/studio.rs")
            .is_file());
        let src = include_str!("studio.rs");
        assert!(src.contains("C17"));
        assert!(src.contains("never fetches"));
    }
}
