//! H3, administrators: the assistant's model — protocol, address, name and
//! key — set on the page and kept by the bridge. The key goes in and never
//! comes back: the dialog only says whether one is set.

use crate::states::AppStore;
use crate::views::name_dialog::ok_button;
use cyberapi_core::ai::{ModelChange, ModelSettings, Protocol};
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::dialog::{DialogClose, DialogFooter};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, Selectable as _, Sizable as _, StyledExt as _, WindowExt as _, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _, Window,
    div, px,
};
use rust_i18n::t;

const PROTOCOLS: [Protocol; 2] = [Protocol::OpenAi, Protocol::Anthropic];

pub fn open_ai_settings_dialog(window: &mut Window, cx: &mut App) {
    let form = cx.new(|cx| AiSettingsForm::new(window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        let submit = {
            let form = form.clone();
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                form.update(cx, |form, cx| form.save(Some(()), window, cx));
                false
            }
        };
        dialog
            .title(t!("ai_settings.title").to_string())
            .w(px(480.))
            .child(crate::views::dialog_body(form.clone()))
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|button| button.label(t!("common.cancel").to_string())))
                    .child(ok_button(
                        "ai-settings-ok",
                        t!("common.save").to_string(),
                        submit.clone(),
                    )),
            )
            .on_ok(submit)
    });
}

struct AiSettingsForm {
    protocol: Protocol,
    url: Entity<InputState>,
    model: Entity<InputState>,
    key: Entity<InputState>,
    /// What the bridge said is in use; `None` until it answered.
    now: Option<Option<ModelSettings>>,
    error: Option<String>,
    saving: bool,
}

impl AiSettingsForm {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let url = cx.new(|cx| InputState::new(window, cx).placeholder("https://api.openai.com/v1"));
        let model = cx.new(|cx| InputState::new(window, cx).placeholder("gpt-4o-mini"));
        let key = cx.new(|cx| InputState::new(window, cx).masked(true));
        let state = AppStore::global(cx).state().clone();
        let asked = state.update(cx, |state, cx| state.ai_settings(cx));
        cx.spawn_in(window, async move |this, cx| {
            let answer = asked.await;
            let _ = this.update_in(cx, |this, window, cx| {
                match answer {
                    Ok(now) => this.show(now, window, cx),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        Self {
            protocol: Protocol::OpenAi,
            url,
            model,
            key,
            now: None,
            error: None,
            saving: false,
        }
    }

    fn show(&mut self, now: Option<ModelSettings>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(settings) = &now {
            self.protocol = settings.protocol;
            self.url
                .update(cx, |input, cx| input.set_value(settings.url.clone(), window, cx));
            self.model
                .update(cx, |input, cx| input.set_value(settings.model.clone(), window, cx));
        }
        let placeholder = if now
            .as_ref()
            .is_some_and(|settings| settings.has_key && settings.on_page)
        {
            t!("ai_settings.key_kept").to_string()
        } else {
            String::new()
        };
        self.key.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.set_placeholder(placeholder, window, cx);
        });
        self.now = Some(now);
    }

    /// Saves the form (`Some`), or removes the page's model (`None`).
    fn save(&mut self, form: Option<()>, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving || self.now.is_none() {
            return;
        }
        let read = |input: &Entity<InputState>, cx: &App| input.read(cx).value().to_string();
        let change = form.map(|()| ModelChange {
            protocol: self.protocol,
            url: read(&self.url, cx),
            model: read(&self.model, cx),
            key: read(&self.key, cx),
        });
        // The bridge checks too; here it is said in the reader's language.
        if let Some(change) = &change {
            let key_kept = matches!(&self.now, Some(Some(now)) if now.on_page && now.has_key);
            let url = change.url.trim();
            self.error = if !(url.starts_with("http://") || url.starts_with("https://")) {
                Some(t!("ai_settings.bad_url").to_string())
            } else if change.model.trim().is_empty() {
                Some(t!("ai_settings.no_model").to_string())
            } else if change.key.trim().is_empty() && !key_kept {
                Some(t!("ai_settings.no_key").to_string())
            } else {
                None
            };
            if self.error.is_some() {
                cx.notify();
                return;
            }
        }
        self.saving = true;
        self.error = None;
        cx.notify();
        let state = AppStore::global(cx).state().clone();
        let saved = state.update(cx, |state, cx| state.save_ai_settings(change, cx));
        let removing = form.is_none();
        cx.spawn_in(window, async move |this, cx| {
            let answer = saved.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.saving = false;
                match answer {
                    Ok(now) if removing => this.show(now, window, cx),
                    Ok(_) => {
                        window.close_dialog(cx);
                        window.push_notification(Notification::success(t!("ai_settings.saved").to_string()), cx);
                    }
                    Err(error) => this.error = Some(error),
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

impl Render for AiSettingsForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, danger) = (theme.muted_foreground, theme.danger);
        let chosen = self.protocol;
        let protocols = PROTOCOLS.iter().enumerate().fold(
            ButtonGroup::new("ai-protocol").compact().small().outline(),
            |group, (index, protocol)| {
                group.child(
                    Button::new(("ai-protocol", index))
                        .label(match protocol {
                            Protocol::OpenAi => "OpenAI",
                            Protocol::Anthropic => "Anthropic",
                        })
                        .selected(*protocol == chosen),
                )
            },
        );
        let status = match &self.now {
            None => t!("ai_settings.loading").to_string(),
            Some(None) => t!("ai_settings.none").to_string(),
            Some(Some(settings)) if settings.on_page => t!("ai_settings.on_page").to_string(),
            Some(Some(_)) => t!("ai_settings.from_options").to_string(),
        };
        let on_page = matches!(&self.now, Some(Some(settings)) if settings.on_page);
        v_flex()
            .gap_3()
            .child(div().text_sm().text_color(muted).child(status))
            .child(
                v_flex()
                    .gap_1p5()
                    .child(
                        div()
                            .text_sm()
                            .font_medium()
                            .child(t!("ai_settings.protocol").to_string()),
                    )
                    .child(protocols.on_click(cx.listener(|this, picks: &Vec<usize>, _, cx| {
                        if let Some(protocol) = picks.first().and_then(|index| PROTOCOLS.get(*index)) {
                            this.protocol = *protocol;
                            cx.notify();
                        }
                    }))),
            )
            .child(field(t!("ai_settings.url").to_string(), &self.url))
            .child(field(t!("ai_settings.model").to_string(), &self.model))
            .child(field(t!("ai_settings.key").to_string(), &self.key))
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(t!("ai_settings.key_note").to_string()),
            )
            .when(on_page, |this| {
                this.child(
                    div().child(
                        Button::new("ai-settings-remove")
                            .ghost()
                            .small()
                            .label(t!("ai_settings.remove").to_string())
                            .on_click(cx.listener(|this, _, window, cx| this.save(None, window, cx))),
                    ),
                )
            })
            .when_some(self.error.clone(), |this, error| {
                this.child(div().text_sm().text_color(danger).child(error))
            })
    }
}
