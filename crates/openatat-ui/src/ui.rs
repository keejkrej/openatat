//! Activating gpui-ce Settings + History + Studio windows.
//!
//! Not the @@ overlay. `QuitMode::LastWindowClosed` so the process exits
//! when the last window closes.

use std::path::PathBuf;

use gpui::{
    actions, div, prelude::*, px, rgb, size, App, Application, Bounds, ClipboardItem, Context,
    CursorStyle, Entity, FocusHandle, Focusable, KeyBinding, QuitMode, SharedString,
    TitlebarOptions, Window, WindowBounds, WindowKind, WindowOptions,
};
use openatat_ipc::UiPage;

use crate::field::{self, read_content, LineEditor};
use crate::history::{self, entry_label, format_timestamp};
use crate::permissions::{LINUX_GRANTS, MAC_GRANTS, WIN_GRANTS};
use crate::providers::{detect_on_path, PROVIDER_CHOICES};
use crate::settings::{self, apply_provider_pick, AgentEdit};
use crate::studio_ui;

actions!(openatat_ui, [Quit]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Settings,
    History,
    Permissions,
}

pub fn run(cli: crate::Cli) -> Result<(), String> {
    Application::new()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(move |cx: &mut App| {
            field::bind_editor_keys(cx);
            studio_ui::bind_studio_keys(cx);
            cx.bind_keys([
                KeyBinding::new("ctrl-q", Quit, None),
                KeyBinding::new("cmd-q", Quit, None),
            ]);
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            match cli.page {
                UiPage::Studio => studio_ui::open(cx, cli.image.clone()),
                page => open_shell(cx, page),
            }
        });
    Ok(())
}

fn open_shell(cx: &mut App, page: UiPage) {
    let bounds = Bounds::centered(None, size(px(820.), px(640.)), cx);
    let open = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("OpenAtat".into()),
                appears_transparent: false,
                traffic_light_position: None,
            }),
            focus: true,
            show: true,
            kind: WindowKind::Normal,
            app_id: Some("openatat-ui".into()),
            window_min_size: Some(size(px(560.), px(420.))),
            ..Default::default()
        },
        |window, cx| cx.new(|cx| Shell::new(page, window, cx)),
    );

    match open {
        Ok(handle) => {
            let _ = handle.update(cx, |_, window, cx| {
                cx.activate(true);
                window.activate_window();
            });
        }
        Err(e) => {
            eprintln!("openatat-ui: failed to open window ({e:?})");
            cx.quit();
        }
    }
}

struct Shell {
    tab: Tab,
    edit: AgentEdit,
    argv: Entity<LineEditor>,
    search: Entity<LineEditor>,
    annotate_path: Entity<LineEditor>,
    detected: Vec<(String, Option<std::path::PathBuf>)>,
    history: Vec<openatat_ipc::HistoryRecord>,
    status: SharedString,
    confirm_clear: bool,
    focus_handle: FocusHandle,
}

