//! An API's assertions (A1): one a row — what of the answer, the header or
//! path when it names one, how it compares, and with what — each marked
//! with how the response the panel shows holds up to it. Under them, what a
//! run takes from the answer for the requests after it (A2): a name, what
//! of the answer, and what it would take from the response shown.
//!
//! A6: a schema is no one line: its row opens it in a dialog's editor.

use crate::states::ApiState;
use crate::views::name_dialog::ok_button;
use crate::views::settings::Choice;
use cyberapi_core::assertions::{Assertion, Checked, Extraction, Op, Problem, Subject, TYPES};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::{DialogClose, DialogFooter};
use gpui_kit::component::input::{Editor, EditorState, Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, IndexPath, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px,
};
use rust_i18n::t;

type Choices = Entity<SelectState<Vec<Choice>>>;

/// A6: where a new schema starts.
const SCHEMA_EXAMPLE: &str = "{\n    \"type\": \"object\",\n    \"required\": [\"id\"],\n    \"properties\": {\n        \"id\": { \"type\": \"integer\" }\n    }\n}\n";

pub(super) fn subject_label(subject: Subject) -> String {
    match subject {
        Subject::Status => t!("assertions.status"),
        Subject::Header => t!("assertions.header"),
        Subject::Json => t!("assertions.json"),
        Subject::Body => t!("assertions.body"),
        Subject::Latency => t!("assertions.latency"),
        Subject::Size => t!("assertions.size"),
        Subject::Schema => t!("assertions.schema"),
    }
    .to_string()
}

pub(super) fn op_label(op: Op) -> String {
    match op {
        Op::Equals => t!("assertions.equals"),
        Op::NotEquals => t!("assertions.not_equals"),
        Op::Contains => t!("assertions.contains"),
        Op::NotContains => t!("assertions.not_contains"),
        Op::Matches => t!("assertions.matches"),
        Op::Exists => t!("assertions.exists"),
        Op::NotExists => t!("assertions.not_exists"),
        Op::Less => t!("assertions.less"),
        Op::Greater => t!("assertions.greater"),
        Op::IsType => t!("assertions.is_type"),
        Op::Conforms => t!("assertions.conforms"),
    }
    .to_string()
}

/// Why a check could not be made, said.
pub(super) fn problem_text(problem: &Problem) -> String {
    match problem {
        Problem::NoResponse => t!("assertions.no_response").to_string(),
        Problem::NotJson => t!("assertions.not_json").to_string(),
        Problem::Path(why) => t!("assertions.bad_path", why = why).to_string(),
        Problem::Pattern(why) => t!("assertions.bad_pattern", why = why).to_string(),
        Problem::NotANumber => t!("assertions.not_a_number").to_string(),
        Problem::UnknownType => t!("assertions.unknown_type", types = TYPES.join(", ")).to_string(),
        Problem::NoTarget => t!("assertions.no_target").to_string(),
        Problem::Schema(why) => t!("assertions.bad_schema", why = why).to_string(),
    }
}

/// What was there, said: the value, or that nothing was.
pub(super) fn actual_text(checked: &Checked) -> String {
    match &checked.actual {
        Some(actual) => t!("assertions.actual", value = actual).to_string(),
        None => t!("assertions.absent").to_string(),
    }
}

fn target_placeholder(subject: Subject) -> &'static str {
    match subject {
        Subject::Header => "Content-Type",
        _ => "$.data.id",
    }
}

fn expected_placeholder(subject: Subject, op: Op) -> String {
    match op {
        Op::Matches => "^ok$".into(),
        Op::IsType => TYPES.join(" / "),
        Op::Less | Op::Greater if subject == Subject::Latency => "500".into(),
        _ if subject == Subject::Status => "200".into(),
        _ => t!("assertions.value").to_string(),
    }
}

struct Row {
    /// Stays with the row as others come and go.
    id: usize,
    enabled: bool,
    subject: Subject,
    op: Op,
    subjects: Choices,
    ops: Choices,
    target: Entity<InputState>,
    expected: Entity<InputState>,
    /// A6: the schema's text, edited in a dialog.
    schema: String,
    /// A6: an import's, until the reader edits it.
    imported: bool,
    _subscriptions: Vec<Subscription>,
    /// The comparison list's, made again with it when the subject changes.
    _ops: Subscription,
}

