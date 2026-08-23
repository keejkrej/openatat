//! The Orb: a nonactivating round avatar owned by `openatatd`.
//!
//! Not gpui, not Quickshell, not Waybar. Overlay stays a separate mapped
//! surface and stays unmapped at idle. Keyboard is none while only the Orb
//! is up. Hide lasts this launch; typed `@@` still works.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::error::Result;
use crate::session::{self, SessionEnd};

pub mod draw;
pub mod policy;
pub mod position;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

pub use policy::{
    apply_drops, auto_attach_c1, click_while_busy_is_ignored, explicit_entry_allowed, OrbDrop,
    OrbOpenKind,
};

static SHOW_ORB: AtomicBool = AtomicBool::new(true);
static STARTED: AtomicBool = AtomicBool::new(false);
static COMMAND: Mutex<OrbCommand> = Mutex::new(OrbCommand::None);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OrbCommand {
    None,
    Hide,
    Show,
    DismissError,
}

pub fn is_shown() -> bool {
    SHOW_ORB.load(Ordering::SeqCst)
}

pub fn hide() {
    SHOW_ORB.store(false, Ordering::SeqCst);
    set_command(OrbCommand::Hide);
}

pub fn show() {
    SHOW_ORB.store(true, Ordering::SeqCst);
    set_command(OrbCommand::Show);
}

pub fn dismiss_error_pill() {
    crate::daemon::clear_last_error();
    set_command(OrbCommand::DismissError);
}

fn set_command(cmd: OrbCommand) {
    *COMMAND.lock().unwrap_or_else(|e| e.into_inner()) = cmd;
}

fn take_command() -> OrbCommand {
    let mut g = COMMAND.lock().unwrap_or_else(|e| e.into_inner());
    let c = *g;
    *g = OrbCommand::None;
    c
}

/// Map the resting Orb. Overlay stays unmapped. No GPU process.
pub fn start() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    SHOW_ORB.store(policy::show_orb_on_launch(), Ordering::SeqCst);
    #[cfg(target_os = "linux")]
    {
        linux::start();
    }
    #[cfg(target_os = "macos")]
    {
        macos::start();
    }
    #[cfg(target_os = "windows")]
    {
        windows::start();
    }
}

pub fn run_orb_click() -> Result<SessionEnd> {
    if click_while_busy_is_ignored(crate::daemon::is_session_busy()) {
        return Ok(SessionEnd::Cancelled);
    }
    session::run_orb_click()
}

pub fn run_orb_drop(drops: Vec<OrbDrop>) -> Result<SessionEnd> {
    if click_while_busy_is_ignored(crate::daemon::is_session_busy()) {
        return Ok(SessionEnd::Cancelled);
    }
    session::run_orb_drop(drops)
}

pub(crate) fn spawn_orb_click() {
    std::thread::Builder::new()
        .name("openatat-orb-click".into())
        .spawn(|| {
            crate::daemon::summon_orb_click();
        })
        .ok();
}

#[cfg(target_os = "macos")]
pub(crate) fn macos_pump() {
    macos::pump();
}