impl Shell {
    fn new(page: UiPage, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let edit = AgentEdit::load_default_path();
        let argv = cx.new(|cx| {
            LineEditor::new(
                cx,
                edit.argv_as_lines(),
                "one argv element per line (not a shell string)",
                true,
                "argv",
            )
        });
        let search =
            cx.new(|cx| LineEditor::new(cx, String::new(), "search prompts…", false, "search"));
        let annotate_path = cx.new(|cx| {
            LineEditor::new(
                cx,
                String::new(),
                "local PNG or JPEG path…",
                false,
                "annotate-path",
            )
        });
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window);
        let tab = match page {
            UiPage::Settings | UiPage::Studio => Tab::Settings,
            UiPage::History => Tab::History,
        };
        let mut shell = Self {
            tab,
            edit,
            argv,
            search,
            annotate_path,
            detected: detect_on_path(std::env::var_os("PATH").as_deref()),
            history: history::load_newest_first(&crate::paths::history_path()),
            status: SharedString::from(match page {
                UiPage::Settings | UiPage::Studio => {
                    "Settings — save writes ~/.config/openatat/agent.toml"
                }
                UiPage::History => "History — prompts only; responses are not stored",
            }),
            confirm_clear: false,
            focus_handle,
        };
        shell.reload_history();
        shell
    }

    fn reload_history(&mut self) {
        self.history = history::load_newest_first(&crate::paths::history_path());
    }

    fn sync_argv_from_field(&mut self, cx: &App) {
        self.edit.set_argv_from_lines(&read_content(&self.argv, cx));
    }

    fn pick_provider(&mut self, id: &str, cx: &mut Context<Self>) {
        self.sync_argv_from_field(cx);
        apply_provider_pick(&mut self.edit, id);
        let lines = self.edit.argv_as_lines();
        self.argv.update(cx, |ed, cx| ed.set_content(lines, cx));
        self.status = SharedString::from(format!("Provider → {}", self.edit.provider));
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        self.sync_argv_from_field(cx);
        let path = crate::paths::agent_config_path();
        match settings::save_to(&path, &self.edit) {
            Ok(()) => {
                self.status = SharedString::from(format!("Saved {}", path.display()));
            }
            Err(e) => {
                self.status = SharedString::from(format!("Save failed: {e}"));
            }
        }
        cx.notify();
    }

    fn reuse(&mut self, prompt: &str, cx: &mut Context<Self>) {
        let text = crate::emit_reuse(prompt);
        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
        println!("{text}");
        self.status =
            SharedString::from("Reused prompt (copied + printed). Agent output is not stored.");
        cx.notify();
    }

    fn clear_history(&mut self, cx: &mut Context<Self>) {
        if !self.confirm_clear {
            self.confirm_clear = true;
            self.status = SharedString::from("Click Clear History again to confirm.");
            cx.notify();
            return;
        }
        match history::clear(&crate::paths::history_path()) {
            Ok(()) => {
                self.history.clear();
                self.confirm_clear = false;
                self.status = SharedString::from("History cleared.");
            }
            Err(e) => {
                self.status = SharedString::from(format!("Clear failed: {e}"));
            }
        }
        cx.notify();
    }
}

impl Focusable for Shell {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Shell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let q = read_content(&self.search, cx);
        let filtered: Vec<_> = history::search(&self.history, &q)
            .into_iter()
            .cloned()
            .collect();

        div()
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x141518))
            .text_color(rgb(0xe8e8ea))
            .font_family("sans-serif")
            .child(self.render_chrome(cx))
            .child(
                div()
                    .id("body")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .p_4()
                    .child(match self.tab {
                        Tab::Settings => self.render_settings(cx).into_any_element(),
                        Tab::History => self.render_history(&filtered, cx).into_any_element(),
                        Tab::Permissions => self.render_permissions().into_any_element(),
                    }),
            )
            .child(self.render_status())
    }
}

