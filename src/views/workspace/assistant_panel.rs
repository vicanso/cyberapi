//! H3: the assistant's column at the workspace's right
//! (`docs/ai-assistant.md` §2): the conversation, and where to ask.

use crate::states::{AppStore, AssistantState, Card, CardState, ChatLine, WorkspaceState};
use cyberapi_core::ai::Role;
use cyberapi_service::assistant::Proposal;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Editor, EditorState, InputEvent};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::text::TextView;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, IconName, Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    ScrollEvent, ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity,
    Window, div, px,
};
use rust_i18n::t;

/// How wide the column is.
pub const WIDTH: f32 = 380.;

/// The most lines of a change a card shows.
const CARD_ROWS: usize = 24;

pub struct AssistantPanel {
    workspace: WeakEntity<WorkspaceState>,
    assistant: Entity<AssistantState>,
    input: Entity<EditorState>,
    /// The conversation's scroll: it follows the answer while the reader
    /// is at its end.
    scroll: ScrollHandle,
    /// Whether the conversation follows the answer: from asking until the
    /// reader scrolls up, and again once they are back at the end.
    follow: bool,
    _subscriptions: Vec<Subscription>,
}

impl AssistantPanel {
    pub fn new(
        workspace: WeakEntity<WorkspaceState>,
        assistant: Entity<AssistantState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            crate::views::fix_editor_search(cx);
            EditorState::new(window, cx)
                .line_number(false)
                .soft_wrap(true)
                .placeholder(t!("assistant.placeholder").to_string())
        });
        let subscriptions = vec![
            cx.observe(&assistant, |_, _, cx| cx.notify()),
            cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { secondary: true, .. }) {
                    this.ask(window, cx);
                }
            }),
        ];
        Self {
            workspace,
            assistant,
            input,
            scroll: ScrollHandle::new(),
            follow: true,
            _subscriptions: subscriptions,
        }
    }

    fn ask(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let question = self.input.read(cx).value().to_string();
        if question.trim().is_empty() || self.assistant.read(cx).asking() {
            return;
        }
        self.follow = true;
        let _ = self
            .workspace
            .update(cx, |workspace, cx| workspace.ask_assistant(question, cx));
        self.input.update(cx, |input, cx| input.set_value("", window, cx));
    }

    fn line(&self, at: usize, line: &ChatLine, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        match line.role {
            Role::User => h_flex()
                .w_full()
                .justify_end()
                .child(
                    div()
                        .max_w(px(WIDTH * 0.8))
                        .px_3()
                        .py_2()
                        .rounded_lg()
                        .bg(theme.muted)
                        .text_sm()
                        .child(line.text.clone()),
                )
                .into_any_element(),
            Role::Assistant => {
                let body: AnyElement = if line.text.is_empty() && line.note.is_some() {
                    div().into_any_element()
                } else if line.text.is_empty() {
                    div()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(t!("assistant.thinking").to_string())
                        .into_any_element()
                } else {
                    TextView::markdown(("assistant-line", at), line.text.clone()).into_any_element()
                };
                let note = line.note.clone().map(|note| {
                    div()
                        .text_xs()
                        .text_color(if line.failed {
                            theme.danger
                        } else {
                            theme.muted_foreground
                        })
                        .child(note)
                });
                let read = (!line.read.is_empty()).then(|| {
                    div().text_xs().text_color(theme.muted_foreground).child(
                        t!(
                            "assistant.read",
                            what = line.read.join(t!("assistant.read_joiner").as_ref())
                        )
                        .to_string(),
                    )
                });
                let cards: Vec<AnyElement> = line
                    .cards
                    .iter()
                    .enumerate()
                    .map(|(index, card)| self.card((at, index), card, cx))
                    .collect();
                v_flex()
                    .w_full()
                    .gap_1()
                    .text_sm()
                    .when(line.failed, |this| this.pl_2().border_l_2().border_color(theme.danger))
                    .children(read)
                    .child(body)
                    .children(cards)
                    .children(note)
                    .into_any_element()
            }
        }
    }
}

