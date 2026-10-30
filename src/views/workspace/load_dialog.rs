//! A7: an API sent over and over by a number of senders at once, a number
//! of times or for a while (desktop only), and what came of it as it goes:
//! sends a second, the error rate, the time they took — mean and
//! percentiles — and how they ended. The `prompt`s it asks for are
//! answered in the dialog (one over it is not drawn in a page). Closing the
//! dialog stops it; nothing goes into the history.

use crate::states::{LoadState, WorkspaceState};
use crate::views::inline_confirm::inline_confirm;
use cyberapi_core::ItemId;
use cyberapi_core::load::{Limit, MAX_COUNT, MAX_SECONDS, MAX_SENDERS, Outcome, Plan, Summary};
use cyberapi_core::response::{format_duration, format_size};
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{
    ActiveTheme as _, Selectable as _, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    Subscription, Window, div, px, relative,
};
use rust_i18n::t;

/// How many of the ways the sends ended are listed.
const OUTCOMES_SHOWN: usize = 6;

/// Opens the load test of `api`.
pub fn open_load_dialog(
    workspace: Entity<WorkspaceState>,
    api: ItemId,
    name: String,
    window: &mut Window,
    cx: &mut App,
) {
    let title = t!("load.title", name = name).to_string();
    let view = cx.new(|cx| LoadView::new(workspace, api, window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title(title.clone())
            .w(px(640.))
            .on_ok(|_, _, _| false)
            .child(view.clone())
            .footer(
                DialogFooter::new().child(
                    Button::new("load-close")
                        .label(t!("common.close").to_string())
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                ),
            )
    });
}

struct LoadView {
    workspace: Entity<WorkspaceState>,
    load: Entity<LoadState>,
    senders: Entity<InputState>,
    count: Entity<InputState>,
    seconds: Entity<InputState>,
    /// Whether it ends after a while rather than after so many.
    timed: bool,
    /// D1: a field for each label its request asks for.
    prompts: Vec<(String, Entity<InputState>)>,
    /// What does not read in the plan.
    error: Option<String>,
    /// D6: asking whether a test in production should start.
    confirming: bool,
    _observe: Subscription,
}

