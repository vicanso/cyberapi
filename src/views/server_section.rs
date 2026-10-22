//! The desktop's bridge in the settings (`docs/server-sync.md` §1, step 5):
//! its address and a sign-in, or who is signed in there and a way out. The
//! page is signed in by its own and has none of this.

use crate::states::{ServerEvent, ServerState, ServerStatus, SignInFailure};
use cyberapi_store_bridge::SignInError;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _, Subscription,
    Window, div, px,
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

    fn render_form(&self, cx: &mut Context<Self>) -> AnyElement {
        let signing_in = self.state.read(cx).signing_in();
        v_flex()
            .gap_2()
            .child(Input::new(&self.address).small())
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&self.name).small()))
                    .child(div().flex_1().child(Input::new(&self.password).small()))
                    .child(
                        Button::new("server-sign-in")
                            .primary()
                            .small()
                            .label(t!("server.sign_in").to_string())
                            .loading(signing_in)
                            .on_click(cx.listener(|this, _, _, cx| this.sign_in(cx))),
                    ),
            )
            .into_any_element()
    }

    fn sign_out_button(&self) -> Button {
        let state = self.state.clone();
        Button::new("server-sign-out")
            .outline()
            .xsmall()
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
        let muted = cx.theme().muted_foreground;
        let state = self.state.read(cx);
        let address = state.address().to_string();
        let line = |text: String| div().flex_1().min_w_0().text_sm().child(text);
        let body: AnyElement = match state.status().clone() {
            ServerStatus::SignedOut => self.render_form(cx),
            ServerStatus::Checking => line(t!("server.checking", address = address).to_string()).into_any_element(),
            ServerStatus::SignedIn(me) => h_flex()
                .gap_3()
                .items_center()
                .child(line(
                    t!(
                        "server.signed_in",
                        address = address,
                        name = me.account.label(),
                        login = me.account.name
                    )
                    .to_string(),
                ))
                .child(self.sign_out_button())
                .into_any_element(),
            ServerStatus::Unreachable(reason) => {
                let state = self.state.clone();
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(line(
                        t!("server.unreachable", address = address, reason = reason).to_string(),
                    ))
                    .child(
                        Button::new("server-retry")
                            .outline()
                            .xsmall()
                            .label(t!("common.retry").to_string())
                            .on_click(move |_, _, cx| state.update(cx, |state, cx| state.check(cx))),
                    )
                    .child(self.sign_out_button())
                    .into_any_element()
            }
        };
        v_flex().gap_1().child(body).child(
            div()
                .max_w(px(520.))
                .text_xs()
                .text_color(muted)
                .child(t!("server.hint").to_string()),
        )
    }
}
