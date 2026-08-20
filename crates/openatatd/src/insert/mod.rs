//! Clipboard-first insert. Verify focus. AT-SPI when a text field is focused.

use openatat_ipc::FocusSnapshot;

use crate::clipboard;
use crate::error::{Error, Result};
use crate::focus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertOutcome {
    Inserted,
    CopiedOnly,
    AbortedFocusChanged,
}

/// Order is mandatory: clipboard, then focus check, then a11y insert.
pub fn tab_insert(text: &str, expected: &FocusSnapshot) -> Result<InsertOutcome> {
    clipboard::copy_text(text)?;
    if focus::address_changed(expected) {
        return Ok(InsertOutcome::AbortedFocusChanged);
    }
    match insert_into_focused_field(text) {
        Ok(true) => Ok(InsertOutcome::Inserted),
        Ok(false) => Ok(InsertOutcome::CopiedOnly),
        Err(e) => {
            eprintln!("openatatd: insert failed after clipboard write ({e}); result is still copied");
            Ok(InsertOutcome::CopiedOnly)
        }
    }
}

fn insert_into_focused_field(text: &str) -> Result<bool> {
    #[cfg(target_os = "linux")]
    {
        return linux::insert_into_focused_field(text);
    }
    #[cfg(target_os = "macos")]
    {
        let _ = text;
        macos::insert_into_focused_field()
    }
    #[cfg(target_os = "windows")]
    {
        let _ = text;
        windows::insert_into_focused_field()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = text;
        Ok(false)
    }
}

