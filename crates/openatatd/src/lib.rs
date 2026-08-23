//! Always-on native applet. Owns the overlay, trigger, capture, insert,
//! and the C10 selection bar. Idle path maps no Wayland surface and starts
//! no gpui window.

pub mod a11y;
pub mod agent;
pub mod capture;
pub mod clipboard;
pub mod daemon;
pub mod error;
pub mod finder;
pub mod focus;
pub mod handoff;
pub mod history;
pub mod insert;
#[cfg(target_os = "macos")]
pub mod macos_runtime;
#[cfg(target_os = "windows")]
pub mod windows_runtime;
pub mod overlay;
pub mod paths;
pub mod platform;
pub mod selection;
pub mod session;
pub mod trigger;
pub mod ui_spawn;

use crate::error::Result;
use crate::selection::PromptAction;
use openatat_ipc::{TriggerSource, UiPage};

#[derive(Debug, Clone)]
pub struct Cli {
    pub once: bool,
    pub headless: bool,
    pub trigger_client: bool,
    pub selection_client: bool,
    pub status_client: bool,
    pub open_ui: Option<UiPage>,
    pub prompt: Option<String>,
    pub selection_text: Option<String>,
    pub action: Option<PromptAction>,
}

impl Cli {
    pub fn parse(args: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        let mut cli = Cli {
            once: false,
            headless: false,
            trigger_client: false,
            selection_client: false,
            status_client: false,
            open_ui: None,
            prompt: None,
            selection_text: None,
            action: None,
        };
        for arg in args {
            match arg.as_ref() {
                "--demo" | "--once" => cli.once = true,
                "--headless" => {
                    cli.headless = true;
                    cli.once = true;
                }
                "trigger" => cli.trigger_client = true,
                "selection" => cli.selection_client = true,
                "status" | "ping" => cli.status_client = true,
                "--settings" => cli.open_ui = Some(UiPage::Settings),
                "--history" => cli.open_ui = Some(UiPage::History),
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                other => {
                    if let Some(p) = other.strip_prefix("--prompt=") {
                        cli.prompt = Some(p.to_string());
                    } else if let Some(p) = other.strip_prefix("--selection=") {
                        cli.selection_text = Some(p.to_string());
                    } else if let Some(p) = other.strip_prefix("--action=") {
                        cli.action = parse_action(p);
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
  openatatd trigger         Summon a running daemon (dev path, not a hotkey)
  openatatd status          Presence for the Omarchy bar chip (idle|busy|error)
  openatatd selection       Probe the focused AT-SPI selection as a mouse-up
  openatatd --settings      Spawn openatat-ui Settings (activating; not the overlay)
  openatatd --history       Spawn openatat-ui History

Linux product trigger: Fcitx5 addon (ime/fcitx5-openatat), not a global bind.
macOS product trigger: listen-only CGEvent tap → ImeFilter (Input Monitoring optional).
Windows product trigger: process-local keyboard hook / Raw Input → ImeFilter (not a hotkey).
The overlay is native (layer-shell / NSPanel / WS_EX_NOACTIVATE), not gpui. See SPEC.md.
"
    );
}

fn parse_action(s: &str) -> Option<PromptAction> {
    match s {
        "ask" => Some(PromptAction::Ask),
        "summarize" => Some(PromptAction::Summarize),
        "explain" => Some(PromptAction::Explain),
        _ => None,
    }
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
        assert!(!cli.selection_client);
        assert!(!cli.status_client);
        assert!(cli.open_ui.is_none());
    }

    #[test]
    fn parse_status_client() {
        let cli = Cli::parse(["status"]);
        assert!(cli.status_client);
        let cli = Cli::parse(["ping"]);
        assert!(cli.status_client);
    }

    #[test]
    fn parse_selection_action() {
        let cli = Cli::parse(["--headless", "--selection=hello", "--action=summarize"]);
        assert_eq!(cli.selection_text.as_deref(), Some("hello"));
        assert_eq!(cli.action, Some(PromptAction::Summarize));
        assert!(cli.headless);
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
    if cli.selection_client {
        return daemon::send_selection_probe();
    }
    if cli.status_client {
        return daemon::send_status();
    }
    if let Some(selected) = cli.selection_text.clone() {
        let action = cli.action.unwrap_or(PromptAction::Ask);
        let prompt = cli.prompt.clone().unwrap_or_default();
        if cli.headless || !platform::has_overlay_display() {
            let end = session::run_headless_selection(action, &selected, &prompt)?;
            eprintln!("openatatd: session ended: {end:?}");
            return Ok(());
        }
        let end_off = selected.len() as i32;
        let sel = crate::a11y::TextSelection::from_parts(selected, 0, end_off, None, None);
        let end = session::run_selection_bar(sel, None)?;
        eprintln!("openatatd: session ended: {end:?}");
        return Ok(());
    }
    if cli.once {
        let source = TriggerSource::Demo;
        let prompt = cli
            .prompt
            .or_else(|| std::env::var("OPENATAT_PROMPT").ok())
            .unwrap_or_else(|| "hello from openatat".into());
        if cli.headless || !platform::has_overlay_display() {
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
