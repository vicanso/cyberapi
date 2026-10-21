//! A key / value list (F4.10): query, headers, auth, form fields. A blank
//! row is always last, and typing in it adds the next; each row can be
//! turned off or removed, the second click on its remove button removing it.
//! Changes are reported 200 ms after the last one.
//!
//! The list can also be edited as text, a `key: value` a line (`kv_text`),
//! which takes a block of headers pasted whole.

use crate::this_build;
use cyberapi_core::model::{KvParam, kv_from_text, kv_text};
use gpui_kit::base::input::Position;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Editor, EditorState, Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement, ParentElement as _,
    PathPromptOptions, Render, SharedString, Styled as _, Subscription, Task, Window, div, px,
};
use rust_i18n::t;
use std::time::Duration;

const REPORT_DELAY: Duration = Duration::from_millis(200);
/// How long a first click on remove waits for the second.
const REMOVE_WAIT: Duration = Duration::from_secs(3);

pub enum KvEvent {
    Changed(Vec<KvParam>),
}

/// What kind of list it is.
#[derive(Clone, Copy, Default)]
pub struct KvOptions {
    /// Basic auth's: the columns are user and password, the password shown
    /// as dots.
    pub masked: bool,
    /// Rows can take a file (multipart, F4.8), where files can be read.
    pub files: bool,
    /// Rows offered ready made, name and a start of the value: the
    /// headers one types most.
    pub presets: &'static [(&'static str, &'static str)],
}

struct Row {
    enabled: bool,
    key: Entity<InputState>,
    value: Entity<InputState>,
    _subscriptions: [Subscription; 2],
}

/// The list as text, while it is edited so.
struct Bulk {
    text: Entity<EditorState>,
    _events: Subscription,
}

pub struct KvEditor {
    rows: Vec<Row>,
    bulk: Option<Bulk>,
    options: KvOptions,
    report: Option<Task<()>>,
    /// The row whose remove button was clicked once, and the wait for the
    /// second click.
    removing: Option<(usize, Task<()>)>,
}

impl EventEmitter<KvEvent> for KvEditor {}

impl KvEditor {
    pub fn new(list: &[KvParam], options: KvOptions, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut editor = Self {
            rows: Vec::new(),
            bulk: None,
            options,
            report: None,
            removing: None,
        };
        editor.set_list(list, window, cx);
        editor
    }

    /// Shows `list` in place of what the rows hold, without reporting it.
    pub fn set_list(&mut self, list: &[KvParam], window: &mut Window, cx: &mut Context<Self>) {
        self.bulk = None;
        self.rows.clear();
        for kv in list {
            self.push_row(kv, window, cx);
        }
        self.push_row(
            &KvParam {
                enabled: true,
                ..Default::default()
            },
            window,
            cx,
        );
        self.report = None;
        self.removing = None;
        cx.notify();
    }

    /// What the two columns are called.
    fn labels(&self) -> (String, String) {
        if self.options.masked {
            (t!("editor.user").to_string(), t!("editor.password").to_string())
        } else {
            (t!("editor.key").to_string(), t!("editor.value").to_string())
        }
    }

