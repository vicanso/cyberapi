//! The desktop's bridge in the settings (`docs/server-sync.md` §1, step 5):
//! its address and a sign-in, or who is signed in there and a way out, as
//! rows of the settings. The page is signed in by its own and has none of
//! this.

use crate::states::{ServerEvent, ServerState, ServerStatus, SignInFailure};
use crate::views::settings::setting_row;
use cyberapi_store_bridge::SignInError;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _, Subscription,
    Window, div,
};
use rust_i18n::t;

pub struct ServerSection {
    state: Entity<ServerState>,
    address: Entity<InputState>,
    name: Entity<InputState>,
    password: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl ServerSection {
    pub fn new(state: Entity<ServerState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let address = state.read(cx).address().to_string();
        let address = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://cyberapi.example.com")
                .default_value(address)
        });
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(t!("server.name").to_string()));
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(t!("server.password").to_string())
        });
        let subscriptions = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.subscribe_in(&state, window, |this, _, event, window, cx| match event {
                ServerEvent::SignedIn => {
                    this.password.update(cx, |input, cx| input.set_value("", window, cx));
                    window.push_notification(Notification::success(t!("server.signed_in_done").to_string()), cx);
                }
                ServerEvent::SignInFailed(failure) => {
                    window.push_notification(Notification::error(failure_text(failure)), cx);
                }
            }),
            cx.subscribe_in(&password, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.sign_in(cx);
                }
            }),
        ];
        Self {
            state,
            address,
            name,
            password,
            _subscriptions: subscriptions,
        }
    }

    fn sign_in(&mut self, cx: &mut Context<Self>) {
        let read = |input: &Entity<InputState>, cx: &Context<Self>| input.read(cx).value().to_string();
        let (address, name, password) = (read(&self.address, cx), read(&self.name, cx), read(&self.password, cx));
        self.state
            .update(cx, |state, cx| state.sign_in(&address, name, password, cx));
    }

    fn render_form(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let signing_in = self.state.read(cx).signing_in();
        vec![
            setting_row(
                t!("server.address").to_string(),
                Some(t!("server.address_desc").to_string()),
                Input::new(&self.address).small(),
                cx,
            ),
            setting_row(t!("server.name").to_string(), None, Input::new(&self.name).small(), cx),
            setting_row(
                t!("server.password").to_string(),
                None,
                Input::new(&self.password).small(),
                cx,
            ),
            h_flex()
                .justify_end()
                .py_2()
                .child(
                    Button::new("server-sign-in")
                        .primary()
                        .small()
                        .label(t!("server.sign_in").to_string())
                        .loading(signing_in)
                        .on_click(cx.listener(|this, _, _, cx| this.sign_in(cx))),
                )
                .into_any_element(),
        ]
    }

    fn sign_out_button(&self) -> Button {
        let state = self.state.clone();
        Button::new("server-sign-out")
            .outline()
            .small()
            .label(t!("server.sign_out").to_string())
            .on_click(move |_, _, cx| state.update(cx, |state, cx| state.sign_out(cx)))
    }
}

fn failure_text(failure: &SignInFailure) -> String {
    match failure {
        SignInFailure::BadAddress => t!("server.bad_address").to_string(),
        SignInFailure::Login(SignInError::WrongLogin) => t!("server.wrong_login").to_string(),
        SignInFailure::Login(SignInError::TooManyTries(_)) => t!("server.too_many").to_string(),
        SignInFailure::Login(SignInError::Failed(reason)) => t!("server.failed", reason = reason).to_string(),
    }
}

impl Render for ServerSection {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx);
        let address = state.address().to_string();
        let rows = match state.status().clone() {
            ServerStatus::SignedOut => self.render_form(cx),
            ServerStatus::Checking => vec![setting_row(
                t!("server.status").to_string(),
                Some(t!("server.checking", address = address).to_string()),
                div(),
                cx,
            )],
            ServerStatus::SignedIn(me) => {
                let name = me.account.label();
                // The login beside the name only where it is another word.
                let account = if name == me.account.name {
                    t!("server.signed_in", address = address, name = name)
                } else {
                    t!(
                        "server.signed_in_named",
                        address = address,
                        name = name,
                        login = me.account.name
                    )
                };
                vec![setting_row(
                    t!("server.account").to_string(),
                    Some(account.to_string()),
                    self.sign_out_button(),
                    cx,
                )]
            }
            ServerStatus::Unreachable(reason) => {
                let state = self.state.clone();
                vec![setting_row(
                    t!("server.status").to_string(),
                    Some(t!("server.unreachable", address = address, reason = reason).to_string()),
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("server-retry")
                                .outline()
                                .small()
                                .label(t!("common.retry").to_string())
                                .on_click(move |_, _, cx| state.update(cx, |state, cx| state.check(cx))),
                        )
                        .child(self.sign_out_button()),
                    cx,
                )]
            }
        };
        v_flex().children(rows)
    }
}
