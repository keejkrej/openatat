//! Minimal single/multi-line editor for Settings / History search.
//! gpui-ce 0.3 has no stock text field; this follows the official input example.

use std::ops::Range;

use gpui::{
    actions, canvas, div, prelude::*, px, rgb, App, Bounds, ClipboardItem, Context, CursorStyle,
    ElementId, ElementInputHandler, Entity, EntityInputHandler, FocusHandle, Focusable, KeyBinding,
    Pixels, Point, SharedString, UTF16Selection, Window,
};

actions!(
    openatat_field,
    [Backspace, Delete, Left, Right, Home, End, SelectAll, Copy, Paste, Cut, Newline]
);

pub fn bind_editor_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, Some("OpenAtatField")),
        KeyBinding::new("delete", Delete, Some("OpenAtatField")),
        KeyBinding::new("left", Left, Some("OpenAtatField")),
        KeyBinding::new("right", Right, Some("OpenAtatField")),
        KeyBinding::new("home", Home, Some("OpenAtatField")),
        KeyBinding::new("end", End, Some("OpenAtatField")),
        KeyBinding::new("ctrl-a", SelectAll, Some("OpenAtatField")),
        KeyBinding::new("cmd-a", SelectAll, Some("OpenAtatField")),
        KeyBinding::new("ctrl-c", Copy, Some("OpenAtatField")),
        KeyBinding::new("cmd-c", Copy, Some("OpenAtatField")),
        KeyBinding::new("ctrl-v", Paste, Some("OpenAtatField")),
        KeyBinding::new("cmd-v", Paste, Some("OpenAtatField")),
        KeyBinding::new("ctrl-x", Cut, Some("OpenAtatField")),
        KeyBinding::new("cmd-x", Cut, Some("OpenAtatField")),
        KeyBinding::new("enter", Newline, Some("OpenAtatField")),
    ]);
}

pub struct LineEditor {
    pub content: String,
    pub placeholder: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    pub multiline: bool,
    focus_handle: FocusHandle,
    id: ElementId,
}

impl LineEditor {
    pub fn new(
        cx: &mut Context<Self>,
        content: impl Into<String>,
        placeholder: impl Into<SharedString>,
        multiline: bool,
        id: impl Into<ElementId>,
    ) -> Self {
        let content = content.into();
        let len = content.len();
        Self {
            content,
            placeholder: placeholder.into(),
            selected_range: len..len,
            selection_reversed: false,
            marked_range: None,
            multiline,
            focus_handle: cx.focus_handle(),
            id: id.into(),
        }
    }

    pub fn set_content(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.content = text.into();
        let len = self.content.len();
        self.selected_range = len..len;
        self.marked_range = None;
        cx.notify();
    }

    fn cursor(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn select(&mut self, range: Range<usize>) {
        let len = self.content.len();
        let start = range.start.min(len);
        let end = range.end.min(len);
        self.selected_range = start..end;
        self.selection_reversed = false;
    }

    fn replace_selection(&mut self, text: &str) {
        if !self.multiline && text.contains('\n') {
            let one = text.replace(['\n', '\r'], "");
            return self.replace_selection(&one);
        }
        let range = self.selected_range.clone();
        self.content.replace_range(range.clone(), text);
        let i = range.start + text.len();
        self.selected_range = i..i;
        self.selection_reversed = false;
        self.marked_range = None;
    }

    fn move_left(&mut self) {
        let i = next_boundary(&self.content, self.cursor().saturating_sub(1));
        self.select(i..i);
    }

    fn move_right(&mut self) {
        let i = next_boundary(&self.content, self.cursor().saturating_add(1));
        self.select(i..i);
    }
}

impl Focusable for LineEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for LineEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus_handle.is_focused(window);
        let display = if self.content.is_empty() && !focused {
            self.placeholder.clone()
        } else {
            SharedString::from(self.content.clone())
        };
        let empty_ph = self.content.is_empty();
        let entity = cx.entity();
        let focus = self.focus_handle.clone();
        let min_h = if self.multiline { px(96.) } else { px(30.) };

