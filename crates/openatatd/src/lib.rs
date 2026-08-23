//! Always-on native applet. Owns the overlay, trigger, capture, and insert.
//! Idle path maps no Wayland surface and starts no gpui window.

pub mod agent;
pub mod capture;
pub mod clipboard;
pub mod daemon;
pub mod error;
pub mod focus;
pub mod history;
pub mod insert;
pub mod overlay;
pub mod paths;
pub mod session;
pub mod trigger;
pub mod ui_spawn;

use crate::error::Result;
use openatat_ipc::{TriggerSource, UiPage};

#[derive(Debug, Clone)]
pub struct Cli {
    pub once: bool,
    pub headless: bool,
    pub trigger_client: bool,
    pub open_ui: Option<UiPage>,
    pub prompt: Option<String>,
}

impl Cli {
    pub fn parse(args: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        let mut cli = Cli {
            once: false,
            headless: false,
            trigger_client: false,
            open_ui: None,
            prompt: None,
        };
        for arg in args {
            match arg.as_ref() {
                "--demo" | "--once" => cli.once = true,
                "--headless" => {
                    cli.headless = true;
                    cli.once = true;
                }
                "trigger" => cli.trigger_client = true,
                "--settings" => cli.open_ui = Some(UiPage::Settings),
                "--history" => cli.open_ui = Some(UiPage::History),
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                other => {
                    if let Some(p) = other.strip_prefix("--prompt=") {
                        cli.prompt = Some(p.to_string());
                    }
                }
            }
        }
        if std::env::var_os("OPENATAT_DEMO").is_some() {
            cli.once = true;
        }
        cli
    }
}

pub fn print_help() {
    print!(
        "\
openatatd — native OpenAtat applet

USAGE:
  openatatd                 Always-on daemon (unix trigger socket)
  openatatd --demo          One interactive session, then exit
  openatatd --headless      One session without a layer surface
  openatatd trigger         Ping a running daemon (dev path, not a hotkey)
  openatatd --settings      Spawn openatat-ui Settings (activating; not the overlay)
  openatatd --history       Spawn openatat-ui History

The product trigger is the Fcitx5 addon (ime/fcitx5-openatat), not a global bind.
See SPEC.md §6 and ime/fcitx5-openatat/README.md.
"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_demo_and_prompt() {
        let cli = Cli::parse(["--demo", "--prompt=hi"]);
        assert!(cli.once);
        assert_eq!(cli.prompt.as_deref(), Some("hi"));
        assert!(!cli.trigger_client);
        assert!(cli.open_ui.is_none());
    }

    #[test]
    fn parse_settings_spawn() {
        let cli = Cli::parse(["--settings"]);
        assert_eq!(cli.open_ui, Some(UiPage::Settings));
        let cli = Cli::parse(["--history"]);
        assert_eq!(cli.open_ui, Some(UiPage::History));
    }
}

pub fn run(cli: Cli) -> Result<()> {
    if let Some(page) = cli.open_ui {
        return ui_spawn::spawn(page);
    }
    if cli.trigger_client {
        return daemon::send_trigger();
    }
    if cli.once {
        let source = TriggerSource::Demo;
        let prompt = cli
            .prompt
            .or_else(|| std::env::var("OPENATAT_PROMPT").ok())
            .unwrap_or_else(|| "hello from openatat".into());
        if cli.headless || std::env::var_os("WAYLAND_DISPLAY").is_none() {
            let end = session::run_headless(source, &prompt)?;
            eprintln!("openatatd: session ended: {end:?}");
            return Ok(());
        }
        let end = session::run_interactive(source)?;
        eprintln!("openatatd: session ended: {end:?}");
        return Ok(());
    }
    daemon::run_daemon()
}
