//! Activating gpui-ce C17 studio window. Not the @@ overlay.
//!
//! Quits when the last window closes (app `QuitMode::LastWindowClosed`).
//! Images are loaded from local paths only — never `ImageSource` from a URI.

use std::path::PathBuf;

use gpui::{
    actions, canvas, div, img, prelude::*, px, rgb, size, App, Bounds, Context, CursorStyle,
    Entity, FocusHandle, Focusable, KeyBinding, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, SharedString, TitlebarOptions, Window, WindowBounds, WindowKind, WindowOptions,
};

use crate::field::{read_content, LineEditor};
use crate::studio::{Document, Pt, Tool};

actions!(openatat_studio, [Undo, Redo, ExportDoc]);

pub fn bind_studio_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-z", Undo, Some("OpenAtatStudio")),
        KeyBinding::new("cmd-z", Undo, Some("OpenAtatStudio")),
        KeyBinding::new("ctrl-shift-z", Redo, Some("OpenAtatStudio")),
        KeyBinding::new("cmd-shift-z", Redo, Some("OpenAtatStudio")),
        KeyBinding::new("ctrl-y", Redo, Some("OpenAtatStudio")),
        KeyBinding::new("cmd-y", Redo, Some("OpenAtatStudio")),
        KeyBinding::new("ctrl-s", ExportDoc, Some("OpenAtatStudio")),
        KeyBinding::new("cmd-s", ExportDoc, Some("OpenAtatStudio")),
    ]);
}

pub fn open(cx: &mut App, image: Option<PathBuf>) {
    let bounds = Bounds::centered(None, size(px(960.), px(720.)), cx);
    let open = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("OpenAtat Studio".into()),
                appears_transparent: false,
                traffic_light_position: None,
            }),
            focus: true,
            show: true,
            kind: WindowKind::Normal,
            app_id: Some("openatat-ui".into()),
            window_min_size: Some(size(px(640.), px(480.))),
            ..Default::default()
        },
        |window, cx| cx.new(|cx| Studio::new(image, window, cx)),
    );
    match open {
        Ok(handle) => {
            let _ = handle.update(cx, |_, window, cx| {
                cx.activate(true);
                window.activate_window();
            });
        }
        Err(e) => {
            eprintln!("openatat-ui: failed to open studio ({e:?})");
            if cx.windows().is_empty() {
                cx.quit();
            }
        }
    }
}

struct Studio {
    doc: Option<Document>,
    tool: Tool,
    path_field: Entity<LineEditor>,
    text_field: Entity<LineEditor>,
    status: SharedString,
    preview: Option<PathBuf>,
    preview_gen: u64,
    canvas_origin: (f32, f32),
    canvas_size: (f32, f32),
    drag_from: Option<Pt>,
    stroke: Vec<Pt>,
    drawing: bool,
    two_click: Option<Pt>,
    focus_handle: FocusHandle,
}