    fn push_row(&mut self, kv: &KvParam, window: &mut Window, cx: &mut Context<Self>) {
        let (key_label, value_label) = self.labels();
        let key = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(key_label)
                .default_value(kv.key.clone())
        });
        let masked = self.options.masked;
        let value = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(value_label)
                .masked(masked)
                .default_value(kv.value.clone())
        });
        let watch = |input: &Entity<InputState>, cx: &mut Context<Self>| {
            cx.subscribe_in(input, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.edited(window, cx);
                }
            })
        };
        let subscriptions = [watch(&key, cx), watch(&value, cx)];
        self.rows.push(Row {
            enabled: kv.enabled,
            key,
            value,
            _subscriptions: subscriptions,
        });
    }

    /// The rows as a list; the blank last one is not part of it.
    pub fn list(&self, cx: &gpui_kit::App) -> Vec<KvParam> {
        if let Some(bulk) = &self.bulk {
            return kv_from_text(&bulk.text.read(cx).value());
        }
        self.rows
            .iter()
            .map(|row| KvParam {
                key: row.key.read(cx).value().to_string(),
                value: row.value.read(cx).value().to_string(),
                enabled: row.enabled,
            })
            .filter(|kv| !kv.key.is_empty() || !kv.value.is_empty())
            .collect()
    }

    fn edited(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Typing in the blank row makes the next one.
        if let Some(last) = self.rows.last()
            && (!last.key.read(cx).value().is_empty() || !last.value.read(cx).value().is_empty())
        {
            self.push_row(
                &KvParam {
                    enabled: true,
                    ..Default::default()
                },
                window,
                cx,
            );
        }
        self.report_later(cx);
    }

    fn report_later(&mut self, cx: &mut Context<Self>) {
        self.removing = None;
        cx.notify();
        self.report = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REPORT_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                let list = this.list(cx);
                cx.emit(KvEvent::Changed(list));
            });
        }));
    }

    fn toggle(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(row) = self.rows.get_mut(index) {
            row.enabled = !row.enabled;
            self.report_later(cx);
        }
    }

    /// F4.10: the first click asks, the second removes.
    fn remove(&mut self, index: usize, cx: &mut Context<Self>) {
        if index + 1 >= self.rows.len() {
            return;
        }
        if matches!(self.removing, Some((asked, _)) if asked == index) {
            self.rows.remove(index);
            self.report_later(cx);
            return;
        }
        let wait = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REMOVE_WAIT).await;
            let _ = this.update(cx, |this, cx| {
                this.removing = None;
                cx.notify();
            });
        });
        self.removing = Some((index, wait));
        cx.notify();
    }

    /// A preset row, before the blank one, its value's end taking the keys:
    /// the rest of it is the reader's to type.
    fn add_preset(&mut self, key: &str, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        let at = self.rows.len().saturating_sub(1);
        self.push_row(
            &KvParam {
                key: key.into(),
                value: value.into(),
                enabled: true,
            },
            window,
            cx,
        );
        if let Some(row) = self.rows.pop() {
            let input = row.value.downgrade();
            self.rows.insert(at, row);
            // Once the row is drawn: a field not drawn yet loses focus.
            window.on_next_frame(move |window, cx| {
                if let Some(input) = input.upgrade() {
                    input.update(cx, |input, cx| {
                        // The presets' values are ASCII: characters are columns.
                        let end = input.value().chars().count() as u32;
                        input.set_cursor_position(Position::new(0, end), window, cx);
                    });
                }
            });
        }
        self.report_later(cx);
    }

    /// From rows to text and back; the text, read, becomes the rows.
    fn toggle_bulk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.bulk.is_some() {
            let list = self.list(cx);
            self.set_list(&list, window, cx);
            // What was typed last may not have been reported yet.
            self.report_later(cx);
            return;
        }
        let text = kv_text(&self.list(cx));
        let text = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("text")
                .soft_wrap(true)
                .placeholder(t!("editor.bulk_placeholder").to_string())
                .default_value(text)
        });
        text.update(cx, |text, cx| text.focus(window, cx));
        let events = cx.subscribe(&text, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.report_later(cx);
            }
        });
        self.bulk = Some(Bulk { text, _events: events });
        self.removing = None;
        cx.notify();
    }

    /// F4.8: a file for a multipart field, as `file://` and its path.
    fn pick_file(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                if let Some(row) = this.rows.get(index) {
                    let value = format!("file://{}", path.display());
                    row.value.update(cx, |input, cx| input.set_value(value, window, cx));
                    this.edited(window, cx);
                }
            });
        })
        .detach();
    }
}

