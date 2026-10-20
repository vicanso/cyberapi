//! The selected API's request (M4): method, environment and URI, the send
//! button, and the Body / Query / Header / Auth tabs. One per selected API;
//! everything it edits goes to the API's `ApiState`.

use super::method_color;
use crate::states::{ApiEvent, ApiState, WorkspaceState};
use crate::this_build;
use crate::views::kv_editor::{KvEditor, KvEvent, KvOptions};
use cyberapi_core::model::{ContentType, HttpMethod, HttpRequest, split_env_prefix};
use cyberapi_core::prefs::RequestTab;
use cyberapi_core::send::{self, form_fields};
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::input::{Editor, EditorState, Input, InputEvent, InputState, TabSize};
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, AnyElement, AppContext as _, Context, Entity, IntoElement, ParentElement as _, PathPromptOptions, Render,
    Styled as _, Subscription, Task, Window, div, px,
};
use rust_i18n::t;
use std::time::Duration;

pub struct RequestEditor {
    workspace: Entity<WorkspaceState>,
    api: Entity<ApiState>,
    uri: Entity<InputState>,
    body: Entity<EditorState>,
    form: Entity<KvEditor>,
    query: Entity<KvEditor>,
    headers: Entity<KvEditor>,
    auth: Entity<KvEditor>,
    /// Redraws the elapsed time while a request is out.
    ticker: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

/// The body editor for a content type: JSON is highlighted where the
/// grammar is compiled in (the desktop).
fn body_state(request: &HttpRequest, window: &mut Window, cx: &mut Context<RequestEditor>) -> Entity<EditorState> {
    let language = match request.content_type {
        ContentType::Json => "json",
        _ => "text",
    };
    let text = if request.content_type.is_key_value() {
        String::new()
    } else {
        request.body.clone()
    };
    cx.new(|cx| {
        EditorState::new(window, cx)
            .language(language)
            .line_number(true)
            .soft_wrap(true)
            .tab_size(TabSize {
                tab_size: 2,
                hard_tabs: false,
            })
            .default_value(text)
    })
}

impl RequestEditor {
    pub fn new(
        workspace: Entity<WorkspaceState>,
        api: Entity<ApiState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let request = api.read(cx).request().clone();
        let uri = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("editor.uri_placeholder").to_string())
                .default_value(split_env_prefix(&request.uri).1.to_string())
        });
        let body = body_state(&request, window, cx);
        let kv = |list, options, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| KvEditor::new(list, options, window, cx))
        };
        let fields = form_fields(&request.body);
        let form = kv(
            &fields,
            KvOptions {
                files: request.content_type == ContentType::Multipart,
                ..Default::default()
            },
            window,
            cx,
        );
        let query = kv(&request.query, KvOptions::default(), window, cx);
        let headers = kv(&request.headers, KvOptions::default(), window, cx);
        let auth = kv(
            &request.auth,
            KvOptions {
                masked: true,
                ..Default::default()
            },
            window,
            cx,
        );
        let mut subscriptions = vec![
            cx.observe(&api, |this, _, cx| {
                this.follow_sending(cx);
                cx.notify();
            }),
            cx.observe(&workspace, |_, _, cx| cx.notify()),
            cx.subscribe_in(&api, window, |this, _, event, window, cx| match event {
                ApiEvent::RequestReplaced => this.reload(window, cx),
                ApiEvent::NotSent(message) => {
                    window.push_notification(Notification::error(message.clone()), cx);
                }
                ApiEvent::Curl(command) => {
                    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(command.clone()));
                    window.push_notification(Notification::success(t!("response.curl_copied").to_string()), cx);
                }
                ApiEvent::Failed(_) => {}
            }),
            // F4.3: the URI is saved as the field is left, or on Enter.
            cx.subscribe_in(&uri, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                    this.save_uri(cx);
                }
            }),
            cx.subscribe(&query, |this, _, KvEvent::Changed(list), cx| {
                this.api.update(cx, |api, cx| api.set_query(list.clone(), cx));
            }),
            cx.subscribe(&headers, |this, _, KvEvent::Changed(list), cx| {
                this.api.update(cx, |api, cx| api.set_headers(list.clone(), cx));
            }),
            cx.subscribe(&auth, |this, _, KvEvent::Changed(list), cx| {
                this.api.update(cx, |api, cx| api.set_auth(list.clone(), cx));
            }),
            cx.subscribe(&form, |this, _, KvEvent::Changed(list), cx| {
                this.api.update(cx, |api, cx| api.set_form(list.clone(), cx));
            }),
        ];
        subscriptions.push(Self::watch_body(&body, window, cx));
        let mut editor = Self {
            workspace,
            api,
            uri,
            body,
            form,
            query,
            headers,
            auth,
            ticker: None,
            _subscriptions: subscriptions,
        };
        editor.follow_sending(cx);
        editor
    }

    fn watch_body(body: &Entity<EditorState>, window: &mut Window, cx: &mut Context<Self>) -> Subscription {
        cx.subscribe_in(body, window, |this, body, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                let text = body.read(cx).value().to_string();
                this.api.update(cx, |api, cx| api.set_body(text, cx));
            }
        })
    }

    /// F7.10: the request came back from the history; every field reads it.
    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let request = self.api.read(cx).request().clone();
        let rest = split_env_prefix(&request.uri).1.to_string();
        self.uri.update(cx, |input, cx| input.set_value(rest, window, cx));
        self.body = body_state(&request, window, cx);
        self._subscriptions.push(Self::watch_body(&self.body, window, cx));
        self.form
            .update(cx, |form, cx| form.set_list(&form_fields(&request.body), window, cx));
        self.query.update(cx, |kv, cx| kv.set_list(&request.query, window, cx));
        self.headers
            .update(cx, |kv, cx| kv.set_list(&request.headers, window, cx));
        self.auth.update(cx, |kv, cx| kv.set_list(&request.auth, window, cx));
        cx.notify();
    }

    /// Hands the API what the fields hold now, ahead of their own delays:
    /// what was typed just before sending goes with it.
    fn flush(&mut self, cx: &mut Context<Self>) {
        self.save_uri(cx);
        let (query, headers, auth) = (
            self.query.read(cx).list(cx),
            self.headers.read(cx).list(cx),
            self.auth.read(cx).list(cx),
        );
        let content_type = self.api.read(cx).request().content_type;
        let body = (!content_type.is_key_value()).then(|| self.body.read(cx).value().to_string());
        let form = content_type.is_key_value().then(|| self.form.read(cx).list(cx));
        self.api.update(cx, |api, cx| {
            api.set_query(query, cx);
            api.set_headers(headers, cx);
            api.set_auth(auth, cx);
            match (body, form) {
                (Some(body), _) => api.set_body(body, cx),
                (_, Some(form)) => api.set_form(form, cx),
                _ => {}
            }
        });
    }

    /// The environment prefix, then what the field holds.
    fn save_uri(&mut self, cx: &mut Context<Self>) {
        let rest = self.uri.read(cx).value().to_string();
        let prefix = split_env_prefix(&self.api.read(cx).request().uri)
            .0
            .map(|name| format!("{{{{{name}}}}}"))
            .unwrap_or_default();
        self.api
            .update(cx, |api, cx| api.set_uri(format!("{prefix}{}", rest.trim()), cx));
    }

    /// F4.4: the environment variable the URI starts with, or none.
    fn set_environment(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        let rest = self.uri.read(cx).value().trim().to_string();
        let uri = match name {
            Some(name) => format!("{{{{{name}}}}}{rest}"),
            None => rest,
        };
        self.api.update(cx, |api, cx| api.set_uri(uri, cx));
    }

    /// F4.6: a new content type empties the body; one with something in it
    /// asks first.
    fn set_content_type(&mut self, content_type: ContentType, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.api.read(cx).request().clone();
        if current.content_type == content_type {
            return;
        }
        if current.body.trim().is_empty() {
            self.apply_content_type(content_type, window, cx);
            return;
        }
        let this = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let this = this.clone();
            alert
                .title(t!("editor.change_type_title").to_string())
                .description(t!("editor.change_type_description").to_string())
                .confirm()
                .ok_text(t!("common.ok").to_string())
                .ok_variant(ButtonVariant::Danger)
                .cancel_text(t!("common.cancel").to_string())
                .on_ok(move |_, window, cx| {
                    let _ = this.update(cx, |this, cx| this.apply_content_type(content_type, window, cx));
                    true
                })
        });
    }

    fn apply_content_type(&mut self, content_type: ContentType, window: &mut Window, cx: &mut Context<Self>) {
        self.api.update(cx, |api, cx| api.set_content_type(content_type, cx));
        let request = self.api.read(cx).request().clone();
        self.body = body_state(&request, window, cx);
        self._subscriptions.push(Self::watch_body(&self.body, window, cx));
        self.form = cx.new(|cx| {
            KvEditor::new(
                &[],
                KvOptions {
                    files: content_type == ContentType::Multipart,
                    ..Default::default()
                },
                window,
                cx,
            )
        });
        self._subscriptions
            .push(cx.subscribe(&self.form, |this, _, KvEvent::Changed(list), cx| {
                this.api.update(cx, |api, cx| api.set_form(list.clone(), cx));
            }));
        cx.notify();
    }

    /// F4.7: JSON indented by two spaces.
    fn format_json(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.body.read(cx).value().to_string();
        match serde_json::from_str::<serde_json::Value>(&send::strip_json_comments(&text)) {
            Ok(json) => {
                let pretty = serde_json::to_string_pretty(&json).unwrap_or(text);
                self.body.update(cx, |body, cx| body.set_value(pretty, window, cx));
            }
            Err(error) => {
                window.push_notification(Notification::warning(format!("{}: {error}", t!("editor.not_json"))), cx);
            }
        }
    }

    /// F5.1: sends — after asking for the file an `openFile` needs — or
    /// cancels the request on its way.
    pub fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.flush(cx);
        let api = self.api.read(cx);
        let picks_file =
            api.sending_since().is_none() && this_build::reads_files() && send::needs(api.request(), &[]).picks_file;
        if !picks_file {
            self.api.update(cx, |api, cx| api.send_or_cancel(None, cx));
            return;
        }
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        let api = self.api.downgrade();
        window
            .spawn(cx, async move |cx| {
                let path = match picked.await {
                    Ok(Ok(Some(paths))) => paths.into_iter().next(),
                    _ => None,
                };
                let path = path.map(|path| path.display().to_string());
                let _ = api.update(cx, |api, cx| api.send_or_cancel(path, cx));
            })
            .detach();
    }

    /// Ticks while a request is out, for the time it has taken.
    fn follow_sending(&mut self, cx: &mut Context<Self>) {
        let sending = self.api.read(cx).sending_since().is_some();
        if sending && self.ticker.is_none() {
            self.ticker = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_millis(100)).await;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        return;
                    }
                }
            }));
        } else if !sending {
            self.ticker = None;
        }
    }

    fn render_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let request = self.api.read(cx).request().clone();
        let sending = self.api.read(cx).sending_since();
        let (env, _) = split_env_prefix(&request.uri);
        let env = env.map(str::to_string);
        let environments: Vec<(String, String)> = self
            .workspace
            .read(cx)
            .environments()
            .map(|variable| (variable.name.clone(), variable.value.clone()))
            .collect();
        let this = cx.entity().downgrade();
        let method_menu = {
            let this = this.clone();
            move |menu: gpui_kit::component::menu::PopupMenu,
                  _: &mut Window,
                  _: &mut Context<gpui_kit::component::menu::PopupMenu>| {
                HttpMethod::EDITABLE.iter().fold(menu, |menu, method| {
                    let (this, method) = (this.clone(), *method);
                    menu.item(PopupMenuItem::new(method.as_str()).on_click(move |_, _, cx| {
                        let _ = this.update(cx, |this, cx| {
                            this.api.update(cx, |api, cx| api.set_method(method, cx));
                        });
                    }))
                })
            }
        };
        let env_menu = {
            let this = this.clone();
            move |menu: gpui_kit::component::menu::PopupMenu,
                  _: &mut Window,
                  _: &mut Context<gpui_kit::component::menu::PopupMenu>| {
                let clear = this.clone();
                let menu = menu.item(PopupMenuItem::new(t!("editor.no_environment").to_string()).on_click(
                    move |_, _, cx| {
                        let _ = clear.update(cx, |this, cx| this.set_environment(None, cx));
                    },
                ));
                let menu = if environments.is_empty() {
                    menu.separator()
                        .item(PopupMenuItem::new(t!("editor.no_environments").to_string()).disabled(true))
                } else {
                    menu.separator()
                };
                environments.iter().fold(menu, |menu, (name, value)| {
                    let (this, chosen) = (this.clone(), name.clone());
                    menu.item(
                        PopupMenuItem::new(format!("{name} | {value}")).on_click(move |_, _, cx| {
                            let chosen = chosen.clone();
                            let _ = this.update(cx, |this, cx| this.set_environment(Some(chosen), cx));
                        }),
                    )
                })
            }
        };
        h_flex()
            .gap_2()
            .p_2()
            .items_center()
            .child(
                Button::new("request-method")
                    .outline()
                    .small()
                    .w(px(96.))
                    .child(
                        div()
                            .font_semibold()
                            .text_color(method_color(request.method, cx))
                            .child(request.method.as_str()),
                    )
                    .dropdown_menu_with_anchor(Anchor::TopLeft, method_menu),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        Button::new("request-env")
                            .ghost()
                            .small()
                            .label(match &env {
                                Some(name) => format!("{{{{{name}}}}}"),
                                None => t!("editor.environment").to_string(),
                            })
                            .when(env.is_none(), |this| this.text_color(cx.theme().muted_foreground))
                            .dropdown_menu_with_anchor(Anchor::TopLeft, env_menu),
                    )
                    .child(div().flex_1().min_w_0().child(Input::new(&self.uri).small())),
            )
            .child(match sending {
                Some(started) => Button::new("request-send")
                    .danger()
                    .small()
                    .label(format!(
                        "{} {:.1}s",
                        t!("editor.cancel_send"),
                        started.elapsed().as_secs_f32()
                    ))
                    .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
                None => Button::new("request-send")
                    .primary()
                    .small()
                    .icon(gpui_kit::assets::IconName::Send)
                    .label(t!("editor.send").to_string())
                    .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
            })
    }

    fn render_tabs(&self, tab: RequestTab, cx: &mut Context<Self>) -> impl IntoElement {
        let request = self.api.read(cx).request().clone();
        let tabs = RequestTab::for_method(request.method);
        let count = |list: &[cyberapi_core::model::KvParam]| list.iter().filter(|kv| kv.enabled).count();
        // F4.13: how many entries each list has.
        let label = |tab: &RequestTab| {
            let (name, count) = match tab {
                RequestTab::Body => (
                    t!("editor.body"),
                    if request.content_type.is_key_value() {
                        count(&form_fields(&request.body))
                    } else {
                        0
                    },
                ),
                RequestTab::Query => (t!("editor.query"), count(&request.query)),
                RequestTab::Headers => (t!("editor.headers"), count(&request.headers)),
                RequestTab::Auth => (t!("editor.auth"), count(&request.auth)),
            };
            if count > 0 {
                format!("{name} ({count})")
            } else {
                name.to_string()
            }
        };
        let workspace = self.workspace.downgrade();
        let owned: Vec<RequestTab> = tabs.to_vec();
        TabBar::new("request-tabs")
            .underline()
            .small()
            .px_2()
            .selected_index(tabs.iter().position(|t| *t == tab).unwrap_or(0))
            .children(tabs.iter().map(|tab| Tab::new().label(label(tab))))
            .on_click(move |index, _, cx| {
                if let Some(tab) = owned.get(*index) {
                    let _ = workspace.update(cx, |workspace, cx| workspace.set_tab(*tab, cx));
                }
            })
    }

    fn render_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let request = self.api.read(cx).request().clone();
        let this = cx.entity().downgrade();
        let type_menu = move |menu: gpui_kit::component::menu::PopupMenu,
                              _: &mut Window,
                              _: &mut Context<gpui_kit::component::menu::PopupMenu>| {
            ContentType::ALL.iter().fold(menu, |menu, content_type| {
                let (this, content_type) = (this.clone(), *content_type);
                menu.item(
                    PopupMenuItem::new(content_type.as_str()).on_click(move |_, window, cx| {
                        let _ = this.update(cx, |this, cx| this.set_content_type(content_type, window, cx));
                    }),
                )
            })
        };
        let toolbar = h_flex()
            .gap_2()
            .px_2()
            .py_1()
            .items_center()
            .child(
                Button::new("body-type")
                    .outline()
                    .xsmall()
                    .label(request.content_type.as_str())
                    .dropdown_menu_with_anchor(Anchor::TopLeft, type_menu),
            )
            .when(request.content_type == ContentType::Json, |this| {
                this.child(
                    Button::new("body-format")
                        .ghost()
                        .xsmall()
                        .icon(gpui_kit::assets::IconName::WandSparkles)
                        .label(t!("editor.format").to_string())
                        .on_click(cx.listener(|this, _, window, cx| this.format_json(window, cx))),
                )
            });
        let content: AnyElement = if request.content_type.is_key_value() {
            div()
                .flex_1()
                .min_h_0()
                .px_2()
                .child(self.form.clone())
                .into_any_element()
        } else {
            div()
                .flex_1()
                .min_h_0()
                .relative()
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .child(Editor::new(&self.body).w_full().h_full().bordered(false)),
                )
                .into_any_element()
        };
        v_flex()
            .flex_1()
            .min_h_0()
            .child(toolbar)
            .child(content)
            .into_any_element()
    }
}

impl Render for RequestEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = self.workspace.read(cx).tab(cx).unwrap_or(RequestTab::Query);
        let content: AnyElement = match tab {
            RequestTab::Body => self.render_body(cx),
            RequestTab::Query => div().p_2().child(self.query.clone()).into_any_element(),
            RequestTab::Headers => div().p_2().child(self.headers.clone()).into_any_element(),
            RequestTab::Auth => v_flex()
                .p_2()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("editor.auth_hint").to_string()),
                )
                .child(self.auth.clone())
                .into_any_element(),
        };
        v_flex()
            .size_full()
            .child(self.render_bar(cx))
            .child(self.render_tabs(tab, cx))
            .child(v_flex().flex_1().min_h_0().child(content))
    }
}