impl Studio {
    fn new(image: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let initial = image
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let path_field = cx.new(|cx| {
            LineEditor::new(cx, initial, "local PNG or JPEG path…", false, "studio-path")
        });
        let text_field =
            cx.new(|cx| LineEditor::new(cx, "Text", "text / label", false, "studio-text"));
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window);
        let mut studio = Self {
            doc: None,
            tool: Tool::Arrow,
            path_field,
            text_field,
            status: SharedString::from(
                "Studio — local PNG/JPEG only. Nothing is uploaded. Video trim is later.",
            ),
            preview: None,
            preview_gen: 0,
            canvas_origin: (16.0, 160.0),
            canvas_size: (720.0, 420.0),
            drag_from: None,
            stroke: Vec::new(),
            drawing: false,
            two_click: None,
            focus_handle,
        };
        if let Some(path) = image {
            studio.load_path(path, cx);
        }
        studio
    }

    fn load_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        match Document::open(&path) {
            Ok(doc) => {
                self.status = SharedString::from(format!(
                    "Opened {} ({}×{}) — edits stay undoable until Export",
                    path.display(),
                    doc.width,
                    doc.height
                ));
                self.doc = Some(doc);
                self.refresh_preview(cx);
            }
            Err(e) => {
                self.status = SharedString::from(e);
            }
        }
        cx.notify();
    }

    fn sync_text(&mut self, cx: &App) {
        if let Some(doc) = self.doc.as_mut() {
            doc.text = read_content(&self.text_field, cx);
        }
    }

    fn refresh_preview(&mut self, _cx: &mut Context<Self>) {
        let Some(doc) = self.doc.as_ref() else {
            self.preview = None;
            return;
        };
        self.preview_gen += 1;
        let dir = crate::paths::cache_dir().join("studio");
        let dest = dir.join(format!(
            "preview-{}-{}.png",
            std::process::id(),
            self.preview_gen
        ));
        if let Some(parent) = dest.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match Document::encode_png(&doc.flatten()).and_then(|bytes| {
            std::fs::write(&dest, bytes).map_err(|e| e.to_string())?;
            Ok(dest)
        }) {
            Ok(path) => self.preview = Some(path),
            Err(e) => {
                self.status = SharedString::from(format!("preview: {e}"));
            }
        }
    }

    fn map_pos(&self, x: f32, y: f32) -> Option<Pt> {
        let doc = self.doc.as_ref()?;
        let (ox, oy) = self.canvas_origin;
        let (cw, ch) = self.canvas_size;
        let lx = x - ox;
        let ly = y - oy;
        if lx < 0.0 || ly < 0.0 || lx > cw || ly > ch {
            return None;
        }
        Some(Pt::new(
            lx / cw * doc.width as f32,
            ly / ch * doc.height as f32,
        ))
    }

    fn display_size(&self) -> (f32, f32) {
        let Some(doc) = self.doc.as_ref() else {
            return (720.0, 420.0);
        };
        fit(doc.width as f32, doc.height as f32, 720.0, 420.0)
    }

    fn pick_tool(&mut self, tool: Tool, cx: &mut Context<Self>) {
        self.tool = tool;
        self.two_click = None;
        self.stroke.clear();
        self.drawing = false;
        self.status = SharedString::from(format!("Tool → {}", tool_label(tool)));
        cx.notify();
    }

    fn do_undo(&mut self, cx: &mut Context<Self>) {
        if let Some(doc) = self.doc.as_mut() {
            if doc.undo() {
                self.status = SharedString::from("Undo");
                self.refresh_preview(cx);
            } else {
                self.status = SharedString::from("Nothing to undo");
            }
        }
        cx.notify();
    }

    fn do_redo(&mut self, cx: &mut Context<Self>) {
        if let Some(doc) = self.doc.as_mut() {
            if doc.redo() {
                self.status = SharedString::from("Redo");
                self.refresh_preview(cx);
            } else {
                self.status = SharedString::from("Nothing to redo");
            }
        }
        cx.notify();
    }

    fn do_export(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.doc.as_ref() else {
            self.status = SharedString::from("Open a local PNG/JPEG first");
            cx.notify();
            return;
        };
        match doc.export() {
            Ok(path) => {
                self.status = SharedString::from(format!(
                    "Exported {} — local file, not uploaded. Overlay reloads this on the next click if still up.",
                    path.display()
                ));
            }
            Err(e) => self.status = SharedString::from(e),
        }
        cx.notify();
    }

    fn press_at(&mut self, x: f32, y: f32, cx: &mut Context<Self>) {
        self.sync_text(cx);
        let Some(pt) = self.map_pos(x, y) else {
            return;
        };
        match self.tool {
            Tool::Text | Tool::Step => {
                if let Some(doc) = self.doc.as_mut() {
                    doc.commit_click(self.tool, pt);
                }
                self.refresh_preview(cx);
                self.status = SharedString::from("Placed");
            }
            Tool::Freehand | Tool::Highlighter => {
                self.drawing = true;
                self.stroke = vec![pt];
                self.drag_from = Some(pt);
            }
            _ => {
                if let Some(start) = self.two_click.take() {
                    if let Some(doc) = self.doc.as_mut() {
                        doc.commit_drag(self.tool, start, pt);
                    }
                    self.refresh_preview(cx);
                    self.status = SharedString::from("Added");
                } else {
                    self.two_click = Some(pt);
                    self.drag_from = Some(pt);
                    self.drawing = true;
                    self.status = SharedString::from("Drag or click the other corner");
                }
            }
        }
        cx.notify();
    }

    fn move_at(&mut self, x: f32, y: f32, _cx: &mut Context<Self>) {
        if !self.drawing {
            return;
        }
        let Some(pt) = self.map_pos(x, y) else {
            return;
        };
        if matches!(self.tool, Tool::Freehand | Tool::Highlighter) {
            self.stroke.push(pt);
        }
    }

    fn release_at(&mut self, x: f32, y: f32, cx: &mut Context<Self>) {
        if !self.drawing {
            return;
        }
        self.drawing = false;
        let Some(pt) = self.map_pos(x, y).or_else(|| self.stroke.last().copied()) else {
            return;
        };
        if matches!(self.tool, Tool::Freehand | Tool::Highlighter) {
            self.stroke.push(pt);
            if let Some(doc) = self.doc.as_mut() {
                doc.commit_stroke(self.tool, std::mem::take(&mut self.stroke));
            }
            self.refresh_preview(cx);
            self.status = SharedString::from("Stroke added");
        } else if let Some(start) = self.drag_from.take() {
            let dist = (pt.x - start.x).abs() + (pt.y - start.y).abs();
            if dist > 4.0 {
                self.two_click = None;
                if let Some(doc) = self.doc.as_mut() {
                    doc.commit_drag(self.tool, start, pt);
                }
                self.refresh_preview(cx);
                self.status = SharedString::from("Added");
            }
        }
        cx.notify();
    }
}

