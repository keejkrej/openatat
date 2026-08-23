//! AT-SPI via `zbus` on the a11y bus. Same bus the insert path already uses.
//!
//! Text selection is `org.a11y.atspi.Text` (`GetNSelections`, `GetSelection`,
//! `GetText`, `GetRangeExtents`). Mouse-up is `RegisterEvent("mouse:b1r")` plus
//! `org.a11y.atspi.Event.Mouse::Button`. This is not a Nautilus file API.

use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::time::Duration;

use zbus::blocking::Connection;
use zbus::zvariant::OwnedObjectPath;
use zbus::{proxy, Address};

use super::{
    interpret_selection_snapshot, is_text_field, Rect, SelectionProbe, ROLE_PASSWORD_TEXT,
};
use crate::error::{Error, Result};
use crate::trigger::FieldKind;

/// `State::Focused` bit in the first u32 of GetState.
const STATE_FOCUSED: u32 = 1 << 12;
const STATE_DEFUNCT: u32 = 1 << 6;
const COORD_TYPE_SCREEN: u32 = 0;

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
    fn delete_text(&self, start_pos: i32, end_pos: i32) -> zbus::Result<bool>;
}

#[proxy(
    interface = "org.a11y.atspi.Text",
    assume_defaults = false,
    gen_blocking = true,
    gen_async = false
)]
trait TextIface {
    fn get_caret_offset(&self) -> zbus::Result<i32>;
    fn get_nselections(&self) -> zbus::Result<i32>;
    fn get_selection(&self, selection_num: i32) -> zbus::Result<(i32, i32)>;
    fn get_text(&self, start_offset: i32, end_offset: i32) -> zbus::Result<String>;
    fn get_range_extents(
        &self,
        start_offset: i32,
        end_offset: i32,
        coord_type: u32,
    ) -> zbus::Result<(i32, i32, i32, i32)>;
    fn get_attributes(&self, offset: i32) -> zbus::Result<(HashMap<String, String>, i32, i32)>;
}

#[proxy(
    interface = "org.a11y.atspi.Registry",
    default_service = "org.a11y.atspi.Registry",
    default_path = "/org/a11y/atspi/registry",
    gen_blocking = true,
    gen_async = false
)]
trait Registry {
    fn register_event(&self, event: &str) -> zbus::Result<()>;
    fn deregister_event(&self, event: &str) -> zbus::Result<()>;
}

pub(super) fn a11y_connection() -> Result<Connection> {
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
        if seen > 120 {
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

fn text_proxy<'a>(
    conn: &'a Connection,
    dest: zbus::names::BusName<'static>,
    path: OwnedObjectPath,
) -> Result<TextIfaceProxy<'a>> {
    TextIfaceProxy::builder(conn)
        .destination(dest)
        .map_err(|e| Error::msg(e.to_string()))?
        .path(path)
        .map_err(|e| Error::msg(e.to_string()))?
        .build()
        .map_err(|e| Error::msg(e.to_string()))
}

fn edit_proxy<'a>(
    conn: &'a Connection,
    dest: zbus::names::BusName<'static>,
    path: OwnedObjectPath,
) -> Result<EditableTextProxy<'a>> {
    EditableTextProxy::builder(conn)
        .destination(dest)
        .map_err(|e| Error::msg(e.to_string()))?
        .path(path)
        .map_err(|e| Error::msg(e.to_string()))?
        .build()
        .map_err(|e| Error::msg(e.to_string()))
}

pub fn probe_field_kind() -> FieldKind {
    let Ok(conn) = a11y_connection() else {
        return FieldKind::Other;
    };
    match walk_focused(&conn) {
        Ok(Some((_, role, ifaces))) => {
            if role == ROLE_PASSWORD_TEXT {
                FieldKind::Secure
            } else if is_text_field(role, &ifaces) {
                FieldKind::AcceptsText
            } else {
                FieldKind::Other
            }
        }
        _ => FieldKind::Other,
    }
}

pub fn probe_selection() -> SelectionProbe {
    let Ok(conn) = a11y_connection() else {
        return SelectionProbe::Unavailable;
    };
    let Ok(Some((node, role, ifaces))) = walk_focused(&conn) else {
        return SelectionProbe::Unavailable;
    };
    if role == ROLE_PASSWORD_TEXT {
        return SelectionProbe::Secure;
    }
    if !is_text_field(role, &ifaces) {
        return SelectionProbe::Unavailable;
    }
    let Ok(dest) = owned_dest(&node.dest) else {
        return SelectionProbe::Unavailable;
    };
    let Ok(text) = text_proxy(&conn, dest, node.path) else {
        return SelectionProbe::Unavailable;
    };
    let n = text.get_nselections().ok();
    let range = n
        .filter(|&count| count > 0)
        .and_then(|_| text.get_selection(0).ok());
    let (got, html) = match range {
        Some((start, end)) if start != end => {
            let got = text.get_text(start, end).ok();
            let html = got
                .as_deref()
                .and_then(|plain| html_from_attrs(&text, start, plain));
            (got, html)
        }
        _ => (None, None),
    };
    let bounds = range.and_then(|(start, end)| {
        text.get_range_extents(start, end, COORD_TYPE_SCREEN)
            .ok()
            .map(|(x, y, width, height)| Rect {
                x,
                y,
                width,
                height,
            })
    });
    interpret_selection_snapshot(role, &ifaces, n, range, got, bounds, html)
}

