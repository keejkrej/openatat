//! Fcitx5 / IBus shaped filter. The product trigger is the C++ addon.
//!
//! There is no shippable Rust crate for a Fcitx5 addon (addons are C++).
//! `ime/fcitx5-openatat` is the adapter: it probes the field every key
//! (Fcitx5 Password/Sensitive + optional AT-SPI; never cached), ignores
//! preedit / composing, feeds committed characters into the same
//! [`ImeFilter`] / [`super::DetectionBuffer`] contract, swallows `@@`,
//! and sends `DaemonRequest::Trigger { source: TriggerSource::Ime, .. }`
//! to `$XDG_RUNTIME_DIR/openatat/trigger.sock`.
//!
//! A global Hyprland bind is **not** this module and is not the product.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::DetectionBuffer;

/// Re-probed on every key. Never store this on the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Secure,
    AcceptsText,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImeEvent {
    /// Must be first on every key. Secure → drop and clear.
    Field { kind: FieldKind },
    /// Preedit / composing. Must not fire the trigger or mutate the buffer.
    Compose { preedit: String },
    /// Committed text after IME.
    Commit { text: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImeAction {
    Ignore,
    Continue,
    FireTrigger,
}

/// In-process IME filter used by tests and documented as the addon contract.
/// The C++ copy lives in `ime/fcitx5-openatat/src/openatat_filter.*`.
#[derive(Debug, Default)]
pub struct ImeFilter {
    buffer: DetectionBuffer,
}

impl ImeFilter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn buffer(&self) -> &DetectionBuffer {
        &self.buffer
    }

    pub fn on_event(&mut self, event: ImeEvent) -> ImeAction {
        match event {
            ImeEvent::Field {
                kind: FieldKind::Secure,
            } => {
                self.buffer.clear();
                ImeAction::Ignore
            }
            ImeEvent::Field { .. } => ImeAction::Continue,
            ImeEvent::Compose { .. } => {
                // Do not touch the two-character buffer while composing.
                ImeAction::Ignore
            }
            ImeEvent::Commit { text } => {
                if self.buffer.push_committed(&text) {
                    self.buffer.clear();
                    ImeAction::FireTrigger
                } else {
                    ImeAction::Continue
                }
            }
        }
    }

    /// Canonical per-key sequence: probe field, then compose or commit.
    pub fn on_key(
        &mut self,
        kind: FieldKind,
        composing: bool,
        committed: Option<&str>,
    ) -> ImeAction {
        match self.on_event(ImeEvent::Field { kind }) {
            ImeAction::Ignore => return ImeAction::Ignore,
            other => {
                if composing {
                    return self.on_event(ImeEvent::Compose {
                        preedit: committed.unwrap_or("").to_string(),
                    });
                }
                if let Some(text) = committed {
                    return self.on_event(ImeEvent::Commit {
                        text: text.to_string(),
                    });
                }
                other
            }
        }
    }
}

/// Product backend. The C++ addon talks to the socket; this only detects setup.
pub trait ImeBackend {
    fn name(&self) -> &'static str;
    fn start(&mut self) -> Result<(), String>;
}

/// Detects the installed `fcitx5-openatat` module. Never fails the daemon.
#[derive(Debug, Default)]
pub struct Fcitx5Backend;

impl ImeBackend for Fcitx5Backend {
    fn name(&self) -> &'static str {
        "fcitx5"
    }

    fn start(&mut self) -> Result<(), String> {
        let running = fcitx5_process_running();
        let installed = fcitx5_addon_installed();
        match (running, installed) {
            (true, true) => {
                eprintln!(
                    "openatatd: Fcitx5 addon present — product @@ trigger is the IME filter"
                );
            }
            (true, false) => {
                eprintln!(
                    "openatatd: Fcitx5 is running but fcitx5-openatat is not installed. \
                     Typing @@ in a field will not summon the overlay. \
                     See ime/fcitx5-openatat/README.md"
                );
            }
            (false, true) => {
                eprintln!(
                    "openatatd: fcitx5-openatat is installed; start Fcitx5 so @@ works in text fields"
                );
            }
            (false, false) => {
                eprintln!(
                    "openatatd: Fcitx5 not detected. Dev trigger still works \
                     (`openatatd trigger` / --demo). Product path: ime/fcitx5-openatat"
                );
            }
        }
        Ok(())
    }
}

/// IBus is not the Omarchy path. A thin engine can reuse the same socket later.
#[derive(Debug, Default)]
pub struct IbusBackend;

impl ImeBackend for IbusBackend {
    fn name(&self) -> &'static str {
        "ibus"
    }

    fn start(&mut self) -> Result<(), String> {
        Ok(())
    }
}

pub fn fcitx5_process_running() -> bool {
    if let Ok(status) = Command::new("fcitx5-remote").arg("-n").status() {
        if status.success() {
            return true;
        }
    }
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        let runtime = PathBuf::from(dir);
        if runtime.join("fcitx5").exists() {
            return true;
        }
        if let Ok(rd) = std::fs::read_dir(&runtime) {
            for ent in rd.flatten() {
                let name = ent.file_name();
                let n = name.to_string_lossy();
                if n.starts_with("fcitx5") {
                    return true;
                }
            }
        }
    }
    false
}

pub fn fcitx5_addon_installed() -> bool {
    fcitx5_addon_conf_dirs()
        .into_iter()
        .any(|d| d.join("openatat.conf").is_file())
        || fcitx5_addon_lib_dirs().into_iter().any(|d| {
            d.join("libopenatat.so").is_file() || d.join("openatat.so").is_file()
        })
}