impl Focusable for Studio {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Studio {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (dw, dh) = self.display_size();
        self.canvas_size = (dw, dh);
        let preview = self.preview.clone();
        let entity = cx.entity();

        div()
            .track_focus(&self.focus_handle)
            .key_context("OpenAtatStudio")
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x141518))
            .text_color(rgb(0xe8e8ea))
            .font_family("sans-serif")
            .on_action(cx.listener(|this, _: &Undo, _, cx| this.do_undo(cx)))
            .on_action(cx.listener(|this, _: &Redo, _, cx| this.do_redo(cx)))
            .on_action(cx.listener(|this, _: &ExportDoc, _, cx| this.do_export(cx)))
            .child(self.render_chrome(cx))
            .child(self.render_tools(cx))
            .child(self.render_fields(cx))
            .child(
                div()
                    .id("studio-body")
                    .flex_1()
                    .min_h(px(0.))
                    .p_4()
                    .child(
                        div()
                            .id("studio-canvas")
                            .w(px(dw))
                            .h(px(dh))
                            .rounded_md()
                            .bg(rgb(0x1c1d22))
                            .border_1()
                            .border_color(rgb(0x2c2d33))
                            .overflow_hidden()
                            .cursor(CursorStyle::Crosshair)
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                                    this.press_at(
                                        f32::from(ev.position.x),
                                        f32::from(ev.position.y),
                                        cx,
                                    );
                                }),
                            )
                            .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, _, cx| {
                                this.move_at(f32::from(ev.position.x), f32::from(ev.position.y), cx);
                            }))
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(move |this, ev: &MouseUpEvent, _, cx| {
                                    this.release_at(
                                        f32::from(ev.position.x),
                                        f32::from(ev.position.y),
                                        cx,
                                    );
                                }),
                            )
                            .child({
                                let entity = entity.clone();
                                canvas(
                                    move |bounds, _, cx| {
                                        entity.update(cx, |this, _| {
                                            this.canvas_origin = (
                                                f32::from(bounds.origin.x),
                                                f32::from(bounds.origin.y),
                                            );
                                            this.canvas_size = (
                                                f32::from(bounds.size.width),
                                                f32::from(bounds.size.height),
                                            );
                                        });
                                    },
                                    |_bounds, _, _, _| {},
                                )
                            })
                            .when_some(preview, |el, path| {
                                el.child(
                                    img(path)
                                        .w(px(dw))
                                        .h(px(dh))
                                        .object_fit(gpui::ObjectFit::Contain),
                                )
                            })
                            .when(self.doc.is_none(), |el| {
                                el.child(
                                    div()
                                        .p_4()
                                        .text_sm()
                                        .text_color(rgb(0x8b8f99))
                                        .child(
                                            "Open a local PNG/JPEG (Tools → Open Annotate, or --image). Nothing is uploaded.",
                                        ),
                                )
                            }),
                    ),
            )
            .child(
                div()
                    .px_4()
                    .py_2()
                    .text_xs()
                    .text_color(rgb(0x9aa0a6))
                    .border_t_1()
                    .border_color(rgb(0x2c2d33))
                    .child(self.status.clone()),
            )
    }
}

