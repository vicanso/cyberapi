//! System administrators: the LDAP directory a sign-in falls back to when
//! the local password does not hold. Kept by the bridge; the bind password
//! goes in and never comes back. An administrator always signs in with
//! their own password, so nothing here can lock one out.

use crate::states::AppStore;
use crate::views::name_dialog::ok_button;
use cyberapi_core::ldap::{DEFAULT_DISPLAY_ATTRIBUTE, DEFAULT_FILTER, LdapChange, LdapProblem, LdapSettings};
use cyberapi_store_bridge::LdapRefusal;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::{DialogClose, DialogFooter};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, Styled as _, Window, div, px,
};
use rust_i18n::t;

pub fn open_ldap_dialog(window: &mut Window, cx: &mut App) {
    let form = cx.new(|cx| LdapForm::new(window, cx));
    window.open_dialog(cx, move |dialog, window, _| {
        let submit = {
            let form = form.clone();
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                form.update(cx, |form, cx| form.save(window, cx));
                false
            }
        };
        dialog
            .title(t!("ldap.title").to_string())
            .w(px(520.))
            .child(
                // Seven fields: on a short window they scroll inside.
                div()
                    .id("ldap-fields")
                    .max_h(window.viewport_size().height - px(220.))
                    .overflow_y_scrollbar()
                    .pr_3()
                    .child(crate::views::dialog_body(form.clone())),
            )
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|button| button.label(t!("common.cancel").to_string())))
                    .child(ok_button("ldap-ok", t!("common.save").to_string(), submit.clone())),
            )
            .on_ok(submit)
    });
}

/// What the last test said.
enum Tested {
    Working,
    Passed,
    Failed(String),
}

struct LdapForm {
    url: Entity<InputState>,
    start_tls: bool,
    bind_dn: Entity<InputState>,
    bind_password: Entity<InputState>,
    base_dn: Entity<InputState>,
    user_filter: Entity<InputState>,
    display_attribute: Entity<InputState>,
    /// What the bridge said is in use; `None` until it answered.
    now: Option<Option<LdapSettings>>,
    tested: Option<Tested>,
    error: Option<String>,
    saving: bool,
}

fn said(refusal: LdapRefusal) -> String {
    match refusal {
        LdapRefusal::Problem(problem) => match problem {
            LdapProblem::Url => t!("ldap.bad_url"),
            LdapProblem::StartTlsOnLdaps => t!("ldap.bad_start_tls"),
            LdapProblem::BaseDn => t!("ldap.bad_base_dn"),
            LdapProblem::Filter => t!("ldap.bad_filter"),
            LdapProblem::BindPassword => t!("ldap.bad_bind_password"),
        }
        .to_string(),
        LdapRefusal::Failed(why) => why,
    }
}