impl Shell {
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
                    .child("OpenAtat"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0x8b8f99))
                    .child("settings · history · studio  ·  not the @@ overlay"),
            )
            .child(div().flex_1())
            .child(tab_btn(
                "Settings",
                self.tab == Tab::Settings,
                cx,
                Tab::Settings,
            ))
            .child(tab_btn(
                "History",
                self.tab == Tab::History,
                cx,
                Tab::History,
            ))
            .child(tab_btn(
                "Permissions",
                self.tab == Tab::Permissions,
                cx,
                Tab::Permissions,
            ))
    }

    fn render_status(&self) -> impl IntoElement {
        div()
            .px_4()
            .py_2()
            .text_xs()
            .text_color(rgb(0x9aa0a6))
            .border_t_1()
            .border_color(rgb(0x2c2d33))
            .child(self.status.clone())
    }

    fn render_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let provider = self.edit.provider.clone();
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(heading("Provider"))
            .child(div().text_sm().text_color(rgb(0x9aa0a6)).child(
                "auto = first binary on PATH. argv is a list, never a shell string. \
                         {prompt} is one element; {prompt_file} is a temp file; neither = stdin.",
            ))
            .child(div().flex().flex_row().flex_wrap().gap_2().children(
                PROVIDER_CHOICES.iter().copied().map(|id| {
                    let on = provider == id;
                    let id_owned = id.to_string();
                    chip(id, on, cx, move |this, cx| {
                        this.pick_provider(&id_owned, cx)
                    })
                }),
            ))
            .child(heading("Detected on PATH"))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(self.detected.iter().map(|(name, path)| {
                        let line = match path {
                            Some(p) => format!("{name}  →  {}", p.display()),
                            None => format!("{name}  —  not found"),
                        };
                        div()
                            .text_sm()
                            .text_color(if path.is_some() {
                                rgb(0xb8e0c0)
                            } else {
                                rgb(0x7a7e88)
                            })
                            .child(line)
                    })),
            )
            .child(heading("Argv template"))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x9aa0a6))
                    .child("One argv element per line. Empty = use the provider default."),
            )
            .child(self.argv.clone())
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .child(action_btn(
                        "Save agent.toml",
                        rgb(0x3d5a9c),
                        cx,
                        |this, cx| {
                            this.save(cx);
                        },
                    ))
                    .child(action_btn("Reload", rgb(0x3a3b42), cx, |this, cx| {
                        this.edit = AgentEdit::load_default_path();
                        let lines = this.edit.argv_as_lines();
                        this.argv.update(cx, |ed, cx| ed.set_content(lines, cx));
                        this.status = SharedString::from("Reloaded from disk.");
                        cx.notify();
                    })),
            )
            .child(div().text_xs().text_color(rgb(0x6d717a)).child(format!(
                "Writes {}  ·  unknown keys and comments are kept",
                crate::paths::agent_config_path().display()
            )))
            .child(heading("Tools"))
            .child(div().text_sm().text_color(rgb(0x9aa0a6)).child(
                "Open Annotate… takes a local PNG or JPEG. Nothing is uploaded. \
                 Video trim / Combine Images stay later.",
            ))
            .child(self.annotate_path.clone())
            .child(action_btn(
                "Open Annotate…",
                rgb(0x3d5a9c),
                cx,
                |this, cx| this.open_annotate(cx),
            ))
    }

    fn open_annotate(&mut self, cx: &mut Context<Self>) {
        let raw = read_content(&self.annotate_path, cx);
        let path = PathBuf::from(raw.trim());
        match crate::studio::Document::open(&path) {
            Ok(_) => {
                studio_ui::open(cx, Some(path));
                self.status = SharedString::from("Opened studio (local file, not uploaded)");
            }
            Err(e) => {
                self.status = SharedString::from(e);
            }
        }
        cx.notify();
    }

    fn render_history(
        &self,
        rows: &[openatat_ipc::HistoryRecord],
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(heading("History"))
            .child(div().text_sm().text_color(rgb(0x9aa0a6)).child(
                "Newest first. Each row is id / timestamp / entry / prompt. \
                         Screenshots and agent replies are not in this file and are never shown.",
            ))
            .child(self.search.clone())
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .child(action_btn("Refresh", rgb(0x3a3b42), cx, |this, cx| {
                        this.reload_history();
                        this.status = SharedString::from(format!("{} entries", this.history.len()));
                        cx.notify();
                    }))
                    .child(action_btn(
                        if self.confirm_clear {
                            "Confirm clear"
                        } else {
                            "Clear History"
                        },
                        rgb(0x8a3a3a),
                        cx,
                        |this, cx| this.clear_history(cx),
                    )),
            )
            .child(div().text_xs().text_color(rgb(0x6d717a)).child(format!(
                "{} shown  ·  {}",
                rows.len(),
                crate::paths::history_path().display()
            )))
            .child(if rows.is_empty() {
                div()
                    .text_sm()
                    .text_color(rgb(0x7a7e88))
                    .child("No matching prompts.")
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(rows.iter().cloned().map(|rec| history_row(rec, cx)))
                    .into_any_element()
            })
    }

    fn render_permissions(&self) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(heading("Linux permissions"))
            .child(div().text_sm().text_color(rgb(0x9aa0a6)).child(
                "Each grant is optional — deny one and the rest still works. \
                         This window only documents them. It does not request OS permissions \
                         (no portal prompts, no Accessibility dialogs).",
            ))
            .children(LINUX_GRANTS.iter().map(|g| grant_card(g)))
            .child(heading("macOS permissions"))
            .child(div().text_sm().text_color(rgb(0x9aa0a6)).child(
                "TCC grants are optional. Input Monitoring, Accessibility, \
                         Screen Recording, and Finder Automation can each be denied; \
                         --demo and the unix socket still work.",
            ))
            .children(MAC_GRANTS.iter().map(|g| grant_card(g)))
            .child(heading("Windows permissions"))
            .child(div().text_sm().text_color(rgb(0x9aa0a6)).child(
                "Each grant is optional. WGC privacy consent can be denied; \
                         --demo and the trigger socket still work. \
                         GraphicsCapturePicker is never used.",
            ))
            .children(WIN_GRANTS.iter().map(|g| grant_card(g)))
    }
}