/// A2: an extraction's row.
struct Take {
    id: usize,
    enabled: bool,
    subject: Subject,
    subjects: Choices,
    name: Entity<InputState>,
    target: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

pub struct AssertionsEditor {
    api: Entity<ApiState>,
    rows: Vec<Row>,
    takes: Vec<Take>,
    next: usize,
    _subscriptions: Vec<Subscription>,
}

impl AssertionsEditor {
    pub fn new(api: Entity<ApiState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let assertions = api.read(cx).request().assertions.clone();
        let extractions = api.read(cx).request().extractions.clone();
        // A new response, or assertions changed outside (F11: Docs response
        // schema): keep rows in step and redraw marks.
        let subscriptions = vec![cx.observe_in(&api, window, |this, api, window, cx| {
            let latest = api.read(cx).request().assertions.clone();
            if this.assertions(cx) != latest {
                this.rows.clear();
                for assertion in &latest {
                    this.push_row(assertion, window, cx);
                }
            }
            let latest_takes = api.read(cx).request().extractions.clone();
            if this.extractions(cx) != latest_takes {
                this.takes.clear();
                for extraction in &latest_takes {
                    this.push_take(extraction, window, cx);
                }
            }
            cx.notify();
        })];
        let mut editor = Self {
            api,
            rows: Vec::new(),
            takes: Vec::new(),
            next: 0,
            _subscriptions: subscriptions,
        };
        for assertion in &assertions {
            editor.push_row(assertion, window, cx);
        }
        for extraction in &extractions {
            editor.push_take(extraction, window, cx);
        }
        editor
    }

