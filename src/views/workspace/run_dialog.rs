//! A3: a folder's or the project's APIs run one after another, once or once
//! for each row of a data file — pasted, or picked on the desktop — with
//! the `prompt`s they ask for answered first, in the dialog: one dialog
//! over another is not drawn in a page. Each send shows as it ends: its
//! answer, how its assertions held, what it took for the ones after it.
//! Closing the dialog stops the run; a send picked opens its API there.

use super::response_panel::HistoryRow;
use crate::states::{RunState, WorkspaceState};
use crate::this_build;
use crate::views::inline_confirm::inline_confirm;
use cyberapi_core::ItemId;
use cyberapi_core::assertions::tally;
use cyberapi_core::response::format_duration;
use cyberapi_core::run::{self as rules, DataError, MAX_DELAY_MS, MAX_RETRIES, RunOptions};
use cyberapi_service::StepResult;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::input::{Editor, EditorState, Input, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ListSizingBehavior,
    ParentElement as _, PathPromptOptions, Render, StatefulInteractiveElement as _, Styled as _, Subscription, Window,
    div, px, uniform_list,
};
use rust_i18n::t;

/// Opens the run of `folder`, or of the whole project.
pub fn open_run_dialog(workspace: Entity<WorkspaceState>, folder: Option<ItemId>, window: &mut Window, cx: &mut App) {
    let title = {
        let workspace = workspace.read(cx);
        let name = match &folder {
            Some(folder) => workspace
                .tree()
                .get(folder)
                .map(|node| node.name().to_string())
                .unwrap_or_default(),
            None => workspace.collection().name.clone(),
        };
        t!("run.title", name = name).to_string()
    };
    let view = cx.new(|cx| RunView::new(workspace, folder, window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title(title.clone())
            .w(px(760.))
            .on_ok(|_, _, _| false)
            .child(view.clone())
            .footer(
                DialogFooter::new().child(
                    Button::new("run-close")
                        .label(t!("common.close").to_string())
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                ),
            )
    });
}

struct RunView {
    workspace: Entity<WorkspaceState>,
    run: Entity<RunState>,
    data: Entity<EditorState>,
    /// A9: pause between sends (ms), retries per failed send, stop on failure.
    delay_ms: Entity<InputState>,
    retries: Entity<InputState>,
    bail: bool,
    /// D1: a field for each label the run's requests ask for.
    prompts: Vec<(String, Entity<InputState>)>,
    /// Why the data or options did not read, or a picked file could not be.
    error: Option<String>,
    /// D6: asking whether a run in production should start.
    confirming: bool,
    _observe: Subscription,
}

impl RunView {
    fn new(
        workspace: Entity<WorkspaceState>,
        folder: Option<ItemId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let run = cx.new(|cx| RunState::new(workspace.clone(), folder, cx));
        let typed = crate::states::typed_prompts(&workspace.read(cx).collection().id, cx);
        let prompts = run
            .read(cx)
            .prompts(cx)
            .into_iter()
            .map(|prompt| {
                let start = typed.get(&prompt.label).cloned().or(prompt.start).unwrap_or_default();
                let input = cx.new(|cx| InputState::new(window, cx).default_value(start));
                (prompt.label, input)
            })
            .collect();
        let data = cx.new(|cx| {
            crate::views::fix_editor_search(cx);
            EditorState::new(window, cx)
                .soft_wrap(false)
                .placeholder(t!("run.data_placeholder").to_string())
        });
        let delay_ms = cx.new(|cx| InputState::new(window, cx).placeholder("0").default_value("0"));
        let retries = cx.new(|cx| InputState::new(window, cx).placeholder("0").default_value("0"));
        let observe = cx.observe(&run, |_, _, cx| cx.notify());
        Self {
            workspace,
            run,
            data,
            delay_ms,
            retries,
            bail: false,
            prompts,
            error: None,
            confirming: false,
            _observe: observe,
        }
    }

    fn options(&self, cx: &App) -> Result<RunOptions, String> {
        let read = |input: &Entity<InputState>, most: u32, cx: &App| -> Result<u32, String> {
            let text = input.read(cx).value().trim().to_string();
            if text.is_empty() {
                return Ok(0);
            }
            text.parse::<u32>()
                .ok()
                .filter(|value| *value <= most)
                .ok_or_else(|| t!("run.bad_number", most = most).to_string())
        };
        Ok(RunOptions {
            delay_ms: read(&self.delay_ms, MAX_DELAY_MS, cx)?,
            retries: read(&self.retries, MAX_RETRIES, cx)?,
            bail: self.bail,
        })
    }

    /// D6: whether starting is asked about first: in production, some of
    /// what runs may change something there.
    fn asks_first(&self, cx: &App) -> bool {
        let workspace = self.workspace.read(cx);
        let apis = &workspace.tree_data().apis;
        self.run.read(cx).planned().iter().any(|planned| {
            apis.iter()
                .find(|api| api.id == planned.api)
                .is_some_and(|api| workspace.asks_before(&api.request, cx))
        })
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        if let Err(why) = self.options(cx) {
            self.error = Some(why);
            cx.notify();
            return;
        }
        if self.asks_first(cx) {
            self.confirming = true;
            cx.notify();
            return;
        }
        self.start_unasked(cx);
    }

