//! The reader's own password, in the browser build (§9.4): changed with the
//! one they have now. An administrator resets anyone else's in the accounts
//! dialog.

use crate::i18n;
use crate::states::{AppStore, DirectoryDone, DirectoryEvent, DirectoryState};
use crate::views::name_dialog::ok_button;
use cyberapi_core::directory::valid_password;
use gpui_kit::component::dialog::{DialogClose, DialogFooter};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, StyledExt as _, WindowExt as _, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    Subscription, Window, div, px,
};
use rust_i18n::t;

pub fn open_password_dialog(window: &mut Window, cx: &mut App) {
    let Some(directory) = AppStore::global(cx).directory().cloned() else {
        return;
    };
    let state = cx.new(|_| DirectoryState::new(directory));
    let form = cx.new(|cx| PasswordForm::new(state, window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        // Kept open until the store answers; the form closes it then.
        let submit = {
            let form = form.clone();
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                form.update(cx, |form, cx| form.submit(window, cx));
                false
            }
        };
        dialog
            .title(t!("password.title").to_string())
            .w(px(420.))
            .child(form.clone())
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|button| button.label(t!("common.cancel").to_string())))
                    .child(ok_button("password-ok", t!("common.save").to_string(), submit.clone())),
            )
            .on_ok(submit)
    });
}

struct PasswordForm {
    state: Entity<DirectoryState>,
    current: Entity<InputState>,
    password: Entity<InputState>,
    again: Entity<InputState>,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl PasswordForm {
    fn new(state: Entity<DirectoryState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let masked =
            |window: &mut Window, cx: &mut Context<Self>| cx.new(|cx| InputState::new(window, cx).masked(true));
        let (current, password, again) = (masked(window, cx), masked(window, cx), masked(window, cx));
        let subscriptions = vec![
            cx.subscribe_in(&state, window, |this, _, event, window, cx| match event {
                DirectoryEvent::Done(DirectoryDone::PasswordChanged) => {
                    window.close_dialog(cx);
                    window.push_notification(Notification::success(t!("password.changed").to_string()), cx);
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
            current,
            password,
            again,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    fn submit(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let read = |input: &Entity<InputState>, cx: &App| input.read(cx).value().to_string();
        let (current, password, again) = (read(&self.current, cx), read(&self.password, cx), read(&self.again, cx));
        self.error = if !valid_password(&password) {
            Some(t!("errors.refused_password_too_short").to_string())
        } else if password != again {
            Some(t!("password.mismatch").to_string())
        } else {
            None
        };
        cx.notify();
        if self.error.is_none() {
            self.state
                .update(cx, |state, cx| state.change_password(current, password, cx));
        }
    }
}

fn field(label: String, input: &Entity<InputState>) -> impl IntoElement {
    v_flex()
        .gap_1p5()
        .child(div().text_sm().font_medium().child(label))
        .child(Input::new(input))
}

impl Render for PasswordForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_3()
            .child(field(t!("password.current").to_string(), &self.current))
            .child(field(t!("password.new").to_string(), &self.password))
            .child(field(t!("password.again").to_string(), &self.again))
            .when_some(self.error.clone(), |this, error| {
                this.child(div().text_sm().text_color(cx.theme().danger).child(error))
            })
    }
}