    fn push_take(&mut self, extraction: &Extraction, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.next;
        self.next += 1;
        let subject_index = Subject::EXTRACTED
            .iter()
            .position(|each| *each == extraction.subject)
            .unwrap_or(0);
        let subjects = Self::choices(
            Subject::EXTRACTED
                .iter()
                .map(|subject| subject_label(*subject))
                .collect(),
            subject_index,
            window,
            cx,
        );
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("token")
                .default_value(extraction.name.clone())
        });
        let target = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(target_placeholder(extraction.subject))
                .default_value(extraction.target.clone())
        });
        let changed = |this: &mut Self, _: Entity<InputState>, event: &InputEvent, cx: &mut Context<Self>| {
            if matches!(event, InputEvent::Change) {
                this.write_takes(cx);
            }
        };
        let subscriptions = vec![
            cx.subscribe_in(
                &subjects,
                window,
                move |this, _, event: &SelectEvent<Vec<Choice>>, window, cx| {
                    if let SelectEvent::Confirm(Some(index)) = event
                        && let Some(subject) = Subject::EXTRACTED.get(*index)
                        && let Some(take) = this.takes.iter_mut().find(|take| take.id == id)
                    {
                        take.subject = *subject;
                        take.target.update(cx, |input, cx| {
                            input.set_placeholder(target_placeholder(*subject), window, cx)
                        });
                        this.write_takes(cx);
                    }
                },
            ),
            cx.subscribe(&name, changed),
            cx.subscribe(&target, changed),
        ];
        self.takes.push(Take {
            id,
            enabled: extraction.enabled,
            subject: extraction.subject,
            subjects,
            name,
            target,
            _subscriptions: subscriptions,
        });
    }

    /// The extractions, as the API keeps them.
    fn extractions(&self, cx: &App) -> Vec<Extraction> {
        self.takes.iter().map(|take| Self::extraction(take, cx)).collect()
    }

    fn extraction(take: &Take, cx: &App) -> Extraction {
        Extraction {
            enabled: take.enabled,
            name: take.name.read(cx).value().trim().to_string(),
            subject: take.subject,
            target: if take.subject.has_target() {
                take.target.read(cx).value().trim().to_string()
            } else {
                String::new()
            },
        }
    }

    fn write_takes(&mut self, cx: &mut Context<Self>) {
        let extractions = self.extractions(cx);
        self.api.update(cx, |api, cx| api.set_extractions(extractions, cx));
        cx.notify();
    }

    /// What the row would take from the response shown, said.
    fn taken(&self, take: &Take, cx: &App) -> Option<(String, bool)> {
        let extraction = Self::extraction(take, cx);
        if !extraction.enabled || (extraction.subject.has_target() && extraction.target.is_empty()) {
            return None;
        }
        match self.api.read(cx).taken(&extraction)? {
            Ok(Some(value)) => Some((t!("assertions.taken", value = value).to_string(), true)),
            Ok(None) => Some((t!("assertions.absent").to_string(), false)),
            Err(problem) => Some((problem_text(&problem), false)),
        }
    }

    fn render_takes(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted, warning) = (theme.border, theme.muted_foreground, theme.warning);
        let rows: Vec<AnyElement> = self
            .takes
            .iter()
            .map(|take| {
                let id = take.id;
                let taken = self.taken(take, cx);
                v_flex()
                    .id(("extraction", id))
                    .py_1()
                    .gap_0p5()
                    .border_t_1()
                    .border_color(border)
                    .child(
                        h_flex()
                            .gap_1p5()
                            .items_center()
                            .child(
                                Checkbox::new(("extraction-enabled", id))
                                    .checked(take.enabled)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(take) = this.takes.iter_mut().find(|take| take.id == id) {
                                            take.enabled = !take.enabled;
                                        }
                                        this.write_takes(cx);
                                    })),
                            )
                            .child(
                                h_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1p5()
                                    .when(!take.enabled, |this| this.opacity(0.45))
                                    .child(div().w(px(130.)).flex_none().child(Input::new(&take.name).small()))
                                    .child(div().w(px(120.)).flex_none().child(Select::new(&take.subjects).small()))
                                    .when(take.subject.has_target(), |this| {
                                        this.child(div().flex_1().min_w_0().child(Input::new(&take.target).small()))
                                    })
                                    .when(!take.subject.has_target(), |this| this.child(div().flex_1())),
                            )
                            .child(
                                Button::new(("extraction-remove", id))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Close)
                                    .tooltip(SharedString::from(t!("common.delete").to_string()))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.takes.retain(|take| take.id != id);
                                        this.write_takes(cx);
                                    })),
                            ),
                    )
                    .when_some(taken, |this, (text, found)| {
                        this.child(
                            div()
                                .pl_7()
                                .text_xs()
                                .truncate()
                                .text_color(if found { muted } else { warning })
                                .child(text),
                        )
                    })
                    .into_any_element()
            })
            .collect();
        let empty = rows.is_empty();
        v_flex()
            .gap_2()
            .pt_2()
            .child(
                div()
                    .text_sm()
                    .font_semibold()
                    .child(t!("assertions.extract").to_string()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(t!("assertions.extract_hint").to_string()),
            )
            .when(!empty, |this| {
                this.child(v_flex().border_b_1().border_color(border).children(rows))
            })
            .child(
                h_flex().child(
                    Button::new("extraction-add")
                        .ghost()
                        .small()
                        .icon(IconName::Plus)
                        .label(t!("assertions.add_extract").to_string())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.push_take(&Extraction::default(), window, cx);
                            this.write_takes(cx);
                        })),
                ),
            )
            .into_any_element()
    }

    fn choices(labels: Vec<String>, chosen: usize, window: &mut Window, cx: &mut Context<Self>) -> Choices {
        let items = labels
            .into_iter()
            .enumerate()
            .map(|(index, label)| Choice {
                label: label.into(),
                index,
            })
            .collect();
        cx.new(|cx| SelectState::new(items, Some(IndexPath::new(chosen)), window, cx))
    }

    fn op_choices(
        id: usize,
        subject: Subject,
        op: Op,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Choices, Subscription) {
        let ops = subject.ops();
        let chosen = ops.iter().position(|each| *each == op).unwrap_or(0);
        let state = Self::choices(ops.iter().map(|op| op_label(*op)).collect(), chosen, window, cx);
        let subscription = cx.subscribe_in(
            &state,
            window,
            move |this, _, event: &SelectEvent<Vec<Choice>>, window, cx| {
                if let SelectEvent::Confirm(Some(index)) = event {
                    this.set_op(id, *index, window, cx);
                }
            },
        );
        (state, subscription)
    }

    fn push_row(&mut self, assertion: &Assertion, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.next;
        self.next += 1;
        let subject_index = Subject::ALL
            .iter()
            .position(|each| *each == assertion.subject)
            .unwrap_or(0);
        let subjects = Self::choices(
            Subject::ALL.iter().map(|subject| subject_label(*subject)).collect(),
            subject_index,
            window,
            cx,
        );
        let (ops, ops_subscription) = Self::op_choices(id, assertion.subject, assertion.op, window, cx);
        let target = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(target_placeholder(assertion.subject))
                .default_value(assertion.target.clone())
        });
        let schema = assertion.subject == Subject::Schema;
        let expected = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(expected_placeholder(assertion.subject, assertion.op))
                .default_value(if schema {
                    String::new()
                } else {
                    assertion.expected.clone()
                })
        });
        let changed = |this: &mut Self, _: Entity<InputState>, event: &InputEvent, cx: &mut Context<Self>| {
            if matches!(event, InputEvent::Change) {
                this.write(cx);
            }
        };
        let subscriptions = vec![
            cx.subscribe_in(
                &subjects,
                window,
                move |this, _, event: &SelectEvent<Vec<Choice>>, window, cx| {
                    if let SelectEvent::Confirm(Some(index)) = event
                        && let Some(subject) = Subject::ALL.get(*index)
                    {
                        this.set_subject(id, *subject, window, cx);
                    }
                },
            ),
            cx.subscribe(&target, changed),
            cx.subscribe(&expected, changed),
        ];
        self.rows.push(Row {
            id,
            enabled: assertion.enabled,
            subject: assertion.subject,
            op: assertion.op,
            subjects,
            ops,
            target,
            expected,
            schema: if schema {
                assertion.expected.clone()
            } else {
                String::new()
            },
            imported: assertion.imported,
            _subscriptions: subscriptions,
            _ops: ops_subscription,
        });
    }

    /// A6: the row's schema in a dialog's editor; Save keeps it. An
    /// import's becomes the reader's own once changed.
    fn edit_schema(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.iter().find(|row| row.id == id) else {
            return;
        };
        let imported = row.imported;
        let text = match row.schema.trim() {
            "" => SCHEMA_EXAMPLE.to_string(),
            _ => row.schema.clone(),
        };
        let editor = cx.new(|cx| {
            crate::views::fix_editor_search(cx);
            EditorState::new(window, cx)
                .language("json")
                .line_number(true)
                .soft_wrap(false)
                .tab_size(crate::views::json_tab())
                .default_value(text.clone())
        });
        let this = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let save = {
                let (editor, this, before) = (editor.clone(), this.clone(), text.clone());
                move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut App| {
                    let written = editor.read(cx).value().to_string();
                    let _ = this.update(cx, |this, cx| {
                        if let Some(row) = this.rows.iter_mut().find(|row| row.id == id) {
                            if written != before {
                                row.imported = false;
                            }
                            row.schema = written;
                        }
                        this.write(cx);
                    });
                    true
                }
            };
            let muted = cx.theme().muted_foreground;
            dialog
                .title(t!("assertions.schema_title").to_string())
                .w(px(640.))
                .child(crate::views::dialog_body(
                    v_flex()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .child(t!("assertions.schema_hint").to_string()),
                        )
                        .when(imported, |this| {
                            this.child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(t!("assertions.schema_imported").to_string()),
                            )
                        })
                        .child(
                            div()
                                .h(px(360.))
                                .border_1()
                                .border_color(cx.theme().border)
                                .rounded_md()
                                .child(Editor::new(&editor).h_full().bordered(false)),
                        ),
                ))
                .footer(
                    DialogFooter::new()
                        .child(DialogClose::new().trigger(|button| button.label(t!("common.cancel").to_string())))
                        .child(ok_button("assertion-schema-save", t!("common.save").to_string(), save)),
                )
                // Enter is the editor's.
                .on_ok(|_, _, _| false)
        });
    }

    /// The comparisons follow the subject: the one picked stays when the
    /// new subject has it.
    fn set_subject(&mut self, id: usize, subject: Subject, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.iter().find(|row| row.id == id) else {
            return;
        };
        if row.subject == subject {
            return;
        }
        let op = if subject.ops().contains(&row.op) {
            row.op
        } else {
            subject.ops()[0]
        };
        let (ops, subscription) = Self::op_choices(id, subject, op, window, cx);
        let Some(row) = self.rows.iter_mut().find(|row| row.id == id) else {
            return;
        };
        row.subject = subject;
        row.op = op;
        row.ops = ops;
        row._ops = subscription;
        // Changed, it is the reader's: an update from the import's source
        // does not take it away (BUG-11).
        row.imported = false;
        row.target.update(cx, |input, cx| {
            input.set_placeholder(target_placeholder(subject), window, cx)
        });
        row.expected.update(cx, |input, cx| {
            input.set_placeholder(expected_placeholder(subject, op), window, cx)
        });
        self.write(cx);
    }

    fn set_op(&mut self, id: usize, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.iter_mut().find(|row| row.id == id) else {
            return;
        };
        let Some(op) = row.subject.ops().get(index).copied() else {
            return;
        };
        if row.op != op {
            row.imported = false;
        }
        row.op = op;
        let subject = row.subject;
        row.expected.update(cx, |input, cx| {
            input.set_placeholder(expected_placeholder(subject, op), window, cx)
        });
        self.write(cx);
    }

    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.push_row(&Assertion::default(), window, cx);
        self.write(cx);
    }

    fn remove(&mut self, id: usize, cx: &mut Context<Self>) {
        self.rows.retain(|row| row.id != id);
        self.write(cx);
    }

    fn toggle(&mut self, id: usize, cx: &mut Context<Self>) {
        if let Some(row) = self.rows.iter_mut().find(|row| row.id == id) {
            row.enabled = !row.enabled;
            // Turned off or on by the reader, theirs to keep so (BUG-11).
            row.imported = false;
        }
        self.write(cx);
    }

    /// The rows, as the API keeps them.
    fn assertions(&self, cx: &App) -> Vec<Assertion> {
        self.rows
            .iter()
            .map(|row| Assertion {
                enabled: row.enabled,
                subject: row.subject,
                target: if row.subject.has_target() {
                    row.target.read(cx).value().trim().to_string()
                } else {
                    String::new()
                },
                op: row.op,
                expected: if row.subject == Subject::Schema {
                    row.schema.clone()
                } else if row.op.takes_value() {
                    row.expected.read(cx).value().to_string()
                } else {
                    String::new()
                },
                imported: row.imported,
            })
            .collect()
    }

    fn write(&mut self, cx: &mut Context<Self>) {
        let assertions = self.assertions(cx);
        self.api.update(cx, |api, cx| api.set_assertions(assertions, cx));
        cx.notify();
    }

    /// How the row's assertion fared: nothing with no response, for a row
    /// turned off, or one whose value is not written yet.
    fn outcome(&self, index: usize, row: &Row, cx: &App) -> Option<Result<Checked, Problem>> {
        let expected = match row.subject {
            Subject::Schema => row.schema.trim().is_empty(),
            _ => row.op.takes_value() && row.expected.read(cx).value().trim().is_empty(),
        };
        if expected {
            return None;
        }
        let checks = self.api.read(cx).checks()?;
        checks
            .iter()
            .find(|(at, _)| *at == index)
            .map(|(_, outcome)| outcome.clone())
    }

    /// The row's mark: held, failed, or could not be checked; what was
    /// there, or why, on hover.
    fn mark(index: usize, outcome: &Result<Checked, Problem>, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let (icon, color, tip) = match outcome {
            Ok(checked) if checked.passed => (IconName::CircleCheck, theme.green, actual_text(checked)),
            Ok(checked) => (IconName::CircleX, theme.danger, actual_text(checked)),
            Err(problem) => (IconName::TriangleAlert, theme.warning, problem_text(problem)),
        };
        div()
            .id(("assertion-mark", index))
            .flex_none()
            .child(Icon::new(icon).small().text_color(color))
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .into_any_element()
    }
}

