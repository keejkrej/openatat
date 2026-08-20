//! Fcitx5 / IBus shaped filter. v0 is the interface + no-op backends.
//!
//! There is no shippable Rust crate for a Fcitx5 addon (addons are C++).
//! There is no maintained IBus engine crate we will ship. P1 writes a small
//! C++ `fcitx5-openatat` (and/or IBus engine) that:
//!   1. probes the focused field every key (AT-SPI; never cached)
//!   2. ignores preedit / composing
//!   3. feeds committed characters into [`super::DetectionBuffer`]
//!   4. on `@@`, swallows the two characters and sends `DaemonRequest::Trigger`
//!      to `$XDG_RUNTIME_DIR/openatat/trigger.sock`
//!
//! A global Hyprland bind is **not** this module and is not the product.

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

/// In-process IME filter used by tests and by the future addon via FFI/socket.
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

/// Product backend. Not implemented in P0 — C++ addon talks to the socket.
pub trait ImeBackend {
    fn name(&self) -> &'static str;
    fn start(&mut self) -> Result<(), String>;
}

/// Stub. No Rust Fcitx5 addon crate exists.
#[derive(Debug, Default)]
pub struct Fcitx5Backend;

impl ImeBackend for Fcitx5Backend {
    fn name(&self) -> &'static str {
        "fcitx5"
    }

    fn start(&mut self) -> Result<(), String> {
        // P1: spawn/expect `fcitx5-openatat` and keep the unix socket.
        Ok(())
    }
}

/// Stub. No maintained Rust IBus engine crate we will depend on.
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
}