        div()
            .id(self.id.clone())
            .key_context("OpenAtatField")
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(|this, _: &Backspace, _, cx| {
                if this.selected_range.is_empty() {
                    this.move_left();
                }
                this.replace_selection("");
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Delete, _, cx| {
                if this.selected_range.is_empty() {
                    this.move_right();
                    this.move_left();
                    if this.cursor() < this.content.len() {
                        this.move_right();
                    }
                }
                this.replace_selection("");
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Left, _, cx| {
                this.move_left();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Right, _, cx| {
                this.move_right();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Home, _, cx| {
                this.select(0..0);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &End, _, cx| {
                let n = this.content.len();
                this.select(n..n);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| {
                let n = this.content.len();
                this.select(0..n);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Copy, _, cx| {
                if !this.selected_range.is_empty() {
                    let t = this.content[this.selected_range.clone()].to_string();
                    cx.write_to_clipboard(ClipboardItem::new_string(t));
                }
            }))
            .on_action(cx.listener(|this, _: &Cut, _, cx| {
                if !this.selected_range.is_empty() {
                    let t = this.content[this.selected_range.clone()].to_string();
                    cx.write_to_clipboard(ClipboardItem::new_string(t));
                    this.replace_selection("");
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &Paste, _, cx| {
                if let Some(item) = cx.read_from_clipboard() {
                    if let Some(text) = item.text() {
                        this.replace_selection(&text);
                        cx.notify();
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &Newline, _, cx| {
                if this.multiline {
                    this.replace_selection("\n");
                    cx.notify();
                }
            }))
            .on_click(cx.listener(|this, _, window, cx| {
                this.focus_handle.focus(window);
                cx.notify();
            }))
            .flex()
            .w_full()
            .min_h(min_h)
            .px(px(8.))
            .py(px(6.))
            .rounded_md()
            .bg(rgb(0x1e1f24))
            .border_1()
            .border_color(if focused {
                rgb(0x6d8cff)
            } else {
                rgb(0x3a3b42)
            })
            .text_sm()
            .text_color(if empty_ph {
                rgb(0x7a7e88)
            } else {
                rgb(0xe8e8ea)
            })
            .child(
                div()
                    .id("field-text")
                    .w_full()
                    .flex_1()
                    .whitespace_normal()
                    .child(display)
                    .child(canvas(
                        |_, _, _| {},
                        move |bounds, _, window, cx| {
                            window.handle_input(
                                &focus,
                                ElementInputHandler::new(bounds, entity),
                                cx,
                            );
                        },
                    )),
            )
    }
}

impl EntityInputHandler for LineEditor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = utf16_to_bytes(&self.content, range_utf16);
        actual_range.replace(offset_to_utf16(&self.content, range.clone()));
        Some(self.content.get(range)?.to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: offset_to_utf16(&self.content, self.selected_range.clone()),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        Some(offset_to_utf16(&self.content, self.marked_range.clone()?))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .map(|r| utf16_to_bytes(&self.content, r))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());
        let safe = clamp_range(&self.content, range);
        self.content.replace_range(safe.clone(), text);
        let i = safe.start + text.len();
        self.selected_range = i..i;
        self.marked_range = None;
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .map(|r| utf16_to_bytes(&self.content, r))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());
        let safe = clamp_range(&self.content, range);
        self.content.replace_range(safe.clone(), new_text);
        self.marked_range = Some(safe.start..safe.start + new_text.len());
        self.selected_range = new_selected_range
            .map(|r| utf16_to_bytes(&self.content, r))
            .unwrap_or_else(|| {
                let i = safe.start + new_text.len();
                i..i
            });
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(element_bounds)
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(offset_to_utf16(&self.content, self.cursor()..self.cursor()).start)
    }
}

fn next_boundary(s: &str, i: usize) -> usize {
    let i = i.min(s.len());
    if s.is_char_boundary(i) {
        i
    } else {
        (0..=i).rev().find(|&j| s.is_char_boundary(j)).unwrap_or(0)
    }
}

fn clamp_range(s: &str, range: Range<usize>) -> Range<usize> {
    let start = next_boundary(s, range.start.min(s.len()));
    let end = next_boundary(s, range.end.min(s.len()));
    start..end.max(start)
}

fn offset_to_utf16(s: &str, range: Range<usize>) -> Range<usize> {
    let start = s[..range.start.min(s.len())].encode_utf16().count();
    let end = s[..range.end.min(s.len())].encode_utf16().count();
    start..end
}

fn utf16_to_bytes(s: &str, range: Range<usize>) -> Range<usize> {
    let mut start = s.len();
    let mut end = s.len();
    let mut u = 0usize;
    for (byte, ch) in s.char_indices() {
        if u == range.start {
            start = byte;
        }
        if u == range.end {
            end = byte;
            break;
        }
        u += ch.len_utf16();
    }
    if u == range.start {
        start = s.len();
    }
    if u == range.end {
        end = s.len();
    }
    start..end
}

pub fn read_content(editor: &Entity<LineEditor>, cx: &App) -> String {
    editor.read(cx).content.clone()
}
