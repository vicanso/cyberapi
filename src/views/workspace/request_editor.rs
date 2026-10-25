//! The selected API's request (M4): method, environment and URI, the send
//! button, and the Body / Query / Header / Auth tabs. One per selected API;
//! everything it edits goes to the API's `ApiState`.

// The action, named apart from the `Send` trait.
use super::{KEY_CONTEXT, Send as SendAction, method_color};
use crate::json_view_local::JsonView;
use crate::states::{ApiEvent, ApiState, AppStore, WorkspaceState};
use crate::this_build;
use crate::views::graphql_hints;
use crate::views::json_tree::{JsonTree, json_view_switch};
use crate::views::kv_editor::{KvEditor, KvEvent, KvOptions};
use crate::views::open_variables_dialog;
use crate::views::template_hints::{self, HintSource};
use cyberapi_core::graphql::GraphQlBody;
use cyberapi_core::import::{request_from_curl, split_url};
use cyberapi_core::model::{ContentType, HttpMethod, HttpRequest, VariableCategory, split_env_prefix};
use cyberapi_core::prefs::RequestTab;
use cyberapi_core::send::{self, form_fields};
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::ANIMATION_DURATION;
use gpui_kit::component::input::{Editor, EditorState, Input, InputEvent, InputState};
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Selectable as _, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, AnyElement, App, AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement as _, PathPromptOptions, Render, Styled as _, Subscription, Task, Window, div, px, relative, rems,
};
use rust_i18n::t;
use std::time::Duration;

/// The headers one types most, offered in the header list's menu: the
/// name, and the value's usual start.
const COMMON_HEADERS: [(&str, &str); 7] = [
    ("Content-Type", "application/json"),
    ("Accept", "application/json"),
    ("Authorization", "Bearer "),
    ("Cache-Control", "no-cache"),
    ("Cookie", ""),
    ("User-Agent", ""),
    ("X-Requested-With", "XMLHttpRequest"),
];

/// The editor column's share of what the tree leaves, as the presets set
/// it (F4.14).
pub const EDITOR_SHARES: [f32; 3] = [0.3, 0.5, 0.7];

pub enum RequestEditorEvent {
    /// A preset asks for the editor column at this share (F4.14).
    EditorShare(f32),
}

impl EventEmitter<RequestEditorEvent> for RequestEditor {}

pub struct RequestEditor {
    workspace: Entity<WorkspaceState>,
    api: Entity<ApiState>,
    uri: Entity<InputState>,
    body: Entity<EditorState>,
    /// A GraphQL body's query, variables and operation, edited apart and
    /// kept together (`cyberapi_core::graphql::GraphQlBody`).
    graphql: GraphQlFields,
    form: Entity<KvEditor>,
    query: Entity<KvEditor>,
    headers: Entity<KvEditor>,
    auth: Entity<KvEditor>,
    /// Redraws the elapsed time while a request is out.
    ticker: Option<Task<()>>,
    /// Why the JSON body would not parse, worked out as it changes.
    json_problem: Option<String>,
    /// The body as a tree, when this API's JSON is shown that way.
    json_tree: Entity<JsonTree>,
    /// `Some` while this API is showing the reader's choice. `None` follows
    /// the saved default, including when that default changes.
    body_as_tree: Option<bool>,
    /// Whether the body editor wraps long lines (F14.7).
    wrap: bool,
    /// What typing `{{` in the body or a value offers (M6).
    hints: HintSource,
    _subscriptions: Vec<Subscription>,
}

/// The body editor for a content type: JSON and XML (in HTML's grammar)
/// are highlighted where the grammars are compiled in (the desktop). Long
/// lines wrap as the settings say (F14.7).
fn body_state(
    request: &HttpRequest,
    hints: &HintSource,
    window: &mut Window,
    cx: &mut Context<RequestEditor>,
) -> Entity<EditorState> {
    let language = match request.content_type {
        ContentType::Json => "json",
        ContentType::Xml => "html",
        _ => "text",
    };
    let text = if request.content_type.is_key_value() {
        String::new()
    } else {
        request.body.clone()
    };
    let wrap = AppStore::global(cx).state().read(cx).prefs().text.wrap;
    cx.new(|cx| {
        let mut body = EditorState::new(window, cx)
            .language(language)
            .line_number(true)
            .soft_wrap(wrap)
            .tab_size(crate::views::json_tab())
            .default_value(text);
        template_hints::complete_in(&mut body, hints);
        body
    })
}

