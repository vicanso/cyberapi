//! A key / value list (F4.10): query, headers, auth, form fields. A blank
//! row is always last, and typing in it adds the next; each row can be
//! turned off or removed, the second click on its remove button removing it.
//! Changes are reported 200 ms after the last one.

use crate::this_build;
use cyberapi_core::model::KvParam;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
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
    /// Values shown as dots: Basic auth's password.
    pub masked: bool,
    /// Rows can take a file (multipart, F4.8), where files can be read.
    pub files: bool,
}

struct Row {
    enabled: bool,
    key: Entity<InputState>,
    value: Entity<InputState>,
    _subscriptions: [Subscription; 2],
}

pub struct KvEditor {
    rows: Vec<Row>,
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
            options,
            report: None,
            removing: None,
        };
        editor.set_list(list, window, cx);
        editor
    }

    /// Shows `list` in place of what the rows hold, without reporting it.
    pub fn set_list(&mut self, list: &[KvParam], window: &mut Window, cx: &mut Context<Self>) {
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

    fn push_row(&mut self, kv: &KvParam, window: &mut Window, cx: &mut Context<Self>) {
        let key = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("editor.key").to_string())
                .default_value(kv.key.clone())
        });
        let masked = self.options.masked;
        let value = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("editor.value").to_string())
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
        let last = self.rows.len().saturating_sub(1);
        let removing = self.removing.as_ref().map(|(index, _)| *index);
        let files = self.options.files && this_build::reads_files();
        let theme = cx.theme();
        let (border, muted) = (theme.border, theme.muted_foreground);
        // A table: the fields sit in its cells without frames of their own,
        // the lines between the cells drawing the grid.
        let cell = move || div().h_full().flex().items_center().border_l_1().border_color(border);
        let actions = move || {
            h_flex()
                .w(px(if files { 56. } else { 32. }))
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
                    .child(cell().w_2_5().flex_none().px_2().child(t!("editor.key").to_string()))
                    .child(cell().flex_1().min_w_0().px_2().child(t!("editor.value").to_string())),
            )
            .child(actions());
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
    }
}