/// Secure-field probe for the IME filter. Never cached by the caller.
pub fn probe_field_kind() -> crate::trigger::FieldKind {
    #[cfg(target_os = "linux")]
    {
        linux::probe_field_kind()
    }
    #[cfg(not(target_os = "linux"))]
    {
        crate::trigger::FieldKind::AcceptsText
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use zbus::blocking::Connection;
    use zbus::zvariant::OwnedObjectPath;
    use zbus::{proxy, Address};

    /// AT-SPI `Role::PasswordText`.
    const ROLE_PASSWORD_TEXT: u32 = 40;
    const ROLE_TEXT: u32 = 61;
    const ROLE_TERMINAL: u32 = 60;
    const ROLE_ENTRY: u32 = 79;
    const ROLE_EDITBAR: u32 = 77;
    const ROLE_DOCUMENT_TEXT: u32 = 94;
    /// `State::Focused` bit in the first u32 of GetState.
    const STATE_FOCUSED: u32 = 1 << 12;
    const STATE_DEFUNCT: u32 = 1 << 6;

    #[proxy(
        interface = "org.a11y.Bus",
        default_service = "org.a11y.Bus",
        default_path = "/org/a11y/bus",
        gen_blocking = true,
        gen_async = false
    )]
    trait A11yBus {
        fn get_address(&self) -> zbus::Result<String>;
    }

    #[proxy(
        interface = "org.a11y.atspi.Accessible",
        assume_defaults = false,
        gen_blocking = true,
        gen_async = false
    )]
    trait Accessible {
        fn get_role(&self) -> zbus::Result<u32>;
        fn get_state(&self) -> zbus::Result<Vec<u32>>;
        fn get_interfaces(&self) -> zbus::Result<Vec<String>>;
        fn get_children(&self) -> zbus::Result<Vec<(String, OwnedObjectPath)>>;
    }

    #[proxy(
        interface = "org.a11y.atspi.EditableText",
        assume_defaults = false,
        gen_blocking = true,
        gen_async = false
    )]
    trait EditableText {
        fn insert_text(&self, position: i32, text: &str, length: i32) -> zbus::Result<bool>;
    }

    #[proxy(
        interface = "org.a11y.atspi.Text",
        assume_defaults = false,
        gen_blocking = true,
        gen_async = false
    )]
    trait TextIface {
        fn get_caret_offset(&self) -> zbus::Result<i32>;
    }

    fn a11y_connection() -> Result<Connection> {
        let session = Connection::session().map_err(|e| Error::msg(e.to_string()))?;
        let bus = A11yBusProxy::new(&session).map_err(|e| Error::msg(e.to_string()))?;
        let addr = bus.get_address().map_err(|e| Error::msg(e.to_string()))?;
        let address: Address = addr.parse().map_err(|e| Error::msg(format!("{e}")))?;
        zbus::blocking::connection::Builder::address(address)
            .map_err(|e| Error::msg(e.to_string()))?
            .build()
            .map_err(|e| Error::msg(e.to_string()))
    }

    struct Node {
        dest: String,
        path: OwnedObjectPath,
    }

    fn owned_dest(name: &str) -> Result<zbus::names::BusName<'static>> {
        zbus::names::BusName::try_from(name.to_string()).map_err(|e| Error::msg(e.to_string()))
    }

    fn walk_focused(conn: &Connection) -> Result<Option<(Node, u32, Vec<String>)>> {
        let root = Node {
            dest: "org.a11y.atspi.Registry".into(),
            path: OwnedObjectPath::try_from("/org/a11y/atspi/accessible/root")
                .map_err(|e| Error::msg(e.to_string()))?,
        };
        let mut q = std::collections::VecDeque::from([root]);
        let mut seen = 0usize;
        while let Some(node) = q.pop_front() {
            seen += 1;
            if seen > 80 {
                break;
            }
            let dest = owned_dest(&node.dest)?;
            let acc = AccessibleProxy::builder(conn)
                .destination(dest)
                .map_err(|e| Error::msg(e.to_string()))?
                .path(node.path.clone())
                .map_err(|e| Error::msg(e.to_string()))?
                .build()
                .map_err(|e| Error::msg(e.to_string()))?;
            let state = acc.get_state().unwrap_or_default();
            let bits = state.first().copied().unwrap_or(0);
            if bits & STATE_DEFUNCT != 0 {
                continue;
            }
            let role = acc.get_role().unwrap_or(0);
            let ifaces = acc.get_interfaces().unwrap_or_default();
            if bits & STATE_FOCUSED != 0 {
                return Ok(Some((node, role, ifaces)));
            }
            if let Ok(children) = acc.get_children() {
                for (dest, path) in children {
                    q.push_back(Node { dest, path });
                }
            }
        }
        Ok(None)
    }

    pub fn probe_field_kind() -> crate::trigger::FieldKind {
        let Ok(conn) = a11y_connection() else {
            return crate::trigger::FieldKind::Other;
        };
        match walk_focused(&conn) {
            Ok(Some((_, role, ifaces))) => {
                if role == ROLE_PASSWORD_TEXT {
                    crate::trigger::FieldKind::Secure
                } else if is_text_field(role, &ifaces) {
                    crate::trigger::FieldKind::AcceptsText
                } else {
                    crate::trigger::FieldKind::Other
                }
            }
            _ => crate::trigger::FieldKind::Other,
        }
    }

    fn is_text_field(role: u32, ifaces: &[String]) -> bool {
        ifaces.iter().any(|i| i.contains("EditableText"))
            || matches!(
                role,
                ROLE_TEXT | ROLE_ENTRY | ROLE_TERMINAL | ROLE_EDITBAR | ROLE_DOCUMENT_TEXT
            )
    }

    pub fn insert_into_focused_field(text: &str) -> Result<bool> {
        let conn = a11y_connection()?;
        let Some((node, role, ifaces)) = walk_focused(&conn)? else {
            return Ok(false);
        };
        if role == ROLE_PASSWORD_TEXT {
            return Ok(false);
        }
        if !is_text_field(role, &ifaces) {
            return Ok(false);
        }
        let dest = owned_dest(&node.dest)?;
        let caret = TextIfaceProxy::builder(&conn)
            .destination(dest.clone())
            .map_err(|e| Error::msg(e.to_string()))?
            .path(node.path.clone())
            .map_err(|e| Error::msg(e.to_string()))?
            .build()
            .ok()
            .and_then(|t| t.get_caret_offset().ok())
            .unwrap_or(0);
        let edit = EditableTextProxy::builder(&conn)
            .destination(dest)
            .map_err(|e| Error::msg(e.to_string()))?
            .path(node.path)
            .map_err(|e| Error::msg(e.to_string()))?
            .build()
            .map_err(|e| Error::msg(e.to_string()))?;
        let ok = edit
            .insert_text(caret, text, text.len() as i32)
            .map_err(|e| Error::msg(e.to_string()))?;
        Ok(ok)
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;

    pub fn insert_into_focused_field() -> Result<bool> {
        // AXUIElementSetAttributeValue / AXUIElement parameterized text insert.
        Err(Error::msg(
            "macOS insert is a stub: AXUIElement (Accessibility)",
        ))
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use super::*;

    pub fn insert_into_focused_field() -> Result<bool> {
        // IUIAutomation / ValuePattern / TextPattern. WM_CHAR is a last resort.
        Err(Error::msg("Windows insert is a stub: UI Automation"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abort_outcome_is_distinct() {
        assert_ne!(
            InsertOutcome::AbortedFocusChanged,
            InsertOutcome::CopiedOnly
        );
        assert_ne!(InsertOutcome::Inserted, InsertOutcome::CopiedOnly);
    }
}