    fn start_unasked(&mut self, cx: &mut Context<Self>) {
        self.confirming = false;
        let options = match self.options(cx) {
            Ok(options) => options,
            Err(why) => {
                self.error = Some(why);
                cx.notify();
                return;
            }
        };
        let data = self.data.read(cx).value().to_string();
        let prompts = self
            .prompts
            .iter()
            .map(|(label, input)| (label.clone(), input.read(cx).value().to_string()))
            .collect();
        let started = self.run.update(cx, |run, cx| run.start(&data, prompts, options, cx));
        self.error = started.err().map(|error| data_error(&error));
        cx.notify();
    }

    fn export_report(&self, html: bool, window: &mut Window, cx: &mut Context<Self>) {
        let (suite, cases) = self.run.read(cx).report_cases(cx);
        if cases.is_empty() {
            return;
        }
        let stem = if suite.trim().is_empty() {
            "run-report".to_string()
        } else {
            suite.trim().to_string()
        };
        let (name, text) = if html {
            (format!("{stem}-report.html"), rules::html_report(&suite, &cases))
        } else {
            (format!("{stem}-junit.xml"), rules::junit(&suite, &cases))
        };
        let saving = cx.background_spawn(async move { this_build::save_to_downloads(&name, text.as_bytes()) });
        window
            .spawn(cx, async move |cx| {
                let saved = saving.await;
                let _ = cx.update(|window, cx| {
                    let note = match saved {
                        Ok(path) => Notification::success(t!("export.saved", path = path).to_string()),
                        Err(message) => Notification::error(message),
                    };
                    window.push_notification(note, cx);
                });
            })
            .detach();
    }