impl Render for KvEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A password's list is not shown as text.
        let toggle = (!self.options.masked).then(|| {
            Button::new("kv-bulk")
                .ghost()
                .xsmall()
                .icon(if self.bulk.is_some() {
                    gpui_kit::assets::IconName::Table
                } else {
                    gpui_kit::assets::IconName::FileText
                })
                .tooltip(if self.bulk.is_some() {
                    t!("editor.table_edit").to_string()
                } else {
                    t!("editor.bulk_edit").to_string()
                })
                .on_click(cx.listener(|this, _, window, cx| this.toggle_bulk(window, cx)))
        });
        if let Some(bulk) = &self.bulk {
            let theme = cx.theme();
            return v_flex()
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.border)
                .overflow_hidden()
                .child(
                    h_flex()
                        .h(px(26.))
                        .pl_2()
                        .pr_0p5()
                        .bg(theme.muted)
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(div().flex_1().child(t!("editor.bulk_hint").to_string()))
                        .children(toggle),
                )
                .child(
                    div()
                        .h(px(240.))
                        .border_t_1()
                        .border_color(theme.border)
                        .child(Editor::new(&bulk.text).w_full().h_full().bordered(false)),
                )
                .into_any_element();
        }
        let (key_label, value_label) = self.labels();
        let last = self.rows.len().saturating_sub(1);
        // A row of the common ones, from a menu.
        let presets = (!self.options.presets.is_empty()).then(|| {
            let (view, presets) = (cx.entity().downgrade(), self.options.presets);
            Button::new("kv-presets")
                .ghost()
                .xsmall()
                .icon(gpui_kit::assets::IconName::ListPlus)
                .tooltip(t!("editor.common_headers").to_string())
                .dropdown_menu_with_anchor(gpui_kit::Anchor::TopRight, move |menu, _, _| {
                    presets.iter().fold(menu, |menu, (key, value)| {
                        let view = view.clone();
                        menu.item(
                            PopupMenuItem::new(format!("{key}: {value}")).on_click(move |_, window, cx| {
                                let _ = view.update(cx, |this, cx| this.add_preset(key, value, window, cx));
                            }),
                        )
                    })
                })
        });
        let removing = self.removing.as_ref().map(|(index, _)| *index);
        let files = self.options.files && this_build::reads_files();
        let theme = cx.theme();
        let (border, muted) = (theme.border, theme.muted_foreground);
        // A table: the fields sit in its cells without frames of their own,
        // the lines between the cells drawing the grid.
        let cell = move || div().h_full().flex().items_center().border_l_1().border_color(border);
        // Room for the file button on a row, or the presets' in the header.
        let wide = files || !self.options.presets.is_empty();
        let actions = move || {
            h_flex()
                .w(px(if wide { 56. } else { 32. }))
                .flex_none()
                .justify_end()
                .pr_0p5()
        };
        let header = h_flex()
            .h(px(26.))
            .bg(theme.muted)
            .text_xs()
            .text_color(muted)
            .child(div().w_8().flex_none())
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(cell().w_2_5().flex_none().px_2().child(key_label))
                    .child(cell().flex_1().min_w_0().px_2().child(value_label)),
            )
            .child(actions().children(presets).children(toggle));
        let rows = self.rows.iter().enumerate().map(|(index, row)| {
            let blank = index == last;
            let remove = if removing == Some(index) {
                Button::new(("kv-remove", index))
                    .danger()
                    .xsmall()
                    .icon(gpui_kit::assets::IconName::Trash)
                    .tooltip(SharedString::from(t!("editor.remove_again").to_string()))
                    .on_click(cx.listener(move |this, _, _, cx| this.remove(index, cx)))
            } else {
                Button::new(("kv-remove", index))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Close)
                    .tooltip(SharedString::from(t!("common.delete").to_string()))
                    .on_click(cx.listener(move |this, _, _, cx| this.remove(index, cx)))
            };
            h_flex()
                .id(("kv-row", index))
                .h(px(30.))
                .items_center()
                .border_t_1()
                .border_color(border)
                .child(div().w_8().flex_none().flex().justify_center().when(!blank, |this| {
                    this.child(
                        Checkbox::new(("kv-enabled", index))
                            .checked(row.enabled)
                            .on_click(cx.listener(move |this, _, _, cx| this.toggle(index, cx))),
                    )
                }))
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .when(!blank && !row.enabled, |this| this.opacity(0.45))
                        .child(
                            cell()
                                .w_2_5()
                                .flex_none()
                                .child(Input::new(&row.key).small().appearance(false)),
                        )
                        .child(
                            cell()
                                .flex_1()
                                .min_w_0()
                                .child(Input::new(&row.value).small().appearance(false)),
                        ),
                )
                .child(
                    actions()
                        .when(files && !blank, |this| {
                            this.child(
                                Button::new(("kv-file", index))
                                    .ghost()
                                    .xsmall()
                                    .icon(gpui_kit::assets::IconName::Paperclip)
                                    .tooltip(t!("editor.choose_file").to_string())
                                    .on_click(
                                        cx.listener(move |this, _, window, cx| this.pick_file(index, window, cx)),
                                    ),
                            )
                        })
                        .when(!blank, |this| this.child(remove)),
                )
        });
        v_flex()
            .rounded(theme.radius)
            .border_1()
            .border_color(border)
            .overflow_hidden()
            .child(header)
            .children(rows.collect::<Vec<_>>())
            .into_any_element()
    }
}