impl LoadView {
    fn new(workspace: Entity<WorkspaceState>, api: ItemId, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let load = cx.new(|cx| LoadState::new(workspace.clone(), api, cx));
        let typed = crate::states::typed_prompts(&workspace.read(cx).collection().id, cx);
        let prompts = load
            .read(cx)
            .prompts(cx)
            .into_iter()
            .map(|prompt| {
                let start = typed.get(&prompt.label).cloned().or(prompt.start).unwrap_or_default();
                let input = cx.new(|cx| InputState::new(window, cx).default_value(start));
                (prompt.label, input)
            })
            .collect();
        let number = |value: &str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).default_value(value.to_string()))
        };
        let observe = cx.observe(&load, |_, _, cx| cx.notify());
        Self {
            workspace,
            senders: number("10", window, cx),
            count: number("100", window, cx),
            seconds: number("30", window, cx),
            load,
            timed: false,
            prompts,
            error: None,
            confirming: false,
            _observe: observe,
        }
    }

    /// The plan as written, or why it does not read.
    fn plan(&self, cx: &App) -> Result<Plan, String> {
        let read = |input: &Entity<InputState>, most: u32, cx: &App| {
            input
                .read(cx)
                .value()
                .trim()
                .parse::<u32>()
                .ok()
                .filter(|value| (1..=most).contains(value))
                .ok_or_else(|| t!("load.bad_number", most = most).to_string())
        };
        let senders = read(&self.senders, MAX_SENDERS, cx)?;
        let limit = match self.timed {
            true => Limit::Seconds(read(&self.seconds, MAX_SECONDS, cx)?),
            false => Limit::Count(read(&self.count, MAX_COUNT, cx)?),
        };
        Ok(Plan { senders, limit })
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        if let Err(why) = self.plan(cx) {
            self.error = Some(why);
            cx.notify();
            return;
        }
        if self.load.read(cx).asks_first(cx) {
            self.confirming = true;
            cx.notify();
            return;
        }
        self.start_unasked(cx);
    }

    fn start_unasked(&mut self, cx: &mut Context<Self>) {
        self.confirming = false;
        let plan = match self.plan(cx) {
            Ok(plan) => plan,
            Err(why) => {
                self.error = Some(why);
                cx.notify();
                return;
            }
        };
        self.error = None;
        let prompts = self
            .prompts
            .iter()
            .map(|(label, input)| (label.clone(), input.read(cx).value().to_string()))
            .collect();
        self.load.update(cx, |load, cx| load.start(plan, prompts, cx));
        cx.notify();
    }

    /// How far along it is, 0 to 1.
    fn progress(&self, cx: &App) -> f32 {
        let load = self.load.read(cx);
        let Some(plan) = load.plan() else {
            return 0.;
        };
        let done = match plan.limit {
            Limit::Count(count) => load.summary().sent as f32 / count.max(1) as f32,
            Limit::Seconds(seconds) => {
                load.elapsed().map(|elapsed| elapsed.as_secs_f32()).unwrap_or_default() / seconds.max(1) as f32
            }
        };
        if load.running() { done.min(1.) } else { 1. }
    }

    fn status(&self, cx: &App) -> String {
        let load = self.load.read(cx);
        let summary = load.summary();
        let elapsed = load.elapsed().map(format_duration).unwrap_or_default();
        match (load.running(), load.plan()) {
            (_, None) => t!("load.ready").to_string(),
            (true, _) => t!(
                "load.running",
                sent = summary.sent,
                failed = summary.failed,
                time = elapsed
            )
            .to_string(),
            (false, _) => t!(
                "load.done",
                sent = summary.sent,
                failed = summary.failed,
                time = elapsed
            )
            .to_string(),
        }
    }

    fn figures(summary: &Summary, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let (muted, border, danger) = (theme.muted_foreground, theme.border, theme.danger);
        let rate = summary.error_rate();
        let cells: Vec<(String, String, bool)> = vec![
            (
                t!("load.per_second").to_string(),
                format!("{:.1}", summary.per_second),
                false,
            ),
            (
                t!("load.error_rate").to_string(),
                format!("{:.1}%", rate * 100.),
                rate > 0.,
            ),
            (t!("load.mean").to_string(), format_duration(summary.mean), false),
            (t!("load.min").to_string(), format_duration(summary.min), false),
            ("P50".into(), format_duration(summary.p50), false),
            ("P90".into(), format_duration(summary.p90), false),
            ("P95".into(), format_duration(summary.p95), false),
            ("P99".into(), format_duration(summary.p99), false),
            (t!("load.max").to_string(), format_duration(summary.max), false),
            (t!("load.received").to_string(), format_size(summary.bytes), false),
        ];
        let rows: Vec<AnyElement> = cells
            .chunks(5)
            .map(|row| {
                h_flex()
                    .children(row.iter().map(|(label, value, bad)| {
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .px_2()
                            .py_1p5()
                            .child(div().text_xs().text_color(muted).child(label.clone()))
                            .child(
                                div()
                                    .text_sm()
                                    .font_semibold()
                                    .when(*bad, |this| this.text_color(danger))
                                    .child(value.clone()),
                            )
                    }))
                    .into_any_element()
            })
            .collect();
        v_flex()
            .border_1()
            .border_color(border)
            .rounded_md()
            .children(rows)
            .into_any_element()
    }

    fn outcomes(summary: &Summary, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let (muted, danger) = (theme.muted_foreground, theme.danger);
        v_flex()
            .gap_0p5()
            .children(summary.outcomes.iter().take(OUTCOMES_SHOWN).map(|(outcome, count)| {
                let (label, bad) = match outcome {
                    Outcome::Status(status) => (status.to_string(), *status >= 400),
                    // BUG-8: by the code's name, as the response panel says it.
                    Outcome::Grpc(code) => (
                        cyberapi_core::grpc::GrpcStatus {
                            code: *code,
                            message: String::new(),
                        }
                        .name()
                        .to_string(),
                        *code != 0,
                    ),
                    Outcome::Error(why) => (why.clone(), true),
                };
                h_flex()
                    .gap_2()
                    .text_xs()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .when(bad, |this| this.text_color(danger))
                            .child(label),
                    )
                    .child(div().flex_none().text_color(muted).child(format!("× {count}")))
            }))
            .into_any_element()
    }
}