/// What the body type menu calls a content type: its MIME type, or GraphQL
/// by name, since a GraphQL body is sent as JSON.
fn content_type_label(content_type: ContentType) -> &'static str {
    match content_type {
        ContentType::GraphQl => "GraphQL",
        other => other.as_str(),
    }
}

/// The fields of a GraphQL body.
struct GraphQlFields {
    query: Entity<EditorState>,
    variables: Entity<EditorState>,
    operation: Entity<InputState>,
}

/// A GraphQL body's fields, filled from `request` when it is one. The
/// query is highlighted as GraphQL and the variables as JSON where the
/// grammars are compiled in; both offer the template hints, and the query
/// the fields of `api`'s endpoint's schema.
fn graphql_fields(
    request: &HttpRequest,
    api: &Entity<ApiState>,
    hints: &HintSource,
    window: &mut Window,
    cx: &mut Context<RequestEditor>,
) -> GraphQlFields {
    let body = if request.content_type == ContentType::GraphQl {
        GraphQlBody::read(&request.body)
    } else {
        GraphQlBody::default()
    };
    let wrap = AppStore::global(cx).state().read(cx).prefs().text.wrap;
    let mut editor = |language: &'static str, text: String, cx: &mut Context<RequestEditor>| {
        cx.new(|cx| {
            let mut editor = EditorState::new(window, cx)
                .language(language)
                .line_number(true)
                .soft_wrap(wrap)
                .tab_size(crate::views::json_tab())
                .default_value(text);
            if language == "graphql" {
                graphql_hints::complete_in(&mut editor, api, hints);
            } else {
                template_hints::complete_in(&mut editor, hints);
            }
            editor
        })
    };
    let query = editor("graphql", body.query, cx);
    let variables = editor("json", body.variables, cx);
    let operation = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(t!("editor.graphql_operation").to_string())
            .default_value(body.operation_name)
    });
    GraphQlFields {
        query,
        variables,
        operation,
    }
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
        let hints = HintSource::new(&workspace);
        let body = body_state(&request, &hints, window, cx);
        let graphql = graphql_fields(&request, &api, &hints, window, cx);
        // Values that are templates offer the hints; Basic auth's are not.
        let kv = |list, options, hints: Option<&HintSource>, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                let kv = KvEditor::new(list, options, window, cx);
                match hints {
                    Some(hints) => kv.hints(hints.clone()),
                    None => kv,
                }
            })
        };
        let fields = form_fields(&request.body);
        let form = kv(
            &fields,
            KvOptions {
                files: request.content_type == ContentType::Multipart,
                ..Default::default()
            },
            Some(&hints),
            window,
            cx,
        );
        let query = kv(&request.query, KvOptions::default(), Some(&hints), window, cx);
        let headers = kv(
            &request.headers,
            KvOptions {
                presets: &COMMON_HEADERS,
                ..Default::default()
            },
            Some(&hints),
            window,
            cx,
        );
        let auth = kv(
            &request.auth,
            KvOptions {
                masked: true,
                ..Default::default()
            },
            None,
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
                ApiEvent::NoEnvironment(name) => {
                    let message = t!("editor.environment_missing", name = name).to_string();
                    window.push_notification(Notification::error(message), cx);
                }
                ApiEvent::GraphQlVariables(reason) => {
                    let message = t!("editor.graphql_variables_broken", reason = reason).to_string();
                    window.push_notification(Notification::error(message), cx);
                }
                ApiEvent::Curl(command) => {
                    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(command.clone()));
                    window.push_notification(Notification::success(t!("response.curl_copied").to_string()), cx);
                }
                ApiEvent::Saved(path) => {
                    window.push_notification(Notification::success(t!("export.saved", path = path).to_string()), cx);
                }
                ApiEvent::NotSaved(message) => {
                    window.push_notification(Notification::error(message.clone()), cx);
                }
                ApiEvent::NoSchema(reason) => {
                    let message = t!("editor.graphql_no_schema", reason = reason).to_string();
                    window.push_notification(Notification::error(message), cx);
                }
                ApiEvent::Failed(_) => {}
            }),
            // F4.3: the URI is saved as the field is left, or on Enter —
            // which also sends it, when there is one (F3.12). Cmd/Ctrl+Enter
            // is the workspace's own Send. A cURL command there becomes the
            // request first; a query, the Query list's (#50).
            cx.subscribe_in(&uri, window, |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Blur => {
                    if !this.take_curl(window, cx) {
                        this.take_query(window, cx);
                        this.save_uri(cx);
                    }
                }
                InputEvent::PressEnter { secondary, .. } => {
                    // A cURL command typed, or pasted in a page (which hands
                    // the field the text rather than the paste to the hook),
                    // is the request Enter then sends.
                    if !this.take_curl(window, cx) {
                        this.take_query(window, cx);
                        this.save_uri(cx);
                    }
                    let has_uri = !this.api.read(cx).request().uri.trim().is_empty();
                    if !*secondary && has_uri {
                        this.send(window, cx);
                    }
                }
                _ => {}
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
        subscriptions.extend(template_hints::watch(&body, window, cx));
        subscriptions.extend(Self::watch_graphql(&graphql, window, cx));
        let app = AppStore::global(cx).state().clone();
        let wrap = app.read(cx).prefs().text.wrap;
        subscriptions.push(cx.observe_in(&app, window, |this, app, window, cx| {
            let wrap = app.read(cx).prefs().text.wrap;
            if wrap != this.wrap {
                this.wrap = wrap;
                for editor in [&this.body, &this.graphql.query, &this.graphql.variables] {
                    editor.update(cx, |editor, cx| editor.set_soft_wrap(wrap, window, cx));
                }
            }
            this.sync_body_tree_if_shown(cx);
            cx.notify();
        }));
        let json_tree = cx.new(JsonTree::new);
        let mut editor = Self {
            workspace,
            api,
            uri,
            body,
            graphql,
            form,
            query,
            headers,
            auth,
            ticker: None,
            json_problem: None,
            json_tree,
            body_as_tree: None,
            wrap,
            hints,
            _subscriptions: subscriptions,
        };
        editor.follow_sending(cx);
        editor.check_json(cx);
        editor.sync_body_tree_if_shown(cx);
        editor
    }

    fn check_json(&mut self, cx: &mut Context<Self>) {
        let request = self.api.read(cx).request();
        let problem = (request.content_type == ContentType::Json)
            .then(|| send::json_problem(&request.body))
            .flatten();
        if problem != self.json_problem {
            self.json_problem = problem;
            cx.notify();
        }
    }

    /// The tree, when this API was switched to it; otherwise the default.
    fn body_shows_tree(&self, cx: &App) -> bool {
        self.body_as_tree
            .unwrap_or(AppStore::global(cx).state().read(cx).json_view() == JsonView::Tree)
    }

    fn sync_body_tree(&mut self, cx: &mut Context<Self>) {
        let text = self.body.read(cx).value().to_string();
        self.json_tree.update(cx, |tree, cx| tree.set_text(&text, cx));
    }

    /// Fill the tree from the editor when this body is actually drawn as one.
    fn sync_body_tree_if_shown(&mut self, cx: &mut Context<Self>) {
        if self.api.read(cx).request().content_type == ContentType::Json && self.body_shows_tree(cx) {
            self.sync_body_tree(cx);
        }
    }

    /// The keys to the URI field — once the name dialog that made the API
    /// has closed: it hands focus back at the end of its animation, over
    /// any given before.
    pub fn focus_uri(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(*ANIMATION_DURATION).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.uri.update(cx, |input, cx| input.focus(window, cx));
            });
        })
        .detach();
    }

    fn watch_body(body: &Entity<EditorState>, window: &mut Window, cx: &mut Context<Self>) -> Subscription {
        cx.subscribe_in(body, window, |this, body, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                let text = body.read(cx).value().to_string();
                this.api.update(cx, |api, cx| api.set_body(text, cx));
                this.check_json(cx);
                this.sync_body_tree_if_shown(cx);
            }
        })
    }

    /// A GraphQL body's fields, kept together as the body whenever one of
    /// them changes.
    fn watch_graphql(fields: &GraphQlFields, window: &mut Window, cx: &mut Context<Self>) -> Vec<Subscription> {
        let mut watching = vec![
            cx.subscribe_in(&fields.query, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.save_graphql(cx);
                }
            }),
            cx.subscribe_in(&fields.variables, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.save_graphql(cx);
                }
            }),
            cx.subscribe_in(&fields.operation, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.save_graphql(cx);
                }
            }),
        ];
        watching.extend(template_hints::watch(&fields.query, window, cx));
        watching.extend(template_hints::watch(&fields.variables, window, cx));
        watching
    }

    /// The GraphQL fields as the body they make.
    fn graphql_body(&self, cx: &App) -> String {
        GraphQlBody {
            query: self.graphql.query.read(cx).value().to_string(),
            variables: self.graphql.variables.read(cx).value().to_string(),
            operation_name: self.graphql.operation.read(cx).value().trim().to_string(),
        }
        .to_body()
    }

    fn save_graphql(&mut self, cx: &mut Context<Self>) {
        if self.api.read(cx).request().content_type != ContentType::GraphQl {
            return;
        }
        let body = self.graphql_body(cx);
        self.api.update(cx, |api, cx| api.set_body(body, cx));
    }

    /// The GraphQL fields made again from the request.
    fn rebuild_graphql(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let request = self.api.read(cx).request().clone();
        self.graphql = graphql_fields(&request, &self.api, &self.hints, window, cx);
        let watching = Self::watch_graphql(&self.graphql, window, cx);
        self._subscriptions.extend(watching);
    }

    /// F7.10: the request came back from the history; every field reads it.
    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let request = self.api.read(cx).request().clone();
        let rest = split_env_prefix(&request.uri).1.to_string();
        self.uri.update(cx, |input, cx| input.set_value(rest, window, cx));
        self.body = body_state(&request, &self.hints, window, cx);
        self._subscriptions.push(Self::watch_body(&self.body, window, cx));
        self._subscriptions
            .extend(template_hints::watch(&self.body, window, cx));
        self.rebuild_graphql(window, cx);
        self.form
            .update(cx, |form, cx| form.set_list(&form_fields(&request.body), window, cx));
        self.query.update(cx, |kv, cx| kv.set_list(&request.query, window, cx));
        self.headers
            .update(cx, |kv, cx| kv.set_list(&request.headers, window, cx));
        self.auth.update(cx, |kv, cx| kv.set_list(&request.auth, window, cx));
        self.check_json(cx);
        self.sync_body_tree_if_shown(cx);
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
        let body = match content_type {
            ContentType::GraphQl => Some(self.graphql_body(cx)),
            _ if content_type.is_key_value() => None,
            _ => Some(self.body.read(cx).value().to_string()),
        };
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

    /// A cURL command in the URI field becomes the request (F10.3 in place):
    /// method, URL, query, headers and body. Answers whether it was one.
    fn take_curl(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let text = self.uri.read(cx).value().to_string();
        self.fill_from_curl(&text, window, cx)
    }

    fn fill_from_curl(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(request) = request_from_curl(text) else {
            return false;
        };
        self.api.update(cx, |api, cx| api.replace_request(request, cx));
        window.push_notification(Notification::success(t!("editor.curl_filled").to_string()), cx);
        true
    }

    /// #50: a query in the URI field goes to the end of the Query list, and
    /// the field keeps the rest, so each parameter has one place. Moved as
    /// the field is committed, not as it is typed, which would take the
    /// text from under the caret.
    fn take_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.uri.read(cx).value().to_string();
        if !text.contains('?') {
            return;
        }
        let (rest, pairs) = split_url(text.trim());
        self.uri.update(cx, |input, cx| input.set_value(rest, window, cx));
        if pairs.is_empty() {
            return;
        }
        let mut query = self.query.read(cx).list(cx);
        query.extend(pairs);
        self.query.update(cx, |kv, cx| kv.set_list(&query, window, cx));
        self.api.update(cx, |api, cx| api.set_query(query, cx));
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
                .ok_text(t!("editor.change_type_ok").to_string())
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
        self.body = body_state(&request, &self.hints, window, cx);
        self._subscriptions.push(Self::watch_body(&self.body, window, cx));
        self._subscriptions
            .extend(template_hints::watch(&self.body, window, cx));
        self.rebuild_graphql(window, cx);
        self.check_json(cx);
        self.sync_body_tree_if_shown(cx);
        let hints = self.hints.clone();
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
            .hints(hints)
        });
        self._subscriptions
            .push(cx.subscribe(&self.form, |this, _, KvEvent::Changed(list), cx| {
                this.api.update(cx, |api, cx| api.set_form(list.clone(), cx));
            }));
        cx.notify();
    }

    /// F4.7: JSON indented by two spaces, its templates as written.
    fn format_json(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.body.read(cx).value().to_string();
        match send::format_json(&text) {
            Ok(pretty) => {
                // set_value does not emit Change, so the tree reads the editor itself.
                self.body.update(cx, |body, cx| body.set_value(pretty, window, cx));
                self.sync_body_tree_if_shown(cx);
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
        let headers = self.workspace.read(cx).global_headers();
        let picks_file = api.sending_since().is_none()
            && this_build::reads_files()
            && send::needs(api.request(), headers).picks_file;
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
        // What the prefix stands for: the address the request goes to.
        // A prefix nothing enabled stands for is sent as it is written.
        let env_value = env
            .as_ref()
            .and_then(|name| environments.iter().find(|(candidate, _)| candidate == name))
            .map(|(_, value)| value.clone());
        let env_tooltip = match (&env, &env_value) {
            (Some(name), Some(value)) => format!("{{{{{name}}}}} = {value}"),
            (Some(name), None) => t!("editor.environment_missing", name = name).to_string(),
            (None, _) => t!("editor.environment").to_string(),
        };
        let this = cx.entity().downgrade();
        let paste = this.clone();
        let method_menu = {
            let this = this.clone();
            let current = request.method;
            move |menu: gpui_kit::component::menu::PopupMenu,
                  _: &mut Window,
                  _: &mut Context<gpui_kit::component::menu::PopupMenu>| {
                HttpMethod::EDITABLE.iter().fold(menu, |menu, method| {
                    let (this, method) = (this.clone(), *method);
                    // Each in the colour the tree gives it.
                    let item = PopupMenuItem::element(move |_, cx| {
                        div()
                            .w(px(64.))
                            .text_xs()
                            .font_bold()
                            .text_color(method_color(method, cx))
                            .child(method.as_str())
                    });
                    menu.item(item.checked(method == current).on_click(move |_, _, cx| {
                        let _ = this.update(cx, |this, cx| {
                            this.api.update(cx, |api, cx| api.set_method(method, cx));
                        });
                    }))
                })
            }
        };
        let env_menu = {
            let this = this.clone();
            let env_now = env.clone();
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
                let manage = this.clone();
                environments
                    .iter()
                    .fold(menu, |menu, (name, value)| {
                        let (this, chosen) = (this.clone(), name.clone());
                        let (label, address) = (name.clone(), value.clone());
                        // The name, and the address it stands for in grey.
                        let item = PopupMenuItem::element(move |_, cx| {
                            h_flex()
                                .gap_3()
                                .max_w(px(420.))
                                .child(div().flex_none().child(label.clone()))
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(address.clone()),
                                )
                        });
                        menu.item(item.checked(env_now.as_ref() == Some(name)).on_click(move |_, _, cx| {
                            let chosen = chosen.clone();
                            let _ = this.update(cx, |this, cx| this.set_environment(Some(chosen), cx));
                        }))
                    })
                    .separator()
                    // F4.4: the dialog the variables are kept in.
                    .item(
                        PopupMenuItem::new(t!("editor.manage_environments").to_string()).on_click(
                            move |_, window, cx| {
                                let Some(this) = manage.upgrade() else { return };
                                let workspace = this.read(cx).workspace.clone();
                                open_variables_dialog(&workspace, VariableCategory::Environment, window, cx);
                            },
                        ),
                    )
            }
        };
        let theme = cx.theme();
        let (border, muted) = (theme.border, theme.muted_foreground);
        let method_color = method_color(request.method, cx);
        let env_color = if env_value.is_some() { theme.info } else { theme.warning };
        // F4.1–F4.4: the method, the environment and the URI read as one
        // address, the way a browser's bar does.
        let address =
            h_flex()
                .flex_1()
                .min_w_0()
                .h(px(30.))
                .items_center()
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.input)
                .bg(theme.background)
                .child(
                    Button::new("request-method")
                        .ghost()
                        .small()
                        .compact()
                        .child(
                            h_flex()
                                .gap_1()
                                .items_center()
                                // As wide as OPTIONS for every method: the
                                // address after it stays put from one API
                                // to the next.
                                .child(
                                    div()
                                        .w(rems(3.5))
                                        .text_xs()
                                        .font_bold()
                                        .text_color(method_color)
                                        .child(request.method.as_str()),
                                )
                                .child(Icon::new(IconName::ChevronDown).xsmall().text_color(muted)),
                        )
                        .dropdown_menu_with_anchor(Anchor::TopLeft, method_menu),
                )
                .child(div().w(px(1.)).h(px(16.)).bg(border))
                .child(
                    Button::new("request-env")
                        .ghost()
                        .xsmall()
                        .ml_1()
                        .child(match &env {
                            // The whole name; a long one is cut, and the
                            // tooltip and the menu still give it all.
                            Some(name) => div()
                                .max_w(rems(8.))
                                .truncate()
                                .px_1p5()
                                .rounded(theme.radius)
                                .bg(env_color.opacity(0.14))
                                .text_color(env_color)
                                .text_xs()
                                .font_medium()
                                .child(name.clone()),
                            None => div()
                                .text_xs()
                                .text_color(muted)
                                .child(t!("editor.environment").to_string()),
                        })
                        .tooltip(env_tooltip)
                        .dropdown_menu_with_anchor(Anchor::TopLeft, env_menu),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(Input::new(&self.uri).small().appearance(false).on_paste(
                            move |clipboard, window, cx| {
                                // A pasted cURL command fills the request in rather
                                // than landing in the field. A pasted URL lands,
                                // and its query then goes to the list (#50).
                                let Some(text) = clipboard.text() else { return false };
                                paste
                                    .update(cx, |this, cx| {
                                        if this.fill_from_curl(&text, window, cx) {
                                            return true;
                                        }
                                        if text.contains('?') {
                                            cx.defer_in(window, |this, window, cx| {
                                                this.take_query(window, cx);
                                                this.save_uri(cx);
                                            });
                                        }
                                        false
                                    })
                                    .unwrap_or(false)
                            },
                        )),
                )
                // Send sits in the bar, not in `Input::suffix`. The field is 24px
                // tall once its own padding is in, and suffix is for an icon.
                .child(
                    match sending {
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
                            .tooltip_with_action(t!("editor.send").to_string(), &SendAction, Some(KEY_CONTEXT))
                            .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
                    }
                    .mr_1(),
                );
        h_flex()
            .h(px(44.))
            .px_3()
            .items_center()
            .border_b_1()
            .border_color(border)
            .child(address)
    }

    fn render_tabs(&self, tab: RequestTab, cx: &mut Context<Self>) -> impl IntoElement {
        let request = self.api.read(cx).request().clone();
        let tabs = RequestTab::for_request(request.has_body_tab());
        // F4.13: how many entries are sent. A `//` line stays in the list and
        // is not counted (#43).
        let count = |list: &[cyberapi_core::model::KvParam]| list.iter().filter(|row| row.enabled).count();
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
        // F4.14: the column at 30%, 50% or 70% of what the tree leaves.
        // One outline group, the chosen share painted pressed.
        let share = AppStore::global(cx).state().read(cx).prefs().columns.editor_ratio;
        let presets = ButtonGroup::new("editor-share")
            .compact()
            .small()
            .outline()
            .children(EDITOR_SHARES.iter().enumerate().map(|(index, preset)| {
                let preset = *preset;
                Button::new(("editor-share", index))
                    .label(format!("{:.0}%", preset * 100.))
                    .selected((share - preset).abs() < 0.01)
                    .tooltip(t!("editor.share_tooltip", share = format!("{:.0}%", preset * 100.)).to_string())
            }))
            .on_click(cx.listener(|_, picks: &Vec<usize>, _, cx| {
                if let Some(&preset) = picks.first().and_then(|index| EDITOR_SHARES.get(*index)) {
                    cx.emit(RequestEditorEvent::EditorShare(preset));
                }
            }));
        TabBar::new("request-tabs")
            .underline()
            .small()
            .px_2()
            .selected_index(tabs.iter().position(|t| *t == tab).unwrap_or(0))
            .children(tabs.iter().map(|tab| Tab::new().label(label(tab))))
            .suffix(presets)
            .on_click(move |index, _, cx| {
                if let Some(tab) = owned.get(*index) {
                    let _ = workspace.update(cx, |workspace, cx| workspace.set_tab(*tab, cx));
                }
            })
    }

    /// A GraphQL body: the query over the variables, with the operation to
    /// run beside the variables' title.
    fn render_graphql(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (muted, border) = (theme.muted_foreground, theme.border);
        let editor = |state: &Entity<EditorState>| {
            div()
                .absolute()
                .inset_0()
                .child(Editor::new(state).w_full().h_full().bordered(false))
        };
        v_flex()
            .flex_1()
            .min_h_0()
            .child(div().flex_1().min_h_0().relative().child(editor(&self.graphql.query)))
            .child(
                h_flex()
                    .px_2()
                    .py_1()
                    .gap_2()
                    .items_center()
                    .border_t_1()
                    .border_color(border)
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(muted)
                            .child(t!("editor.graphql_variables").to_string()),
                    )
                    .child(div().w(px(180.)).child(Input::new(&self.graphql.operation).xsmall())),
            )
            .child(
                div()
                    .h(relative(0.35))
                    .flex_none()
                    .relative()
                    .child(editor(&self.graphql.variables)),
            )
            .into_any_element()
    }

    /// The GraphQL variables indented, as the JSON body's Format does.
    fn format_variables(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.graphql.variables.read(cx).value().to_string();
        match send::format_json(&text) {
            Ok(pretty) => {
                self.graphql
                    .variables
                    .update(cx, |editor, cx| editor.set_value(pretty, window, cx));
                self.save_graphql(cx);
            }
            Err(problem) => {
                let message = t!("editor.graphql_variables_broken", reason = problem).to_string();
                window.push_notification(Notification::error(message), cx);
            }
        }
    }

    fn render_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let request = self.api.read(cx).request().clone();
        let this = cx.entity().downgrade();
        let current = request.content_type;
        let type_menu = move |menu: gpui_kit::component::menu::PopupMenu,
                              _: &mut Window,
                              _: &mut Context<gpui_kit::component::menu::PopupMenu>| {
            ContentType::ALL.iter().fold(menu, |menu, content_type| {
                let (this, content_type) = (this.clone(), *content_type);
                menu.item(
                    PopupMenuItem::new(content_type_label(content_type))
                        .checked(content_type == current)
                        .on_click(move |_, window, cx| {
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
                    .ghost()
                    .xsmall()
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(content_type_label(request.content_type))
                            .child(
                                Icon::new(IconName::ChevronDown)
                                    .xsmall()
                                    .text_color(cx.theme().muted_foreground),
                            ),
                    )
                    .dropdown_menu_with_anchor(Anchor::TopLeft, type_menu),
            )
            .when(request.content_type == ContentType::Json, |this| {
                let tree = self.body_shows_tree(cx);
                this.child(
                    Button::new("body-format")
                        .ghost()
                        .xsmall()
                        .icon(gpui_kit::assets::IconName::WandSparkles)
                        .label(t!("editor.format").to_string())
                        .on_click(cx.listener(|this, _, window, cx| this.format_json(window, cx))),
                )
                .child(json_view_switch(
                    "request-json-view",
                    tree,
                    cx.listener(|this, picks: &Vec<usize>, _, cx| {
                        this.body_as_tree = Some(picks.first().copied() == Some(1));
                        this.sync_body_tree_if_shown(cx);
                        cx.notify();
                    }),
                ))
            })
            // Before it is sent: where the JSON goes wrong.
            .when_some(self.json_problem.clone(), |this, problem| {
                let warning = cx.theme().warning;
                this.child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .justify_end()
                        .gap_1()
                        .items_center()
                        .text_xs()
                        .text_color(warning)
                        .child(Icon::new(IconName::TriangleAlert).xsmall().text_color(warning))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .child(t!("editor.json_problem", why = problem).to_string()),
                        ),
                )
            });
        let toolbar = toolbar.when(request.content_type == ContentType::GraphQl, |this| {
            // The schema the query's fields are offered from: asked for
            // here, and asked again after the endpoint changed.
            let (asking, schema) = {
                let api = self.api.read(cx);
                (api.asking_schema(), api.schema(cx))
            };
            let label = match &schema {
                Some(schema) => crate::i18n::counted("editor.graphql_schema_types", schema.type_count()),
                None => t!("editor.graphql_schema").to_string(),
            };
            this.child(
                Button::new("body-format-variables")
                    .ghost()
                    .xsmall()
                    .icon(gpui_kit::assets::IconName::WandSparkles)
                    .label(t!("editor.format_variables").to_string())
                    .on_click(cx.listener(|this, _, window, cx| this.format_variables(window, cx))),
            )
            .child(
                Button::new("body-graphql-schema")
                    .ghost()
                    .xsmall()
                    .icon(gpui_kit::assets::IconName::ListTree)
                    .label(label)
                    .loading(asking)
                    .tooltip(t!("editor.graphql_schema_about").to_string())
                    .on_click(cx.listener(|this, _, _, cx| this.api.update(cx, |api, cx| api.ask_schema(cx)))),
            )
        });
        let content: AnyElement = if request.content_type == ContentType::GraphQl {
            self.render_graphql(cx)
        } else if request.content_type.is_key_value() {
            div()
                .id("request-form")
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .px_2()
                .pb_2()
                .child(self.form.clone())
                .into_any_element()
        } else if request.content_type == ContentType::Json && self.body_shows_tree(cx) {
            div()
                .flex_1()
                .min_h_0()
                .relative()
                .child(div().absolute().inset_0().child(self.json_tree.clone()))
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
            // A long list scrolls rather than running off the column.
            RequestTab::Query => div()
                .id("request-query")
                .size_full()
                .overflow_y_scrollbar()
                .p_2()
                .child(self.query.clone())
                .into_any_element(),
            RequestTab::Headers => div()
                .id("request-headers")
                .size_full()
                .overflow_y_scrollbar()
                .p_2()
                .child(self.headers.clone())
                .into_any_element(),
            RequestTab::Auth => v_flex()
                .id("request-auth")
                .size_full()
                .overflow_y_scrollbar()
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