#[cfg(target_os = "macos")]
pub(crate) fn macos_handle_event(event: &objc2_app_kit::NSEvent) -> bool {
    macos::handle_event(event)
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_pump() {
    windows::pump();
}

pub(crate) fn spawn_orb_drop(drops: Vec<OrbDrop>) {
    std::thread::Builder::new()
        .name("openatat-orb-drop".into())
        .spawn(move || {
            crate::daemon::summon_orb_drop(drops);
        })
        .ok();
}

/// Source-scan contract: overlay / Orb stay native; no gpui, no activation.
#[cfg(test)]
mod source_policy_tests {
    use super::*;

    fn read(rel: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    fn crate_src() -> String {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        walk(&root)
    }

    fn walk(dir: &std::path::Path) -> String {
        let mut out = String::new();
        let Ok(rd) = std::fs::read_dir(dir) else {
            return out;
        };
        for ent in rd.flatten() {
            let p = ent.path();
            if p.is_dir() {
                out.push_str(&walk(&p));
            } else if p.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push_str(&std::fs::read_to_string(&p).unwrap_or_default());
            }
        }
        out
    }

    #[test]
    fn hide_does_not_disable_typed_at_at() {
        hide();
        assert!(!is_shown());
        assert!(policy::typed_trigger_allowed(is_shown()));
        assert!(policy::explicit_entry_allowed(is_shown()));
        show();
        assert!(is_shown());
    }

    #[test]
    fn no_set_foreground_window_in_orb_or_overlay() {
        for rel in [
            "src/orb/linux.rs",
            "src/orb/macos.rs",
            "src/orb/windows.rs",
            "src/overlay/linux.rs",
            "src/overlay/macos.rs",
            "src/overlay/windows.rs",
        ] {
            let src = read(rel);
            assert!(
                !src.contains("SetForegroundWindow("),
                "{rel} must never call SetForegroundWindow"
            );
        }
        let _ = crate_src();
    }

    #[test]
    fn orb_and_overlay_are_not_gpui() {
        let files = [
            "src/orb/linux.rs",
            "src/orb/macos.rs",
            "src/orb/windows.rs",
            "src/orb/draw.rs",
            "src/orb/policy.rs",
            "src/overlay/mod.rs",
            "src/overlay/draw.rs",
            "src/overlay/controller.rs",
            "src/overlay/linux.rs",
            "src/overlay/macos.rs",
            "src/overlay/windows.rs",
        ];
        for rel in files {
            let src = read(rel);
            let code = src.split("mod tests").next().unwrap_or(&src);
            assert!(!code.contains("gpui::"), "{rel} must not use gpui");
            assert!(!code.contains("iced::"), "{rel} must not use iced");
            assert!(!code.contains("ScreenCaptureFrame"), "{rel}");
        }
        let overlay = read("src/overlay/mod.rs");
        assert!(overlay.contains("Not gpui") || overlay.contains("not gpui"));
    }

    #[test]
    fn studio_lives_in_openatat_ui_not_the_applet() {
        assert!(!std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/studio.rs")
            .exists());
        let spawn = read("src/ui_spawn.rs");
        let spawn_code = spawn.split("mod tests").next().unwrap_or(&spawn);
        assert!(spawn_code.contains("--studio"));
        assert!(spawn_code.contains("--image"));
        assert!(spawn_code.contains("never links gpui"));
        assert!(!spawn_code.contains("gpui::"));
        let attach = read("src/studio_attach.rs");
        let attach_code = attach.split("mod tests").next().unwrap_or(&attach);
        assert!(!attach_code.contains("gpui::"));
        assert!(attach_code.contains("spawn"));
    }

    #[test]
    fn linux_orb_keyboard_is_none() {
        let linux = read("src/orb/linux.rs");
        assert!(
            linux.contains("KeyboardInteractivity::None"),
            "idle Orb must not steal the keyboard"
        );
        assert!(
            !linux.contains("KeyboardInteractivity::Exclusive"),
            "Orb must not use Exclusive keyboard"
        );
        assert!(
            !linux.contains("KeyboardInteractivity::OnDemand"),
            "Orb must not use OnDemand — that is the overlay"
        );
    }

    #[test]
    fn quickshell_chip_is_not_the_orb() {
        let qml = read("../../omarchy/openatat/Widget.qml");
        assert!(qml.contains("not the Orb"));
        assert!(!qml.contains("zwlr_layer_shell"));
        assert!(!qml.contains("@—@"));
        assert!(!qml.contains("KeyboardInteractivity"));
    }

    #[test]
    fn windows_orb_uses_noactivate() {
        let win = read("src/orb/windows.rs");
        assert!(win.contains("WS_EX_NOACTIVATE"));
        assert!(win.contains("WS_EX_LAYERED"));
        assert!(win.contains("MA_NOACTIVATE"));
        assert!(!win.contains("SetForegroundWindow("));
    }

    #[test]
    fn macos_orb_is_nonactivating_panel() {
        let mac = read("src/orb/macos.rs");
        assert!(
            mac.contains("NonactivatingPanel")
                || mac.contains("NSWindowStyleMaskNonactivatingPanel")
        );
        assert!(mac.contains("NSPanel"));
    }
}