fn html_from_attrs(text: &TextIfaceProxy<'_>, offset: i32, plain: &str) -> Option<String> {
    let (attrs, _, _) = text.get_attributes(offset).ok()?;
    let bold = attrs.iter().any(|(k, v)| {
        let k = k.to_ascii_lowercase();
        (k == "weight" || k == "font-weight")
            && (v.contains("bold") || v.parse::<i32>().ok().is_some_and(|n| n >= 600))
    });
    let italic = attrs.iter().any(|(k, v)| {
        let k = k.to_ascii_lowercase();
        (k == "style" || k == "font-style") && v.to_ascii_lowercase().contains("italic")
    });
    if !bold && !italic {
        return None;
    }
    let mut inner = html_escape(plain);
    if bold {
        inner = format!("<b>{inner}</b>");
    }
    if italic {
        inner = format!("<i>{inner}</i>");
    }
    Some(format!("<meta charset=\"utf-8\"><div>{inner}</div>"))
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
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
    let caret = text_proxy(&conn, dest.clone(), node.path.clone())
        .ok()
        .and_then(|t| t.get_caret_offset().ok())
        .unwrap_or(0);
    let edit = edit_proxy(&conn, dest, node.path)?;
    let ok = edit
        .insert_text(caret, text, text.len() as i32)
        .map_err(|e| Error::msg(e.to_string()))?;
    Ok(ok)
}

pub fn replace_range(text: &str, start: i32, end: i32) -> Result<bool> {
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
    let edit = edit_proxy(&conn, dest, node.path)?;
    let deleted = edit
        .delete_text(start, end)
        .map_err(|e| Error::msg(e.to_string()))?;
    if !deleted {
        return Ok(false);
    }
    let ok = edit
        .insert_text(start, text, text.len() as i32)
        .map_err(|e| Error::msg(e.to_string()))?;
    Ok(ok)
}

/// Pointer position from the AT-SPI mouse-up that triggered a probe.
pub struct MouseUpHit {
    pub probe: SelectionProbe,
    pub pointer: Option<(i32, i32)>,
}

/// AT-SPI mouse-up watcher. Keyboard `object:text-selection-changed` is
/// not registered, so shift+arrow selections never arrive here.
pub fn spawn_mouse_up_watcher(tx: Sender<MouseUpHit>) {
    std::thread::Builder::new()
        .name("openatat-sel".into())
        .spawn(move || {
            if let Err(e) = watch_mouse_up(tx) {
                eprintln!("openatatd: selection watcher idle ({e})");
            }
        })
        .ok();
}

fn watch_mouse_up(tx: Sender<MouseUpHit>) -> Result<()> {
    let conn = a11y_connection()?;
    let registry = RegistryProxy::new(&conn).map_err(|e| Error::msg(e.to_string()))?;
    // libatspi event names: mouse:b1r is left-button release.
    for ev in ["mouse:b1r", "mouse:button"] {
        let _ = registry.register_event(ev);
    }
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.a11y.atspi.Event.Mouse")
        .map_err(|e| Error::msg(e.to_string()))?
        .member("Button")
        .map_err(|e| Error::msg(e.to_string()))?
        .build();
    let iter = zbus::blocking::MessageIterator::for_match_rule(rule, &conn, Some(32))
        .map_err(|e| Error::msg(e.to_string()))?;
    for msg in iter {
        let Ok(msg) = msg else {
            continue;
        };
        let Ok((detail, x, y)) = parse_mouse_button(&msg) else {
            continue;
        };
        if !super::mouse_button_is_left_release(&detail) {
            continue;
        }
        // Give the toolkit a tick to commit the drag selection.
        std::thread::sleep(Duration::from_millis(20));
        let hit = MouseUpHit {
            probe: probe_selection(),
            pointer: Some((x, y)),
        };
        if tx.send(hit).is_err() {
            break;
        }
    }
    Ok(())
}

fn parse_mouse_button(msg: &zbus::message::Message) -> zbus::Result<(String, i32, i32)> {
    // org.a11y.atspi.Event.Mouse.Button: (s, i, i, v, a{sv})
    let body = msg.body();
    if let Ok((detail, x, y, _, _)) = body.deserialize::<(
        String,
        i32,
        i32,
        zbus::zvariant::Value<'_>,
        HashMap<String, zbus::zvariant::Value<'_>>,
    )>() {
        return Ok((detail, x, y));
    }
    body.deserialize::<(String, i32, i32)>()
}
