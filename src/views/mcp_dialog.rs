//! The reader's MCP tokens (the bridge's `/v1/mcp`, after zedis): what an AI
//! assistant signs in with as them. Issued with a name, a lifetime and
//! whether it may send; its secret shown once, with the line that adds it
//! to Claude Code; revoked after asking, inside the dialog.

use crate::i18n;
use crate::states::{AppStore, DirectoryDone, DirectoryEvent, DirectoryState};
use crate::this_build;
use crate::views::inline_confirm::inline_confirm;
use cyberapi_core::McpTokenId;
use cyberapi_core::Timestamp;
use cyberapi_core::directory::{MCP_TOKEN_DAYS, McpToken, NewMcpToken, valid_token_name};
use cyberapi_core::format::days_until;
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::dialog::{DialogClose, DialogFooter};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{
    ActiveTheme as _, IconName, Selectable as _, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClipboardItem, Context, Entity, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, Window, div, px,
};
use rust_i18n::t;

pub fn open_mcp_dialog(window: &mut Window, cx: &mut App) {
    let Some(directory) = AppStore::global(cx).directory(cx) else {
        return;
    };
    let state = cx.new(|_| DirectoryState::new(directory));
    state.update(cx, |state, cx| state.reload_mcp_tokens(cx));
    let view = cx.new(|cx| McpTokens::new(state, window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title(t!("mcp.title").to_string())
            .w(px(600.))
            .child(crate::views::dialog_body(view.clone()))
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|button| button.label(t!("common.close").to_string()))),
            )
            // No button confirms: Enter closes nothing.
            .on_ok(|_, _, _| false)
    });
}

struct McpTokens {
    state: Entity<DirectoryState>,
    name: Entity<InputState>,
    /// The lifetime picked: one of `MCP_TOKEN_DAYS`, or `None` for none.
    days: Option<u32>,
    can_send: bool,
    /// H1.
    can_write: bool,
    /// The token being asked about before it is revoked.
    revoking: Option<McpTokenId>,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

/// The lifetimes offered, in order: a month, the default three, a year,
/// none.
fn lifetimes() -> [Option<u32>; 4] {
    [
        Some(MCP_TOKEN_DAYS[1]),
        Some(MCP_TOKEN_DAYS[0]),
        Some(MCP_TOKEN_DAYS[2]),
        None,
    ]
}

fn lifetime_label(days: Option<u32>) -> String {
    match days {
        Some(365) => t!("mcp.year").to_string(),
        Some(days) => t!("mcp.days", count = days).to_string(),
        None => t!("mcp.never").to_string(),
    }
}

/// Puts `text` on the clipboard, when the reader asks, and says so.
fn copy(text: &str, window: &mut Window, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
    window.push_notification(Notification::success(t!("common.copied").to_string()), cx);
}

impl McpTokens {
    fn new(state: Entity<DirectoryState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(t!("mcp.name_placeholder").to_string()));
        let subscriptions = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.subscribe_in(&state, window, |this, _, event, window, cx| match event {
                DirectoryEvent::Done(DirectoryDone::McpTokenIssued) => {
                    this.error = None;
                    this.name.update(cx, |name, cx| name.set_value("", window, cx));
                    cx.notify();
                }
                DirectoryEvent::Done(_) => {}
                DirectoryEvent::Failed(error) => {
                    this.error = Some(i18n::store_error(error));
                    cx.notify();
                }
            }),
        ];
        Self {
            state,
            name,
            days: Some(MCP_TOKEN_DAYS[0]),
            can_send: false,
            can_write: false,
            revoking: None,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    fn issue(&mut self, cx: &mut Context<Self>) {
        let name = self.name.read(cx).value().trim().to_string();
        if !valid_token_name(&name) {
            self.error = Some(t!("errors.refused_bad_token_name").to_string());
            cx.notify();
            return;
        }
        let input = NewMcpToken {
            name,
            can_send: self.can_send,
            can_write: self.can_write,
            days: self.days,
        };
        self.state.update(cx, |state, cx| state.issue_mcp_token(input, cx));
    }

    fn render_form(&self, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let chosen = self.days;
        let group = lifetimes().into_iter().enumerate().fold(
            ButtonGroup::new("mcp-days").compact().small().outline(),
            |group, (index, days)| {
                group.child(
                    Button::new(("mcp-days", index))
                        .label(lifetime_label(days))
                        .selected(days == chosen),
                )
            },
        );
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().flex_1().child(Input::new(&self.name).small()))
                    .child(
                        Button::new("mcp-issue")
                            .primary()
                            .small()
                            .label(t!("mcp.issue").to_string())
                            .on_click(cx.listener(|this, _, _, cx| this.issue(cx))),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .text_sm()
                    .child(div().text_color(muted).child(t!("mcp.lifetime").to_string()))
                    .child(group.on_click(cx.listener(|this, picks: &Vec<usize>, _, cx| {
                        if let Some(days) = picks.first().and_then(|index| lifetimes().get(*index).copied()) {
                            this.days = days;
                            cx.notify();
                        }
                    }))),
            )
            .child(
                h_flex()
                    .gap_3()
                    .items_start()
                    .child(Switch::new("mcp-can-send").checked(self.can_send).on_click(cx.listener(
                        |this, checked: &bool, _, cx| {
                            this.can_send = *checked;
                            cx.notify();
                        },
                    )))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(div().text_sm().child(t!("mcp.can_send").to_string()))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(t!("mcp.can_send_desc").to_string()),
                            ),
                    ),
            )
            // H1.
            .child(
                h_flex()
                    .gap_3()
                    .items_start()
                    .child(
                        Switch::new("mcp-can-write")
                            .checked(self.can_write)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.can_write = *checked;
                                cx.notify();
                            })),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(div().text_sm().child(t!("mcp.can_write").to_string()))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(t!("mcp.can_write_desc").to_string()),
                            ),
                    ),
            )
            .when_some(self.error.clone(), |this, error| {
                this.child(div().text_sm().text_color(cx.theme().danger).child(error))
            })
            .into_any_element()
    }

    /// The secret just issued, once, and the line that adds it to Claude
    /// Code.
    fn render_issued(&self, secret: &str, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let server = AppStore::global(cx).server_address(cx);
        let endpoint = this_build::mcp_endpoint(server.as_deref());
        let command =
            format!("claude mcp add --transport http cyberapi {endpoint} --header \"Authorization: Bearer {secret}\"");
        let line = |id: &'static str, text: String| {
            let copied = text.clone();
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_xs()
                        .font_family(theme.mono_font_family.clone())
                        .child(text),
                )
                .child(
                    Button::new(id)
                        .outline()
                        .xsmall()
                        .icon(IconName::Copy)
                        .label(t!("common.copy").to_string())
                        .on_click(move |_, window, cx| copy(&copied, window, cx)),
                )
        };
        v_flex()
            .gap_2()
            .p_3()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.warning)
            .child(div().text_sm().font_semibold().child(t!("mcp.issued").to_string()))
            .child(line("mcp-copy-token", secret.to_string()))
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(t!("mcp.command").to_string()),
            )
            .child(line("mcp-copy-command", command))
            .into_any_element()
    }

    fn render_token(&self, index: usize, token: &McpToken, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let now = Timestamp::new(chrono::Local::now().fixed_offset());
        let lasts = match token.expires_at {
            Some(at) => i18n::counted("mcp.expires_in", days_until(at, now) as usize),
            None => t!("mcp.never").to_string(),
        };
        let id = token.id.clone();
        h_flex()
            .gap_2()
            .py_1()
            .items_center()
            .text_sm()
            .child(div().flex_1().min_w_0().truncate().child(token.name.clone()))
            .when(token.can_send, |this| {
                this.child(Tag::warning().small().child(t!("mcp.sends").to_string()))
            })
            .when(token.can_write, |this| {
                this.child(Tag::warning().small().child(t!("mcp.writes").to_string()))
            })
            .child(div().text_xs().text_color(muted).child(lasts))
            .child(
                Button::new(("mcp-revoke", index))
                    .ghost()
                    .xsmall()
                    .label(t!("mcp.revoke").to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.revoking = Some(id.clone());
                        cx.notify();
                    })),
            )
            .into_any_element()
    }
}