impl Render for AssertionsEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted) = (theme.border, theme.muted_foreground);
        let rows: Vec<AnyElement> = self
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let id = row.id;
                let outcome = self.outcome(index, row, cx);
                // A failed one says what was there under it, and one that
                // could not be checked why.
                let said = match &outcome {
                    Some(Ok(checked)) if !checked.passed => Some((actual_text(checked), cx.theme().danger)),
                    Some(Err(problem)) => Some((problem_text(problem), cx.theme().warning)),
                    _ => None,
                };
                let controls = h_flex()
                    .gap_1p5()
                    .items_center()
                    .child(
                        Checkbox::new(("assertion-enabled", id))
                            .checked(row.enabled)
                            .on_click(cx.listener(move |this, _, _, cx| this.toggle(id, cx))),
                    )
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1p5()
                            .when(!row.enabled, |this| this.opacity(0.45))
                            .child(div().w(px(120.)).flex_none().child(Select::new(&row.subjects).small()))
                            .when(row.subject.has_target(), |this| {
                                this.child(div().flex_1().min_w_0().child(Input::new(&row.target).small()))
                            })
                            .child(div().w(px(110.)).flex_none().child(Select::new(&row.ops).small()))
                            // A6: the schema opens in a dialog.
                            .when(row.subject == Subject::Schema, |this| {
                                let label = match row.schema.trim().is_empty() {
                                    true => t!("assertions.schema_write"),
                                    false => t!("assertions.schema_edit"),
                                };
                                this.child(
                                    h_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .gap_1p5()
                                        .items_center()
                                        .child(
                                            Button::new(("assertion-schema", id))
                                                .outline()
                                                .xsmall()
                                                .label(label.to_string())
                                                .on_click(cx.listener(move |this, _, window, cx| {
                                                    this.edit_schema(id, window, cx)
                                                })),
                                        )
                                        .when(row.imported, |this| {
                                            this.child(
                                                div()
                                                    .text_xs()
                                                    .text_color(muted)
                                                    .child(t!("assertions.from_import").to_string()),
                                            )
                                        }),
                                )
                            })
                            .when(row.subject != Subject::Schema && row.op.takes_value(), |this| {
                                this.child(div().flex_1().min_w_0().child(Input::new(&row.expected).small()))
                            })
                            // Room for the inputs left by what a row has not.
                            .when(!row.subject.has_target() && !row.op.takes_value(), |this| {
                                this.child(div().flex_1())
                            }),
                    )
                    .child(
                        div()
                            .w(px(16.))
                            .flex_none()
                            .children(outcome.as_ref().map(|outcome| Self::mark(index, outcome, cx))),
                    )
                    .child(
                        Button::new(("assertion-remove", id))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Close)
                            .tooltip(SharedString::from(t!("common.delete").to_string()))
                            .on_click(cx.listener(move |this, _, _, cx| this.remove(id, cx))),
                    );
                v_flex()
                    .id(("assertion", id))
                    .py_1()
                    .gap_0p5()
                    .border_t_1()
                    .border_color(border)
                    .child(controls)
                    .when_some(said, |this, (text, color)| {
                        this.child(div().pl_7().text_xs().text_color(color).child(text))
                    })
                    .into_any_element()
            })
            .collect();
        let empty = rows.is_empty();
        v_flex()
            .id("request-assertions")
            .size_full()
            .overflow_y_scrollbar()
            .px_3()
            .py_2()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(t!("assertions.hint").to_string()),
            )
            .when(!empty, |this| {
                this.child(v_flex().border_b_1().border_color(border).children(rows))
            })
            .child(
                h_flex().child(
                    Button::new("assertion-add")
                        .ghost()
                        .small()
                        .icon(IconName::Plus)
                        .label(t!("assertions.add").to_string())
                        .on_click(cx.listener(|this, _, window, cx| this.add(window, cx))),
                ),
            )
            .child(self.render_takes(cx))
    }
}