fn grant_card(g: &'static crate::permissions::PermissionGrant) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .p_3()
        .rounded_md()
        .bg(rgb(0x1c1d22))
        .border_1()
        .border_color(rgb(0x2c2d33))
        .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(g.name))
        .child(div().text_sm().child(g.unlocks))
        .child(div().text_xs().text_color(rgb(0x8b8f99)).child(g.how))
}

fn heading(text: &'static str) -> impl IntoElement {
    div()
        .text_sm()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(rgb(0xc5c8d0))
        .child(text)
}

fn tab_btn(label: &'static str, on: bool, cx: &mut Context<Shell>, tab: Tab) -> impl IntoElement {
    div()
        .id(label)
        .cursor(CursorStyle::PointingHand)
        .px_3()
        .py_1()
        .rounded_md()
        .bg(if on { rgb(0x2d3d66) } else { rgb(0x2a2b31) })
        .text_sm()
        .on_click(cx.listener(move |this, _, _, cx| {
            this.tab = tab;
            this.confirm_clear = false;
            if tab == Tab::History {
                this.reload_history();
            }
            cx.notify();
        }))
        .child(label)
}

fn chip(
    label: &'static str,
    on: bool,
    cx: &mut Context<Shell>,
    on_click: impl Fn(&mut Shell, &mut Context<Shell>) + 'static,
) -> impl IntoElement {
    div()
        .id(label)
        .cursor(CursorStyle::PointingHand)
        .px_3()
        .py_1()
        .rounded_md()
        .bg(if on { rgb(0x3d5a9c) } else { rgb(0x2a2b31) })
        .text_sm()
        .on_click(cx.listener(move |this, _, _, cx| on_click(this, cx)))
        .child(label)
}

fn action_btn(
    label: &'static str,
    color: gpui::Rgba,
    cx: &mut Context<Shell>,
    on_click: impl Fn(&mut Shell, &mut Context<Shell>) + 'static,
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

fn history_row(rec: openatat_ipc::HistoryRecord, cx: &mut Context<Shell>) -> impl IntoElement {
    let prompt = rec.prompt.clone();
    let id = rec.id.clone();
    div()
        .id(SharedString::from(id.clone()))
        .flex()
        .flex_col()
        .gap_1()
        .p_3()
        .rounded_md()
        .bg(rgb(0x1c1d22))
        .border_1()
        .border_color(rgb(0x2c2d33))
        .child(
            div()
                .flex()
                .flex_row()
                .gap_3()
                .text_xs()
                .text_color(rgb(0x8b8f99))
                .child(format_timestamp(&rec.timestamp))
                .child(entry_label(&rec))
                .child(id),
        )
        .child(div().text_sm().child(rec.prompt.clone()))
        .child(action_btn("Reuse", rgb(0x3d5a9c), cx, move |this, cx| {
            this.reuse(&prompt, cx);
        }))
}