impl Render for McpTokens {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border) = (theme.muted_foreground, theme.border);
        let tokens: Vec<McpToken> = self.state.read(cx).mcp_tokens().to_vec();
        let issued = self.state.read(cx).issued().map(|(_, secret)| secret.clone());
        let asking = self
            .revoking
            .as_ref()
            .and_then(|id| tokens.iter().find(|token| &token.id == id).cloned());
        let rows: Vec<AnyElement> = tokens
            .iter()
            .enumerate()
            .map(|(index, token)| self.render_token(index, token, cx))
            .collect();
        let form = self.render_form(cx);
        let issued = issued.map(|secret| self.render_issued(&secret, cx));
        let confirm = asking.map(|token| {
            let (state, id) = (self.state.clone(), token.id.clone());
            let this = cx.entity().downgrade();
            let cancel = this.clone();
            inline_confirm(
                t!("mcp.revoke_title", name = token.name).to_string(),
                t!("mcp.revoke_description").to_string(),
                None,
                t!("mcp.revoke").to_string(),
                move |_, cx| {
                    let _ = cancel.update(cx, |this, cx| {
                        this.revoking = None;
                        cx.notify();
                    });
                },
                move |_, cx| {
                    let id = id.clone();
                    state.update(cx, |state, cx| state.revoke_mcp_token(id, cx));
                    let _ = this.update(cx, |this, cx| {
                        this.revoking = None;
                        cx.notify();
                    });
                },
                cx,
            )
        });
        v_flex()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .text_color(muted)
                    .child(SharedString::from(t!("mcp.intro").to_string())),
            )
            .child(form)
            .children(issued)
            .child(
                v_flex()
                    .gap_1()
                    .pt_2()
                    .border_t_1()
                    .border_color(border)
                    .child(div().text_xs().text_color(muted).child(t!("mcp.tokens").to_string()))
                    .when(rows.is_empty(), |this| {
                        this.child(div().text_sm().text_color(muted).child(t!("mcp.none").to_string()))
                    })
                    .children(rows),
            )
            .children(confirm)
    }
}