impl Render for LoadView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, danger, border, primary) = (theme.muted_foreground, theme.danger, theme.border, theme.primary);
        let load = self.load.read(cx);
        let running = load.running();
        let tested = load.plan().is_some();
        let failure = load.failure().map(str::to_string);
        let summary = load.summary().clone();
        let status = self.status(cx);
        let progress = self.progress(cx);
        let prompts: Vec<AnyElement> = self
            .prompts
            .iter()
            .map(|(label, input)| {
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().w(px(160.)).flex_none().truncate().text_sm().child(label.clone()))
                    .child(div().flex_1().child(Input::new(input).small()))
                    .into_any_element()
            })
            .collect();
        let action = if running {
            Button::new("load-stop")
                .outline()
                .label(t!("run.stop").to_string())
                .on_click(cx.listener(|this, _, _, cx| this.load.update(cx, |load, cx| load.stop(cx))))
        } else {
            Button::new("load-start")
                .primary()
                .icon(gpui_kit::assets::IconName::Play)
                .label(t!("run.start").to_string())
                .on_click(cx.listener(|this, _, _, cx| this.start(cx)))
        };
        let amount = if self.timed { &self.seconds } else { &self.count };
        v_flex()
            .gap_3()
            .child(div().text_xs().text_color(muted).child(t!("load.hint").to_string()))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .text_sm()
                    .child(t!("load.senders").to_string())
                    .child(div().w(px(64.)).child(Input::new(&self.senders).small()))
                    .child(div().w(px(12.)))
                    .child(
                        [t!("load.by_count"), t!("load.by_time")]
                            .into_iter()
                            .enumerate()
                            .fold(
                                ButtonGroup::new("load-limit").compact().small().outline(),
                                |group, (index, label)| {
                                    group.child(
                                        Button::new(("load-limit", index))
                                            .label(label.to_string())
                                            .selected((index == 1) == self.timed),
                                    )
                                },
                            )
                            .on_click(cx.listener(|this, picks: &Vec<usize>, _, cx| {
                                this.timed = picks.first() == Some(&1);
                                cx.notify();
                            })),
                    )
                    .child(div().w(px(80.)).child(Input::new(amount).small()))
                    .child(
                        div().text_color(muted).child(
                            if self.timed {
                                t!("load.seconds")
                            } else {
                                t!("load.times")
                            }
                            .to_string(),
                        ),
                    ),
            )
            .when_some(self.error.clone(), |this, error| {
                this.child(div().text_xs().text_color(danger).child(error))
            })
            .when(!prompts.is_empty(), |this| {
                this.child(
                    v_flex()
                        .gap_1p5()
                        .child(div().text_sm().child(t!("editor.prompt_title").to_string()))
                        .children(prompts),
                )
            })
            .map(|this| {
                if !self.confirming {
                    return this.child(
                        h_flex()
                            .gap_3()
                            .items_center()
                            .child(action)
                            .child(div().text_sm().text_color(muted).child(status)),
                    );
                }
                // D6: asked in the dialog — one over it is not drawn in a page.
                let environment = self
                    .workspace
                    .read(cx)
                    .current_environment()
                    .map(|environment| environment.name.clone())
                    .unwrap_or_default();
                let (keep, go) = (cx.entity().downgrade(), cx.entity().downgrade());
                this.child(inline_confirm(
                    t!("production.title").to_string(),
                    t!("load.production_ask", environment = environment).to_string(),
                    None,
                    t!("run.start").to_string(),
                    move |_, cx| {
                        let _ = keep.update(cx, |this, cx| {
                            this.confirming = false;
                            cx.notify();
                        });
                    },
                    move |_, cx| {
                        let _ = go.update(cx, |this, cx| this.start_unasked(cx));
                    },
                    cx,
                ))
            })
            .when(tested, |this| {
                this.child(
                    div()
                        .h(px(4.))
                        .w_full()
                        .rounded_full()
                        .bg(border)
                        .child(div().h_full().rounded_full().bg(primary).w(relative(progress))),
                )
            })
            .when_some(failure, |this, failure| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(danger)
                        .child(t!("load.not_sent", why = failure).to_string()),
                )
            })
            .when(summary.sent > 0, |this| {
                this.child(Self::figures(&summary, cx))
                    .child(Self::outcomes(&summary, cx))
            })
    }
}