impl AssistantPanel {
    /// H3d: a proposal, and what the reader may do with it.
    fn card(&self, at: (usize, usize), card: &Card, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let editable = self
            .workspace
            .upgrade()
            .is_some_and(|workspace| workspace.read(cx).editable());
        let (kind, name) = match &card.proposal {
            Proposal::Create { place, name, .. } => (
                t!("assistant.card_create"),
                if place.is_empty() {
                    name.clone()
                } else {
                    format!("{place}/{name}")
                },
            ),
            Proposal::Update { name, rename, .. } => (
                t!("assistant.card_update"),
                match rename {
                    Some(rename) => t!("assistant.card_rename", from = name, to = rename).to_string(),
                    None => name.clone(),
                },
            ),
            Proposal::Send { name, .. } => (t!("assistant.card_send"), name.clone()),
        };
        let sends = matches!(card.proposal, Proposal::Send { .. });
        let id = |what: &'static str| SharedString::from(format!("assistant-card-{what}-{}-{}", at.0, at.1));
        let workspace = self.workspace.clone();
        let apply = move |anyway: bool| {
            let workspace = workspace.clone();
            move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut gpui_kit::App| {
                let _ = workspace.update(cx, |workspace, cx| workspace.apply_card(at, anyway, cx));
            }
        };
        let discard = Button::new(id("discard"))
            .ghost()
            .xsmall()
            .label(t!("assistant.discard").to_string())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.assistant.update(cx, |assistant, cx| {
                    assistant.set_card_state(at, CardState::Discarded, cx)
                });
            }));
        let (status, failed, buttons): (Option<String>, bool, Vec<AnyElement>) = match &card.state {
            CardState::Open if sends => (
                None,
                false,
                vec![
                    Button::new(id("send"))
                        .primary()
                        .xsmall()
                        .label(t!("assistant.send").to_string())
                        .on_click(apply(false))
                        .into_any_element(),
                    discard.into_any_element(),
                ],
            ),
            CardState::Open => (
                (!editable).then(|| t!("assistant.viewer").to_string()),
                false,
                vec![
                    Button::new(id("apply"))
                        .primary()
                        .xsmall()
                        .disabled(!editable)
                        .label(t!("assistant.apply").to_string())
                        .on_click(apply(false))
                        .into_any_element(),
                    discard.into_any_element(),
                ],
            ),
            CardState::Changed => (
                Some(t!("assistant.changed").to_string()),
                true,
                vec![
                    Button::new(id("anyway"))
                        .xsmall()
                        .disabled(!editable)
                        .label(t!("assistant.anyway").to_string())
                        .on_click(apply(true))
                        .into_any_element(),
                    discard.into_any_element(),
                ],
            ),
            CardState::Failed(why) => (
                Some(t!("assistant.card_failed", why = why).to_string()),
                true,
                vec![
                    Button::new(id("retry"))
                        .xsmall()
                        .disabled(!editable)
                        .label(t!("assistant.retry").to_string())
                        .on_click(apply(false))
                        .into_any_element(),
                    discard.into_any_element(),
                ],
            ),
            CardState::Working => (Some(t!("assistant.working").to_string()), false, Vec::new()),
            CardState::Applied { .. } => {
                let workspace = self.workspace.clone();
                (
                    Some(t!("assistant.applied").to_string()),
                    false,
                    vec![
                        Button::new(id("undo"))
                            .ghost()
                            .xsmall()
                            .disabled(!editable)
                            .label(t!("assistant.undo").to_string())
                            .on_click(move |_, _, cx| {
                                let _ = workspace.update(cx, |workspace, cx| workspace.undo_card(at, cx));
                            })
                            .into_any_element(),
                    ],
                )
            }
            CardState::Undone => (Some(t!("assistant.undone").to_string()), false, Vec::new()),
            CardState::Discarded => (Some(t!("assistant.discarded").to_string()), false, Vec::new()),
            CardState::Sent => (Some(t!("assistant.sent").to_string()), false, Vec::new()),
        };
        let rows = card
            .compared
            .as_ref()
            .filter(|_| matches!(card.state, CardState::Open | CardState::Changed | CardState::Failed(_)))
            .map(|compared| {
                let more = compared.rows.len() > CARD_ROWS || compared.cut;
                v_flex()
                    .py_1()
                    .text_xs()
                    .font_family(theme.mono_font_family.clone())
                    .border_t_1()
                    .border_color(theme.border)
                    .children(super::versions_dialog::diff_rows(compared, CARD_ROWS, cx))
                    .when(more, |this| {
                        this.child(div().px_2().text_color(theme.muted_foreground).child("…"))
                    })
            });
        v_flex()
            .mt_1()
            .border_1()
            .border_color(theme.border)
            .rounded_md()
            .overflow_hidden()
            .child(
                h_flex()
                    .px_2()
                    .py_1p5()
                    .gap_2()
                    .bg(theme.muted)
                    .child(div().flex_none().text_xs().font_semibold().child(kind.to_string()))
                    .child(div().flex_1().min_w_0().truncate().text_xs().child(name)),
            )
            .children(rows)
            .child(
                h_flex()
                    .px_2()
                    .py_1p5()
                    .gap_2()
                    .items_center()
                    .border_t_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(if failed { theme.danger } else { theme.muted_foreground })
                            .children(status),
                    )
                    .children(buttons),
            )
            .into_any_element()
    }
}