impl Studio {
    fn render_chrome(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .px_4()
            .py_3()
            .bg(rgb(0x1c1d22))
            .border_b_1()
            .border_color(rgb(0x2c2d33))
            .child(
                div()
                    .text_lg()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Studio"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0x8b8f99))
                    .child("annotate · crop · local only  ·  not the @@ overlay"),
            )
            .child(div().flex_1())
            .child(action_btn("Undo", rgb(0x3a3b42), cx, |this, cx| {
                this.do_undo(cx)
            }))
            .child(action_btn("Redo", rgb(0x3a3b42), cx, |this, cx| {
                this.do_redo(cx)
            }))
            .child(action_btn("Export PNG", rgb(0x3d5a9c), cx, |this, cx| {
                this.do_export(cx)
            }))
    }

    fn render_tools(&self, cx: &mut Context<Self>) -> impl IntoElement {
        const TOOLS: &[(Tool, &str)] = &[
            (Tool::Arrow, "Arrow"),
            (Tool::Rect, "Rect"),
            (Tool::Ellipse, "Ellipse"),
            (Tool::Freehand, "Freehand"),
            (Tool::Highlighter, "Highlighter"),
            (Tool::Text, "Text"),
            (Tool::Step, "Step"),
            (Tool::Blur, "Blur"),
            (Tool::Pixelate, "Pixelate"),
            (Tool::Spotlight, "Spotlight"),
            (Tool::Crop, "Crop"),
        ];
        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap_2()
            .px_4()
            .py_2()
            .children(TOOLS.iter().copied().map(|(tool, label)| {
                let on = self.tool == tool;
                div()
                    .id(label)
                    .cursor(CursorStyle::PointingHand)
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .bg(if on { rgb(0x3d5a9c) } else { rgb(0x2a2b31) })
                    .text_sm()
                    .on_click(cx.listener(move |this, _, _, cx| this.pick_tool(tool, cx)))
                    .child(label)
            }))
    }

    fn render_fields(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_row()
            .gap_2()
            .px_4()
            .py_2()
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x8b8f99))
                            .child("Local image"),
                    )
                    .child(self.path_field.clone()),
            )
            .child(action_btn("Open", rgb(0x3d5a9c), cx, |this, cx| {
                let raw = read_content(&this.path_field, cx);
                this.load_path(PathBuf::from(raw.trim()), cx);
            }))
            .child(
                div()
                    .w(px(200.))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_xs().text_color(rgb(0x8b8f99)).child("Text"))
                    .child(self.text_field.clone()),
            )
    }
}

fn tool_label(tool: Tool) -> &'static str {
    match tool {
        Tool::Arrow => "Arrow",
        Tool::Rect => "Rect",
        Tool::Ellipse => "Ellipse",
        Tool::Freehand => "Freehand",
        Tool::Highlighter => "Highlighter",
        Tool::Text => "Text",
        Tool::Step => "Step",
        Tool::Blur => "Blur",
        Tool::Pixelate => "Pixelate",
        Tool::Spotlight => "Spotlight",
        Tool::Crop => "Crop",
    }
}

fn fit(w: f32, h: f32, max_w: f32, max_h: f32) -> (f32, f32) {
    if w <= 0.0 || h <= 0.0 {
        return (max_w, max_h);
    }
    let s = (max_w / w).min(max_h / h).min(1.0);
    ((w * s).max(1.0), (h * s).max(1.0))
}

fn action_btn(
    label: &'static str,
    color: gpui::Rgba,
    cx: &mut Context<Studio>,
    on_click: impl Fn(&mut Studio, &mut Context<Studio>) + 'static,
) -> impl IntoElement {
    div()
        .id(label)
        .cursor(CursorStyle::PointingHand)
        .px_3()
        .py_2()
        .rounded_md()
        .bg(color)
        .text_sm()
        .on_click(cx.listener(move |this, _, _, cx| on_click(this, cx)))
        .child(label)
}