impl LdapForm {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = |placeholder: &str, masked: bool, window: &mut Window, cx: &mut Context<Self>| {
            let placeholder = placeholder.to_string();
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).masked(masked))
        };
        let url = input("ldaps://ldap.example.com:636", false, window, cx);
        let bind_dn = input("cn=reader,dc=example,dc=com", false, window, cx);
        let bind_password = input("", true, window, cx);
        let base_dn = input("ou=people,dc=example,dc=com", false, window, cx);
        let user_filter = input(DEFAULT_FILTER, false, window, cx);
        let display_attribute = input(DEFAULT_DISPLAY_ATTRIBUTE, false, window, cx);
        let state = AppStore::global(cx).state().clone();
        let asked = state.update(cx, |state, cx| state.ldap_settings(cx));
        cx.spawn_in(window, async move |this, cx| {
            let answer = asked.await;
            let _ = this.update_in(cx, |this, window, cx| {
                match answer {
                    Ok(now) => this.show(now, window, cx),
                    Err(refusal) => this.error = Some(said(refusal)),
                }
                cx.notify();
            });
        })
        .detach();
        Self {
            url,
            start_tls: false,
            bind_dn,
            bind_password,
            base_dn,
            user_filter,
            display_attribute,
            now: None,
            tested: None,
            error: None,
            saving: false,
        }
    }

    fn show(&mut self, now: Option<LdapSettings>, window: &mut Window, cx: &mut Context<Self>) {
        let settings = now.clone().unwrap_or_default();
        let set = |input: &Entity<InputState>, value: &str, window: &mut Window, cx: &mut Context<Self>| {
            let value = value.to_string();
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        };
        set(&self.url, &settings.url, window, cx);
        set(&self.bind_dn, &settings.bind_dn, window, cx);
        set(&self.bind_password, "", window, cx);
        set(&self.base_dn, &settings.base_dn, window, cx);
        set(&self.user_filter, &settings.user_filter, window, cx);
        set(&self.display_attribute, &settings.display_attribute, window, cx);
        self.start_tls = settings.start_tls;
        let kept = if settings.has_bind_password {
            t!("ldap.password_kept").to_string()
        } else {
            String::new()
        };
        self.bind_password
            .update(cx, |input, cx| input.set_placeholder(kept, window, cx));
        self.now = Some(now);
    }

    fn change(&self, cx: &App) -> LdapChange {
        let read = |input: &Entity<InputState>| input.read(cx).value().to_string();
        LdapChange {
            url: read(&self.url),
            start_tls: self.start_tls,
            bind_dn: read(&self.bind_dn),
            bind_password: read(&self.bind_password),
            base_dn: read(&self.base_dn),
            user_filter: read(&self.user_filter),
            display_attribute: read(&self.display_attribute),
        }
    }

    fn test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.tested, Some(Tested::Working)) {
            return;
        }
        let change = self.change(cx);
        self.tested = Some(Tested::Working);
        cx.notify();
        let state = AppStore::global(cx).state().clone();
        let tested = state.update(cx, |state, cx| state.test_ldap(change, cx));
        cx.spawn_in(window, async move |this, cx| {
            let answer = tested.await;
            let _ = this.update(cx, |this, cx| {
                this.tested = Some(match answer {
                    Ok(()) => Tested::Passed,
                    Err(refusal) => Tested::Failed(said(refusal)),
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.send(Some(self.change(cx)), window, cx);
    }

    /// Saves `change`, or with `None` turns LDAP sign-in off.
    fn send(&mut self, change: Option<LdapChange>, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving || self.now.is_none() {
            return;
        }
        let turning_off = change.is_none();
        self.saving = true;
        self.error = None;
        cx.notify();
        let state = AppStore::global(cx).state().clone();
        let saved = state.update(cx, |state, cx| state.save_ldap(change, cx));
        cx.spawn_in(window, async move |this, cx| {
            let answer = saved.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.saving = false;
                match answer {
                    Ok(now) if turning_off => this.show(now, window, cx),
                    Ok(_) => {
                        window.close_dialog(cx);
                        window.push_notification(Notification::success(t!("ldap.saved").to_string()), cx);
                    }
                    Err(refusal) => this.error = Some(said(refusal)),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

fn field(label: String, input: &Entity<InputState>) -> impl IntoElement {
    v_flex()
        .gap_1p5()
        .child(div().text_sm().font_medium().child(label))
        .child(Input::new(input))
}

impl Render for LdapForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, danger, success) = (theme.muted_foreground, theme.danger, theme.success);
        let status = match &self.now {
            None => t!("ldap.loading"),
            Some(None) => t!("ldap.off"),
            Some(Some(_)) => t!("ldap.on"),
        };
        let on = matches!(&self.now, Some(Some(_)));
        let tested = self.tested.as_ref().map(|tested| match tested {
            Tested::Working => div().text_color(muted).child(t!("ldap.testing").to_string()),
            Tested::Passed => div().text_color(success).child(t!("ldap.test_passed").to_string()),
            Tested::Failed(why) => div()
                .text_color(danger)
                .child(t!("ldap.test_failed", why = why).to_string()),
        });
        v_flex()
            .gap_3()
            .child(div().text_sm().text_color(muted).child(status.to_string()))
            .child(field(t!("ldap.url").to_string(), &self.url))
            .child(
                Checkbox::new("ldap-start-tls")
                    .checked(self.start_tls)
                    .label(t!("ldap.start_tls").to_string())
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.start_tls = *checked;
                        cx.notify();
                    })),
            )
            .child(field(t!("ldap.bind_dn").to_string(), &self.bind_dn))
            .child(field(t!("ldap.bind_password").to_string(), &self.bind_password))
            .child(field(t!("ldap.base_dn").to_string(), &self.base_dn))
            .child(field(t!("ldap.user_filter").to_string(), &self.user_filter))
            .child(field(t!("ldap.display_attribute").to_string(), &self.display_attribute))
            .child(div().text_xs().text_color(muted).child(t!("ldap.note").to_string()))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("ldap-test")
                            .small()
                            .label(t!("ldap.test").to_string())
                            .on_click(cx.listener(|this, _, window, cx| this.test(window, cx))),
                    )
                    .when(on, |this| {
                        this.child(
                            Button::new("ldap-off")
                                .ghost()
                                .small()
                                .label(t!("ldap.turn_off").to_string())
                                .on_click(cx.listener(|this, _, window, cx| this.send(None, window, cx))),
                        )
                    })
                    .children(tested.map(|tested| tested.text_sm())),
            )
            .when_some(self.error.clone(), |this, error| {
                this.child(div().text_sm().text_color(danger).child(error))
            })
    }
}
