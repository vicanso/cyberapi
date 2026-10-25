//! Every account, for a system administrator (browser build, §9.1): invite
//! someone to make one (`docs/server-sync.md` §5), make an account an
//! administrator or not, disable it, reset its password.

use crate::i18n;
use crate::states::{AppStore, DirectoryDone, DirectoryEvent, DirectoryState};
use crate::views::inline_confirm::inline_confirm;
use crate::views::invites::{copy_link, invites_section};
use cyberapi_core::directory::{AccountInfo, valid_password};
use cyberapi_store::AccountEdit;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, IconName, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, Styled as _, Subscription, Window, div, px,
};
use rust_i18n::t;

pub fn open_accounts_dialog(window: &mut Window, cx: &mut App) {
    let Some(directory) = AppStore::global(cx).directory(cx) else {
        return;
    };
    let state = cx.new(|_| DirectoryState::new(directory));
    let editor = cx.new(|cx| AccountsEditor::new(state, window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title(t!("accounts.title").to_string())
            .w(px(860.))
            .child(crate::views::dialog_body(editor.clone()))
            // Enter is a field's own, not the dialog's to close on.
            .on_ok(|_, _, _| false)
    });
}

struct AccountsEditor {
    state: Entity<DirectoryState>,
    /// The account whose password is being reset, and the new one.
    resetting: Option<(AccountInfo, Entity<InputState>)>,
    _subscriptions: Vec<Subscription>,
}

impl AccountsEditor {
    fn new(state: Entity<DirectoryState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        state.update(cx, |state, cx| {
            state.reload_me(cx);
            state.reload_accounts(cx);
            state.load_invites(cx);
        });
        let subscriptions = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.subscribe_in(&state, window, |this, _, event, window, cx| match event {
                DirectoryEvent::Done(DirectoryDone::InviteCreated(link)) => copy_link(link, window, cx),
                DirectoryEvent::Done(DirectoryDone::PasswordReset) => {
                    this.resetting = None;
                    window.push_notification(Notification::success(t!("password.reset_done").to_string()), cx);
                    cx.notify();
                }
                DirectoryEvent::Done(_) => {}
                DirectoryEvent::Failed(error) => {
                    window.push_notification(Notification::error(i18n::store_error(error)), cx);
                }
            }),
        ];
        Self {
            state,
            resetting: None,
            _subscriptions: subscriptions,
        }
    }

    fn start_reset(&mut self, account: AccountInfo, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(t!("password.new").to_string())
        });
        self.resetting = Some((account, input));
        cx.notify();
    }

    fn render_resetting(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (account, input) = self.resetting.clone()?;
        let (cancelled, confirmed) = (cx.entity().downgrade(), cx.entity().downgrade());
        Some(
            inline_confirm(
                t!("password.reset_title", name = account.label()).to_string(),
                t!("password.reset_hint").to_string(),
                Some(Input::new(&input).small().into_any_element()),
                t!("password.reset").to_string(),
                move |_, cx| {
                    let _ = cancelled.update(cx, |this, cx| {
                        this.resetting = None;
                        cx.notify();
                    });
                },
                move |window, cx| {
                    let password = input.read(cx).value().to_string();
                    if !valid_password(&password) {
                        let warning = t!("errors.refused_password_too_short").to_string();
                        window.push_notification(Notification::warning(warning), cx);
                        return;
                    }
                    let id = account.id.clone();
                    let _ = confirmed.update(cx, |this, cx| {
                        this.state
                            .update(cx, |state, cx| state.reset_password(id, password, cx));
                    });
                },
                cx,
            )
            .into_any_element(),
        )
    }

    fn edit(&mut self, account: &AccountInfo, admin: bool, disabled: bool, cx: &mut Context<Self>) {
        let edit = AccountEdit {
            display_name: account.display_name.clone(),
            admin,
            disabled,
        };
        let id = account.id.clone();
        self.state.update(cx, |state, cx| state.update_account(id, edit, cx));
    }
}

impl Render for AccountsEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border) = (theme.muted_foreground, theme.border);
        let state = self.state.read(cx);
        let me = state.me().map(|me| me.account.id.clone());
        let accounts = state.accounts().to_vec();
        let header = h_flex()
            .gap_2()
            .text_xs()
            .text_color(muted)
            .child(div().w(px(180.)).child(t!("accounts.name").to_string()))
            .child(div().flex_1().child(t!("accounts.display_name").to_string()))
            .child(div().w(px(110.)).child(t!("accounts.admin").to_string()))
            .child(div().w(px(80.)).child(t!("accounts.disabled").to_string()))
            .child(div().w(px(130.)));
        let rows = accounts.into_iter().enumerate().map(|(index, account)| {
            let yourself = me.as_ref() == Some(&account.id);
            let (for_admin, for_disabled, for_reset) = (account.clone(), account.clone(), account.clone());
            h_flex()
                .id(("account", index))
                .gap_2()
                .py_1()
                .items_center()
                .text_sm()
                .child(div().w(px(180.)).truncate().child(account.name.clone()))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(muted)
                        .child(account.display_name.clone()),
                )
                .child(
                    div().w(px(110.)).child(
                        Checkbox::new(("account-admin", index))
                            .checked(account.admin)
                            .disabled(yourself)
                            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                let account = for_admin.clone();
                                this.edit(&account, *checked, account.disabled, cx);
                            })),
                    ),
                )
                .child(
                    div().w(px(80.)).child(
                        Checkbox::new(("account-disabled", index))
                            .checked(account.disabled)
                            .disabled(yourself)
                            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                let account = for_disabled.clone();
                                this.edit(&account, account.admin, *checked, cx);
                            })),
                    ),
                )
                .child(
                    div().w(px(130.)).child(
                        Button::new(("account-reset", index))
                            .ghost()
                            .xsmall()
                            .label(t!("accounts.reset_password").to_string())
                            .on_click(
                                cx.listener(move |this, _, window, cx| this.start_reset(for_reset.clone(), window, cx)),
                            ),
                    ),
                )
        });
        let invites = invites_section(&self.state, None, cx);
        v_flex()
            .gap_2()
            .child(div().text_sm().text_color(muted).child(t!("accounts.hint").to_string()))
            .child(header)
            .child(
                div()
                    .id("accounts")
                    .max_h(px(360.))
                    .overflow_y_scrollbar()
                    .children(rows),
            )
            .children(self.render_resetting(cx))
            .when_some(invites, |this, invites| this.child(invites))
            .child(
                h_flex()
                    .gap_2()
                    .pt_2()
                    .items_center()
                    .border_t_1()
                    .border_color(border)
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(muted)
                            .child(t!("accounts.invite_hint").to_string()),
                    )
                    .child(
                        Button::new("account-invite")
                            .primary()
                            .small()
                            .icon(IconName::Plus)
                            .label(t!("accounts.invite").to_string())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.state.update(cx, |state, cx| state.create_invite(None, cx));
                            })),
                    ),
            )
    }
}