pub fn fcitx5_addon_conf_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        dirs.push(PathBuf::from(xdg).join("fcitx5/addon"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local/share/fcitx5/addon"));
    }
    if let Ok(data_dirs) = std::env::var("XDG_DATA_DIRS") {
        for part in data_dirs.split(':').filter(|s| !s.is_empty()) {
            dirs.push(Path::new(part).join("fcitx5/addon"));
        }
    }
    dirs.push(PathBuf::from("/usr/share/fcitx5/addon"));
    dirs.push(PathBuf::from("/usr/local/share/fcitx5/addon"));
    dirs
}

pub fn fcitx5_addon_lib_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        dirs.push(home.join(".local/lib/fcitx5"));
        dirs.push(home.join(".local/lib64/fcitx5"));
    }
    dirs.push(PathBuf::from("/usr/lib/fcitx5"));
    dirs.push(PathBuf::from("/usr/lib64/fcitx5"));
    dirs.push(PathBuf::from("/usr/local/lib/fcitx5"));
    dirs.push(PathBuf::from("/usr/lib/x86_64-linux-gnu/fcitx5"));
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_does_not_fire_even_if_preedit_is_ats() {
        let mut ime = ImeFilter::new();
        let action = ime.on_key(FieldKind::AcceptsText, true, Some("@@"));
        assert_eq!(action, ImeAction::Ignore);
        assert!(!ime.buffer().is_trigger());
        assert_eq!(ime.buffer().len(), 0);
    }

    #[test]
    fn committed_ats_fire_and_clear() {
        let mut ime = ImeFilter::new();
        assert_eq!(
            ime.on_key(FieldKind::AcceptsText, false, Some("@")),
            ImeAction::Continue
        );
        assert_eq!(
            ime.on_key(FieldKind::AcceptsText, false, Some("@")),
            ImeAction::FireTrigger
        );
        assert_eq!(ime.buffer().len(), 0);
    }

    #[test]
    fn secure_field_skipped_every_key_never_cached() {
        let mut ime = ImeFilter::new();
        assert_eq!(
            ime.on_key(FieldKind::AcceptsText, false, Some("@")),
            ImeAction::Continue
        );
        // Next key is a password field — probe is not cached from the previous key.
        assert_eq!(
            ime.on_key(FieldKind::Secure, false, Some("@")),
            ImeAction::Ignore
        );
        assert_eq!(ime.buffer().len(), 0);
        // Back in a text field, we start over (no leftover `@`).
        assert_eq!(
            ime.on_key(FieldKind::AcceptsText, false, Some("@")),
            ImeAction::Continue
        );
        assert!(!ime.buffer().is_trigger());
    }

    #[test]
    fn compose_then_commit_can_still_trigger() {
        let mut ime = ImeFilter::new();
        assert_eq!(
            ime.on_key(FieldKind::AcceptsText, true, Some("@")),
            ImeAction::Ignore
        );
        assert_eq!(
            ime.on_key(FieldKind::AcceptsText, false, Some("@@")),
            ImeAction::FireTrigger
        );
    }

    #[test]
    fn fcitx5_conf_dirs_include_xdg_data_home() {
        let old = std::env::var_os("XDG_DATA_HOME");
        std::env::set_var("XDG_DATA_HOME", "/tmp/openatat-fcitx-xdg");
        let dirs = fcitx5_addon_conf_dirs();
        assert!(dirs
            .iter()
            .any(|d| d == Path::new("/tmp/openatat-fcitx-xdg/fcitx5/addon")));
        match old {
            Some(v) => std::env::set_var("XDG_DATA_HOME", v),
            None => std::env::remove_var("XDG_DATA_HOME"),
        }
    }

    #[test]
    fn fcitx5_start_never_errors() {
        let mut backend = Fcitx5Backend;
        assert!(backend.start().is_ok());
    }

    #[test]
    fn cpp_filter_state_machine_matches_rust_contract() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ime/fcitx5-openatat");
        // Prefer g++: some images ship a `c++` clang that cannot find libstdc++.
        let cxx = ["g++", "c++", "clang++"].into_iter().find(|c| {
            let probe = std::env::temp_dir().join(format!("openatat-cxx-probe-{}", c));
            let status = Command::new(c)
                .args(["-x", "c++", "-o"])
                .arg(&probe)
                .args(["-", "-lstdc++"])
                .stdin(std::process::Stdio::piped())
                .spawn()
                .and_then(|mut child| {
                    use std::io::Write;
                    if let Some(mut stdin) = child.stdin.take() {
                        let _ = stdin.write_all(b"int main(){return 0;}\n");
                    }
                    child.wait()
                });
            let _ = std::fs::remove_file(&probe);
            status.map(|s| s.success()).unwrap_or(false)
        });
        let Some(cxx) = cxx else {
            eprintln!("skipping C++ filter test: no working C++ compiler");
            return;
        };
        let tmp = std::env::temp_dir().join(format!(
            "openatat-filter-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let compile = Command::new(cxx)
            .arg("-std=c++17")
            .arg("-O0")
            .arg(src.join("src/openatat_filter.cpp"))
            .arg(src.join("src/openatat_filter_test.cpp"))
            .arg(src.join("src/openatat_socket.cpp"))
            .arg("-o")
            .arg(&tmp)
            .status()
            .expect("spawn C++ compiler");
        assert!(
            compile.success(),
            "C++ ImeFilter tests failed to compile (no Fcitx5 required)"
        );
        let run = Command::new(&tmp).status().expect("run C++ filter test");
        let _ = std::fs::remove_file(&tmp);
        assert!(run.success(), "C++ ImeFilter contract tests failed");
    }
}