    fn pick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let read = cx.background_spawn(async move { this_build::read_text(&path) }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match read {
                    Ok(text) => {
                        this.error = None;
                        this.data.update(cx, |data, cx| data.set_value(text, window, cx));
                    }
                    Err(message) => this.error = Some(message),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn summary(&self, cx: &App) -> String {
        let run = self.run.read(cx);
        let total = run.planned().len() * run.passes();
        let elapsed = run.elapsed().map(format_duration).unwrap_or_default();
        if run.running() {
            return t!("run.running", done = run.results().len(), total = total, time = elapsed).to_string();
        }
        if run.results().is_empty() {
            return crate::i18n::counted("run.planned", run.planned().len());
        }
        let (held, failed) = run.tally();
        t!("run.done", held = held, failed = failed, time = elapsed).to_string()
    }

    /// A step's row: its answer in two lines, then a line of what it noted
    /// — always there, so every row is as tall and the list draws only the
    /// rows in view (PERF-5).
    fn row(&self, index: usize, result: &StepResult, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let (muted, danger, success) = (theme.muted_foreground, theme.danger, theme.success);
        let run = self.run.read(cx);
        let planned = run.planned().get(result.step).cloned();
        let name = planned.as_ref().map(|planned| planned.name.clone()).unwrap_or_default();
        let passes = run.passes();
        let running = run.running();
        let pass = (passes > 1).then(|| format!("#{}", result.pass + 1));
        let (passed, total) = tally(&result.outcomes);
        let mut notes: Vec<(String, gpui_kit::Hsla)> = Vec::new();
        if total > 0 {
            let color = if passed == total { success } else { danger };
            notes.push((
                t!("assertions.passed", passed = passed, total = total).to_string(),
                color,
            ));
        }
        if result.attempts > 1 {
            notes.push((t!("run.attempts", count = result.attempts).to_string(), muted));
        }
        if !result.extracted.is_empty() {
            let names: Vec<&str> = result.extracted.iter().map(|(name, _)| name.as_str()).collect();
            notes.push((t!("run.took", names = names.join(", ")).to_string(), muted));
        }
        let body: AnyElement = match &result.response {
            Ok(response) => {
                if let Some(error) = &response.error {
                    notes.push((error.clone(), danger));
                }
                let today = chrono::Local::now().date_naive();
                HistoryRow::of(response, today, cx)
                    .render(Some(&name), 640., cx)
                    .into_any_element()
            }
            Err(why) => v_flex()
                .child(
                    h_flex()
                        .gap_3()
                        .child(
                            div()
                                .font_semibold()
                                .text_color(danger)
                                .child(t!("run.not_sent").to_string()),
                        )
                        .child(div().min_w_0().truncate().child(name.clone())),
                )
                .child(div().text_xs().truncate().text_color(danger).child(why.clone()))
                .into_any_element(),
        };
        let open = result.response.as_ref().ok().map(|response| response.id.clone());
        let (workspace, api) = (self.workspace.clone(), planned.map(|planned| planned.api));
        let hover = cx.theme().accent;
        let notes = if notes.is_empty() {
            // As tall as a line of notes.
            vec![div().child("\u{a0}")]
        } else {
            notes
                .into_iter()
                .map(|(text, color)| div().min_w_0().truncate().text_color(color).child(text))
                .collect()
        };
        h_flex()
            .id(("run-row", index))
            .w_full()
            .px_2()
            .py_1p5()
            .gap_2()
            .items_start()
            .rounded_md()
            .text_sm()
            .when_some(pass, |row, pass| {
                row.child(div().w(px(36.)).flex_none().text_color(muted).child(pass))
            })
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(body)
                    .child(h_flex().gap_3().text_xs().overflow_hidden().children(notes)),
            )
            // Picked once the run is over: closing the dialog would stop it.
            .when(!running && open.is_some(), |row| {
                row.cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .on_click(move |_, window, cx| {
                        if let (Some(api), Some(id)) = (&api, &open) {
                            workspace.update(cx, |workspace, cx| workspace.open_response(api, id, cx));
                            window.close_dialog(cx);
                        }
                    })
            })
            .into_any_element()
    }
}

fn data_error(error: &DataError) -> String {
    match error {
        DataError::Json { message } => t!("run.data_json", why = message).to_string(),
        DataError::NotRows => t!("run.data_not_rows").to_string(),
        DataError::Quote { line } => t!("run.data_quote", line = line).to_string(),
        DataError::Width { line, found, named } => {
            t!("run.data_width", line = line, found = found, named = named).to_string()
        }
    }
}

impl Render for RunView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, danger, border) = (theme.muted_foreground, theme.danger, theme.border);
        let running = self.run.read(cx).running();
        let empty = self.run.read(cx).planned().is_empty();
        let summary = self.summary(cx);
        let count = self.run.read(cx).results().len();
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
            Button::new("run-stop")
                .outline()
                .label(t!("run.stop").to_string())
                .on_click(cx.listener(|this, _, _, cx| this.run.update(cx, |run, cx| run.stop(cx))))
        } else {
            Button::new("run-start")
                .primary()
                .icon(gpui_kit::assets::IconName::Play)
                .label(t!("run.start").to_string())
                .disabled(empty)
                .on_click(cx.listener(|this, _, _, cx| this.start(cx)))
        };
        v_flex()
            .gap_3()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .items_center()
                            .child(div().flex_1().text_sm().child(t!("run.data").to_string()))
                            .when(this_build::reads_files(), |this| {
                                this.child(
                                    Button::new("run-pick")
                                        .ghost()
                                        .xsmall()
                                        .label(t!("import.choose_file").to_string())
                                        .on_click(cx.listener(|this, _, window, cx| this.pick(window, cx))),
                                )
                            }),
                    )
                    .child(
                        div()
                            .h(px(96.))
                            .border_1()
                            .border_color(border)
                            .rounded_md()
                            .child(Editor::new(&self.data).w_full().h_full().bordered(false)),
                    )
                    .child(div().text_xs().text_color(muted).child(t!("run.data_hint").to_string()))
                    .when_some(self.error.clone(), |this, error| {
                        this.child(div().text_xs().text_color(danger).child(error))
                    }),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .text_sm()
                    .child(t!("run.delay_ms").to_string())
                    .child(div().w(px(72.)).child(Input::new(&self.delay_ms).small()))
                    .child(div().text_color(muted).child("ms"))
                    .child(div().w(px(12.)))
                    .child(t!("run.retries").to_string())
                    .child(div().w(px(56.)).child(Input::new(&self.retries).small()))
                    .child(div().w(px(12.)))
                    .child(
                        Checkbox::new("run-bail")
                            .checked(self.bail)
                            .label(t!("run.bail").to_string())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.bail = !this.bail;
                                cx.notify();
                            })),
                    ),
            )
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
                            .child(div().flex_1().text_sm().text_color(muted).child(summary))
                            .when(!running && count > 0, |row| {
                                row.child(
                                    Button::new("run-export-junit")
                                        .outline()
                                        .xsmall()
                                        .label(t!("run.export_junit").to_string())
                                        .on_click(
                                            cx.listener(|this, _, window, cx| this.export_report(false, window, cx)),
                                        ),
                                )
                                .child(
                                    Button::new("run-export-html")
                                        .outline()
                                        .xsmall()
                                        .label(t!("run.export_html").to_string())
                                        .on_click(
                                            cx.listener(|this, _, window, cx| this.export_report(true, window, cx)),
                                        ),
                                )
                            }),
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
                    t!("production.run_ask", environment = environment).to_string(),
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
            .when(count > 0, |this| {
                this.child(
                    div().border_t_1().border_color(border).pt_1().child(
                        uniform_list(
                            "run-results",
                            count,
                            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                                let run = this.run.read(cx);
                                let results = run.results().get(range.clone()).unwrap_or_default();
                                results
                                    .iter()
                                    .zip(range)
                                    .map(|(result, index)| this.row(index, result, cx))
                                    .collect::<Vec<_>>()
                            }),
                        )
                        .with_sizing_behavior(ListSizingBehavior::Infer)
                        .max_h(px(360.)),
                    ),
                )
            })
    }
}