impl Render for AssistantPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Every frame, not once a change: the Markdown is read after the
        // frame that shows it, and grows the conversation a frame later.
        if self.follow {
            self.scroll.scroll_to_bottom();
        }
        let theme = cx.theme();
        let (border, muted) = (theme.border, theme.muted_foreground);
        let model = AppStore::global(cx)
            .state()
            .read(cx)
            .ai_model()
            .unwrap_or_default()
            .to_string();
        let assistant = self.assistant.read(cx);
        let (asking, empty, with_response) = (
            assistant.asking(),
            assistant.lines().is_empty(),
            assistant.with_response(),
        );
        let lines: Vec<AnyElement> = assistant
            .lines()
            .iter()
            .enumerate()
            .map(|(at, line)| self.line(at, line, cx))
            .collect();
        let workspace = self.workspace.clone();
        v_flex()
            .size_full()
            // ⌘/Ctrl-Enter in an input of the workspace sends the open API
            // (`Workspace > Input`); in here it asks the model instead.
            .on_action(cx.listener(|this, _: &super::Send, window, cx| this.ask(window, cx)))
            .child(
                // As tall as the tree's head and the tabs bar beside it, so
                // their bottom line runs on.
                h_flex()
                    .h(px(36.))
                    .flex_none()
                    .px_3()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(border)
                    .child(div().text_sm().font_semibold().child(t!("assistant.title").to_string()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(muted)
                            .child(model),
                    )
                    .when(!empty, |this| {
                        this.child(
                            Button::new("assistant-clear")
                                .ghost()
                                .xsmall()
                                .label(t!("assistant.clear").to_string())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.assistant.update(cx, |assistant, cx| assistant.clear(cx));
                                })),
                        )
                    })
                    .child(
                        Button::new("assistant-close")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Close)
                            .tooltip(SharedString::from(t!("assistant.close").to_string()))
                            .on_click(move |_, _, cx| {
                                let _ = workspace.update(cx, |workspace, cx| workspace.toggle_assistant(cx));
                            }),
                    ),
            )
            .child(
                div()
                    .id("assistant-lines")
                    .flex_1()
                    .min_h_0()
                    .track_scroll(&self.scroll)
                    .overflow_y_scroll()
                    .on_scroll(cx.listener(|this, event: &ScrollEvent, _, _| {
                        // Up stops following; down to the end follows again.
                        let up = event.delta.pixel_delta(px(16.)).y > px(0.);
                        let at_end = this.scroll.max_offset().y + this.scroll.offset().y < px(24.);
                        this.follow = !up && at_end;
                    }))
                    .vertical_scrollbar(&self.scroll)
                    .child(
                        v_flex()
                            .p_3()
                            .gap_3()
                            .when(empty, |this| {
                                this.child(
                                    div()
                                        .text_sm()
                                        .text_color(muted)
                                        .child(t!("assistant.empty").to_string()),
                                )
                            })
                            .children(lines),
                    ),
            )
            .child(
                v_flex()
                    .flex_none()
                    .p_2()
                    .gap_2()
                    .border_t_1()
                    .border_color(border)
                    .child(
                        div()
                            .h(px(88.))
                            .border_1()
                            .border_color(border)
                            .rounded_md()
                            .child(Editor::new(&self.input).h_full().bordered(false)),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .child(
                                h_flex()
                                    .gap_3()
                                    .items_center()
                                    .child(
                                        Checkbox::new("assistant-with-response")
                                            .small()
                                            .checked(with_response)
                                            .label(t!("assistant.with_response").to_string())
                                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                                let checked = *checked;
                                                this.assistant.update(cx, |assistant, cx| {
                                                    assistant.set_with_response(checked, cx)
                                                });
                                            })),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(muted)
                                            .child(t!("assistant.hint").to_string()),
                                    ),
                            )
                            .child(if asking {
                                Button::new("assistant-stop")
                                    .small()
                                    .label(t!("assistant.stop").to_string())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.assistant.update(cx, |assistant, cx| assistant.stop(cx));
                                    }))
                            } else {
                                Button::new("assistant-ask")
                                    .primary()
                                    .small()
                                    .label(t!("assistant.ask").to_string())
                                    .on_click(cx.listener(|this, _, window, cx| this.ask(window, cx)))
                            }),
                    ),
            )
    }
}
