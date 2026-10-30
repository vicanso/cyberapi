//! The selected API's request (M4): method, environment and URI, the send
//! button, and the Body / Query / Header / Auth tabs. One per selected API;
//! everything it edits goes to the API's `ApiState`.

// The action, named apart from the `Send` trait.
use super::auth_editor::{AuthEditor, AuthFor};
use super::prompt_dialog::open_prompt_dialog;
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
use cyberapi_core::grpc::{FROM_REFLECTION, GrpcBody};
use cyberapi_core::import::{request_from_curl, split_url};
use cyberapi_core::model::{ContentType, HttpMethod, HttpRequest, VariableCategory, split_env_prefix};
use cyberapi_core::prefs::RequestTab;
use cyberapi_core::send::{self, form_fields, unsent_body};
use cyberapi_core::template::Answers;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::ANIMATION_DURATION;
use gpui_kit::component::input::{Editor, EditorState, Input, InputEvent, InputState, RopeExt as _};
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, AnyElement, App, AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement as _, PathPromptOptions, Render, Styled as _, Subscription, Task, Window, div, px, relative, rems,
};
use rust_i18n::t;
use std::collections::HashMap;
use std::time::Duration;

/// The headers one types most, offered in the header list's menu: the
/// name, and the value's usual start.
pub(super) const COMMON_HEADERS: [(&str, &str); 7] = [
    ("Content-Type", "application/json"),
    ("Accept", "application/json"),
    ("Authorization", "Bearer "),
    ("Cache-Control", "no-cache"),
    ("Cookie", ""),
    ("User-Agent", ""),
    ("X-Requested-With", "XMLHttpRequest"),
];

/// A header list's line as text, for its bulk editor (#43).
pub(super) const HEADER_EXAMPLE: &str = "Accept: application/json";

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
    /// #71: the API's own sending settings.
    settings: Entity<super::request_settings::RequestSettingsView>,
    /// A1: what its answers are checked against.
    assertions: Entity<super::assertions_editor::AssertionsEditor>,
    /// F3.
    docs: Entity<super::docs_editor::DocsEditor>,
    /// The response whose GraphQL errors the query marks, and the query it
    /// was sent with (#70).
    marked: Option<(String, String)>,
    /// A gRPC call's message (`cyberapi_core::grpc::GrpcBody`); the rest of
    /// the call is the API state's.
    grpc_message: Entity<EditorState>,
    form: Entity<KvEditor>,
    query: Entity<KvEditor>,
    headers: Entity<KvEditor>,
    auth: Entity<AuthEditor>,
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

/// A gRPC call's message editor: JSON, with the template hints.
fn grpc_message_state(
    request: &HttpRequest,
    hints: &HintSource,
    window: &mut Window,
    cx: &mut Context<RequestEditor>,
) -> Entity<EditorState> {
    let message = if request.method == HttpMethod::Grpc {
        GrpcBody::read(&request.body).message
    } else {
        String::new()
    };
    let wrap = AppStore::global(cx).state().read(cx).prefs().text.wrap;
    cx.new(|cx| {
        let mut editor = EditorState::new(window, cx)
            .language("json")
            .line_number(true)
            .soft_wrap(wrap)
            .tab_size(crate::views::json_tab())
            .default_value(message);
        template_hints::complete_in(&mut editor, hints);
        editor
    })
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

/// What the address field says while empty: a gRPC call's address is
/// `host:port` more often than a URL, and it takes no cURL command (#73).
fn uri_placeholder(method: HttpMethod) -> String {
    match method {
        HttpMethod::Grpc => t!("editor.grpc_uri_placeholder"),
        HttpMethod::WebSocket => t!("editor.ws_uri_placeholder"),
        _ => t!("editor.uri_placeholder"),
    }
    .to_string()
}

/// How many of the settings an API sets of its own (#71).
fn overridden(settings: &cyberapi_core::prefs::RequestSettings) -> usize {
    let timeouts = [
        settings.timeouts.connect,
        settings.timeouts.write,
        settings.timeouts.read,
    ];
    timeouts.iter().filter(|seconds| **seconds > 0).count()
        + usize::from(settings.follow_redirects.is_some())
        + usize::from(settings.accept_invalid_certs.is_some())
        + usize::from(settings.http_version.is_some())
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
                .placeholder(uri_placeholder(request.method))
                .default_value(split_env_prefix(&request.uri).1.to_string())
        });
        let hints = HintSource::new(&workspace);
        let body = body_state(&request, &hints, window, cx);
        let graphql = graphql_fields(&request, &api, &hints, window, cx);
        let grpc_message = grpc_message_state(&request, &hints, window, cx);
        // Values that are templates offer the hints.
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
        let query = kv(
            &request.query,
            KvOptions {
                example: "page: 1",
                ..Default::default()
            },
            Some(&hints),
            window,
            cx,
        );
        let headers = kv(
            &request.headers,
            KvOptions {
                presets: &COMMON_HEADERS,
                example: HEADER_EXAMPLE,
                ..Default::default()
            },
            Some(&hints),
            window,
            cx,
        );
        let auth = {
            let (api, workspace) = (api.clone(), workspace.clone());
            cx.new(|cx| AuthEditor::new(AuthFor::Api(api), workspace, window, cx))
        };
        let settings = {
            let (api, hints) = (api.clone(), hints.clone());
            cx.new(|cx| super::request_settings::RequestSettingsView::new(api, hints, window, cx))
        };
        let assertions = {
            let api = api.clone();
            cx.new(|cx| super::assertions_editor::AssertionsEditor::new(api, window, cx))
        };
        let docs = {
            let api = api.clone();
            cx.new(|cx| super::docs_editor::DocsEditor::new(api, window, cx))
        };
        let mut subscriptions = vec![
            cx.observe(&api, |this, _, cx| {
                this.follow_sending(cx);
                this.mark_graphql_errors(cx);
                cx.notify();
            }),
            cx.observe(&workspace, |_, _, cx| cx.notify()),
            cx.subscribe_in(&api, window, |this, _, event, window, cx| match event {
                ApiEvent::RequestReplaced => this.reload(window, cx),
                ApiEvent::NotSent(message) => {
                    window.push_notification(Notification::error(message.clone()), cx);
                }
                ApiEvent::NoEnvironment { name, within } => {
                    let message = match within {
                        Some(within) => t!("editor.environment_missing_in", name = name, within = within),
                        None => t!("editor.environment_missing", name = name),
                    }
                    .to_string();
                    window.push_notification(Notification::error(message), cx);
                }
                ApiEvent::EnvUnset(name) => {
                    let message = t!("editor.env_unset", name = name).to_string();
                    window.push_notification(Notification::error(message), cx);
                }
                ApiEvent::GraphQlVariables(reason) => {
                    let message = t!("editor.graphql_variables_broken", reason = reason).to_string();
                    window.push_notification(Notification::error(message), cx);
                }
                ApiEvent::Code(target, code) => {
                    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(code.clone()));
                    window.push_notification(Notification::success(crate::views::copied_code(*target, code)), cx);
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
                ApiEvent::GrpcDefinitions { methods, example } => {
                    let message = crate::i18n::counted("editor.grpc_loaded", *methods);
                    window.push_notification(Notification::success(message), cx);
                    if let Some(example) = example
                        && this.grpc_message.read(cx).value().trim().is_empty()
                    {
                        this.set_grpc_message(example.clone(), window, cx);
                    }
                }
                ApiEvent::NoGrpcDefinitions(reason) => {
                    let message = t!("editor.grpc_no_definitions", reason = reason).to_string();
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
                        this.take_prefix(window, cx);
                        this.save_uri(cx);
                    }
                }
                InputEvent::PressEnter { secondary, .. } => {
                    // A cURL command typed, or pasted in a page (which hands
                    // the field the text rather than the paste to the hook),
                    // is the request Enter then sends.
                    if !this.take_curl(window, cx) {
                        this.take_query(window, cx);
                        this.take_prefix(window, cx);
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
            cx.subscribe(&form, |this, _, KvEvent::Changed(list), cx| {
                this.api.update(cx, |api, cx| api.set_form(list.clone(), cx));
            }),
        ];
        subscriptions.push(Self::watch_body(&body, window, cx));
        subscriptions.extend(template_hints::watch(&body, window, cx));
        subscriptions.extend(Self::watch_graphql(&graphql, window, cx));
        subscriptions.extend(Self::watch_grpc(&grpc_message, window, cx));
        let app = AppStore::global(cx).state().clone();
        let wrap = app.read(cx).prefs().text.wrap;
        subscriptions.push(cx.observe_in(&app, window, |this, app, window, cx| {
            let wrap = app.read(cx).prefs().text.wrap;
            if wrap != this.wrap {
                this.wrap = wrap;
                for editor in [
                    &this.body,
                    &this.graphql.query,
                    &this.graphql.variables,
                    &this.grpc_message,
                ] {
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
            marked: None,
            settings,
            grpc_message,
            form,
            query,
            headers,
            auth,
            assertions,
            docs,
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

    /// The address field's text. Tests read it to see which API the panel shows.
    #[cfg(test)]
    pub(super) fn shown_uri(&self, cx: &App) -> String {
        self.uri.read(cx).value().to_string()
    }

    /// `text` in the address field, not given to the API yet: typed, and
    /// neither entered nor left.
    #[cfg(test)]
    pub(super) fn type_uri(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.uri
            .update(cx, |uri, cx| uri.set_value(text.to_string(), window, cx));
    }

    /// The keys to the URI field now, its text selected to type over (#64).
    pub fn focus_uri_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.uri.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
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
            // A gRPC call's body is its own editor's: this one is hidden.
            if matches!(event, InputEvent::Change) && this.api.read(cx).request().method != HttpMethod::Grpc {
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
                    this.mark_graphql_errors(cx);
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

    /// A gRPC call's message, kept in its body as it changes.
    fn watch_grpc(message: &Entity<EditorState>, window: &mut Window, cx: &mut Context<Self>) -> Vec<Subscription> {
        let mut watching = vec![cx.subscribe_in(message, window, |this, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.save_grpc(cx);
            }
        })];
        watching.extend(template_hints::watch(message, window, cx));
        watching
    }

    /// The call's body with the message as the editor holds it.
    fn grpc_body(&self, cx: &App) -> String {
        let mut body = GrpcBody::read(&self.api.read(cx).request().body);
        body.message = self.grpc_message.read(cx).value().to_string();
        body.to_body()
    }

    fn save_grpc(&mut self, cx: &mut Context<Self>) {
        if self.api.read(cx).request().method != HttpMethod::Grpc {
            return;
        }
        let body = self.grpc_body(cx);
        self.api.update(cx, |api, cx| api.set_body(body, cx));
    }

    fn set_grpc_message(&mut self, message: String, window: &mut Window, cx: &mut Context<Self>) {
        self.grpc_message
            .update(cx, |editor, cx| editor.set_value(message, window, cx));
        self.save_grpc(cx);
    }

    /// Calls `service`'s `method`. A message not written yet — empty, or the
    /// example of the method chosen before — becomes the new one's example.
    fn choose_grpc_method(&mut self, service: &str, method: &str, window: &mut Window, cx: &mut Context<Self>) {
        let before = self.api.read(cx).grpc_example(cx);
        let example = self
            .api
            .update(cx, |api, cx| api.choose_grpc_method(service, method, cx));
        let written = self.grpc_message.read(cx).value().trim().to_string();
        let untouched = written.is_empty() || before.is_some_and(|before| before.trim() == written);
        if let Some(example) = example.filter(|_| untouched) {
            self.set_grpc_message(example, window, cx);
        }
    }

    /// Before asking the server something for the request (its schema, its
    /// reflection): what the fields hold is the request's — the address
    /// still being typed too (#56) — and there is an address to ask.
    fn ready_to_ask(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.flush(cx);
        if self.api.read(cx).request().uri.trim().is_empty() {
            window.push_notification(Notification::error(t!("editor.address_first").to_string()), cx);
            self.uri.update(cx, |input, cx| input.focus(window, cx));
            return false;
        }
        true
    }

    /// The `.proto` files the reader picks, compiled for the definitions.
    fn import_protos(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        let api = self.api.downgrade();
        window
            .spawn(cx, async move |cx| {
                if let Ok(Ok(Some(paths))) = picked.await
                    && !paths.is_empty()
                {
                    let _ = api.update(cx, |api, cx| api.import_protos(paths, cx));
                }
            })
            .detach();
    }

    /// The message indented, as the JSON body's Format does.
    fn format_grpc_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.grpc_message.read(cx).value().to_string();
        match send::format_json(&text) {
            Ok(pretty) => self.set_grpc_message(pretty, window, cx),
            Err(problem) => {
                let message = t!("editor.grpc_message_broken", reason = problem).to_string();
                window.push_notification(Notification::error(message), cx);
            }
        }
    }

    /// A gRPC call's body: which method it calls and where its definitions
    /// come from, then its message.
    fn render_grpc(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let (body, known, loading, open, finished) = {
            let api = self.api.read(cx);
            (
                GrpcBody::read(&api.request().body),
                api.grpc_definitions(cx),
                api.loading_grpc(),
                api.call_open(),
                api.call_finished(),
            )
        };
        // The service by its own name: the package is in the menu.
        let chosen = (!body.service.is_empty()).then(|| {
            let service = body.service.rsplit('.').next().unwrap_or(&body.service);
            format!("{service}/{}", body.method)
        });
        let methods = known
            .as_ref()
            .map(|known| known.definitions.methods())
            .unwrap_or_default();
        let this = cx.entity().downgrade();
        let current = (body.service.clone(), body.method.clone());
        let method_menu = move |menu: gpui_kit::component::menu::PopupMenu,
                                _: &mut Window,
                                _: &mut Context<gpui_kit::component::menu::PopupMenu>| {
            if methods.is_empty() {
                return menu.item(PopupMenuItem::label(t!("editor.grpc_no_methods").to_string()));
            }
            let mut menu = menu.max_h(px(420.)).scrollable(true);
            let mut service = "";
            for method in &methods {
                // Each service's methods under its name.
                if method.service != service {
                    service = &method.service;
                    menu = menu.item(PopupMenuItem::label(service.to_string()));
                }
                let label = if method.client_streaming && method.server_streaming {
                    format!("{} · {}", method.method, t!("editor.grpc_bidi_streaming"))
                } else if method.client_streaming {
                    format!("{} · {}", method.method, t!("editor.grpc_client_streaming"))
                } else if method.server_streaming {
                    format!("{} · {}", method.method, t!("editor.grpc_server_streaming"))
                } else {
                    method.method.clone()
                };
                let (this, picked) = (this.clone(), (method.service.clone(), method.method.clone()));
                let checked = picked == current;
                menu = menu.item(
                    PopupMenuItem::new(label)
                        .checked(checked)
                        .on_click(move |_, window, cx| {
                            let _ =
                                this.update(cx, |this, cx| this.choose_grpc_method(&picked.0, &picked.1, window, cx));
                        }),
                );
            }
            menu
        };
        let source = match known.as_ref().map(|known| known.source.as_str()) {
            Some(FROM_REFLECTION) => t!("editor.grpc_from_reflection").to_string(),
            Some(files) => t!("editor.grpc_from_files", files = files).to_string(),
            None => t!("editor.grpc_no_source").to_string(),
        };
        let toolbar = h_flex()
            .gap_2()
            .px_2()
            .py_1()
            .items_center()
            .overflow_hidden()
            .child(
                Button::new("grpc-method")
                    .ghost()
                    .xsmall()
                    .when(!body.service.is_empty(), |this| this.tooltip(body.path()))
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .child(chosen.unwrap_or_else(|| t!("editor.grpc_choose_method").to_string())),
                            )
                            .child(Icon::new(IconName::ChevronDown).xsmall().text_color(muted)),
                    )
                    .dropdown_menu_with_anchor(Anchor::TopLeft, method_menu),
            )
            .child(
                Button::new("grpc-reflect")
                    .ghost()
                    .xsmall()
                    .icon(gpui_kit::assets::IconName::ListTree)
                    .label(t!("editor.grpc_reflect").to_string())
                    .loading(loading)
                    .tooltip(t!("editor.grpc_reflect_about").to_string())
                    .on_click(cx.listener(|this, _, window, cx| {
                        if this.ready_to_ask(window, cx) {
                            this.api.update(cx, |api, cx| api.load_grpc_reflection(cx));
                        }
                    })),
            )
            .when(this_build::reads_files(), |this| {
                this.child(
                    Button::new("grpc-import")
                        .ghost()
                        .xsmall()
                        .icon(gpui_kit::assets::IconName::FileCode)
                        .label(t!("editor.grpc_import").to_string())
                        .disabled(loading)
                        .on_click(cx.listener(|this, _, window, cx| this.import_protos(window, cx))),
                )
            })
            .child(
                Button::new("grpc-example")
                    .ghost()
                    .xsmall()
                    .icon(gpui_kit::assets::IconName::Braces)
                    .tooltip(t!("editor.grpc_example").to_string())
                    .disabled(body.service.is_empty())
                    .on_click(cx.listener(|this, _, window, cx| {
                        if let Some(example) = this.api.read(cx).grpc_example(cx) {
                            this.set_grpc_message(example, window, cx);
                        }
                    })),
            )
            .child(
                Button::new("grpc-format")
                    .ghost()
                    .xsmall()
                    .icon(gpui_kit::assets::IconName::WandSparkles)
                    .tooltip(t!("editor.format").to_string())
                    .on_click(cx.listener(|this, _, window, cx| this.format_grpc_message(window, cx))),
            )
            .when(body.client_streams(), |this| {
                this.child(
                    Button::new("grpc-send-message")
                        .primary()
                        .xsmall()
                        .icon(gpui_kit::assets::IconName::Send)
                        .label(t!("editor.ws_send_message").to_string())
                        .disabled(!open || finished)
                        .tooltip_with_action(t!("editor.ws_send_message").to_string(), &SendAction, Some(KEY_CONTEXT))
                        .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
                )
                .when(!body.both_streams(), |this| {
                    this.child(
                        Button::new("grpc-end-send")
                            .ghost()
                            .xsmall()
                            .label(t!("editor.grpc_end_send").to_string())
                            .disabled(!open || finished)
                            .tooltip(t!("editor.grpc_end_send_about").to_string())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.api.update(cx, |api, cx| api.finish_grpc(cx));
                            })),
                    )
                })
                .when(!open, |this| {
                    this.child(div().min_w_0().truncate().text_xs().text_color(muted).child(
                        if this_build::streams_bodies() {
                            t!("editor.grpc_message_hint").to_string()
                        } else {
                            t!("editor.grpc_stream_desktop_only").to_string()
                        },
                    ))
                })
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_right()
                    .text_xs()
                    .text_color(muted)
                    .truncate()
                    .child(source),
            );
        v_flex()
            .flex_1()
            .min_h_0()
            .child(toolbar)
            .child(
                div().flex_1().min_h_0().relative().child(
                    div()
                        .absolute()
                        .inset_0()
                        .child(Editor::new(&self.grpc_message).w_full().h_full().bordered(false)),
                ),
            )
            .into_any_element()
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
        // A gRPC call keeps its content type unused: the fields are hidden.
        let request = self.api.read(cx).request();
        if request.content_type != ContentType::GraphQl || request.method == HttpMethod::Grpc {
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
        self.marked = None;
        self.mark_graphql_errors(cx);
    }

    /// #70: where the shown response's errors are, marked in the query it
    /// was sent with — while the query is that one still: once it changes,
    /// the places are someone else's.
    fn mark_graphql_errors(&mut self, cx: &mut Context<Self>) {
        let query = self.graphql.query.read(cx).value().to_string();
        let api = self.api.read(cx);
        let found = api
            .shown_response()
            .filter(|_| api.request().content_type == ContentType::GraphQl)
            .filter(|response| {
                response.request.content_type == ContentType::GraphQl
                    && GraphQlBody::read(&response.request.body).query == query
            });
        // What is marked already is not read again (PERF-2).
        let key = found.map(|response| (response.id.clone(), query.clone()));
        if key == self.marked {
            return;
        }
        let places = found
            .map(|response| api.graphql_errors(response).places.clone())
            .unwrap_or_default();
        self.marked = key;
        let marks: Vec<gpui_kit::base::input::Diagnostic> = places
            .into_iter()
            .map(|place| {
                // The word the place starts, or its one character.
                let line = query.lines().nth(place.line - 1).unwrap_or_default();
                let start = place.column - 1;
                let word = line
                    .chars()
                    .skip(start)
                    .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
                    .count()
                    .max(1);
                let at =
                    |column: usize| gpui_kit::component::input::Position::new((place.line - 1) as u32, column as u32);
                gpui_kit::base::input::Diagnostic {
                    range: at(start)..at(start + word),
                    severity: gpui_kit::base::input::DiagnosticSeverity::Error,
                    message: place.message.into(),
                    ..Default::default()
                }
            })
            .collect();
        self.graphql.query.update(cx, |editor, cx| {
            if let Some(set) = editor.diagnostics_mut() {
                set.clear();
                set.extend(marks);
            }
            cx.notify();
        });
    }

    /// F7.10: the request came back from the history; every field reads it.
    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let request = self.api.read(cx).request().clone();
        let rest = split_env_prefix(&request.uri).1.to_string();
        self.uri.update(cx, |input, cx| {
            input.set_value(rest, window, cx);
            input.set_placeholder(uri_placeholder(request.method), window, cx);
        });
        self.body = body_state(&request, &self.hints, window, cx);
        self._subscriptions.push(Self::watch_body(&self.body, window, cx));
        self._subscriptions
            .extend(template_hints::watch(&self.body, window, cx));
        self.rebuild_graphql(window, cx);
        self.settings = {
            let (api, hints) = (self.api.clone(), self.hints.clone());
            cx.new(|cx| super::request_settings::RequestSettingsView::new(api, hints, window, cx))
        };
        self.assertions = {
            let api = self.api.clone();
            cx.new(|cx| super::assertions_editor::AssertionsEditor::new(api, window, cx))
        };
        self.docs = {
            let api = self.api.clone();
            cx.new(|cx| super::docs_editor::DocsEditor::new(api, window, cx))
        };
        self.auth = {
            let (api, workspace) = (self.api.clone(), self.workspace.clone());
            cx.new(|cx| AuthEditor::new(AuthFor::Api(api), workspace, window, cx))
        };
        self.grpc_message = grpc_message_state(&request, &self.hints, window, cx);
        let watching = Self::watch_grpc(&self.grpc_message, window, cx);
        self._subscriptions.extend(watching);
        self.form
            .update(cx, |form, cx| form.set_list(&form_fields(&request.body), window, cx));
        self.query.update(cx, |kv, cx| kv.set_list(&request.query, window, cx));
        self.headers
            .update(cx, |kv, cx| kv.set_list(&request.headers, window, cx));
        self.check_json(cx);
        self.sync_body_tree_if_shown(cx);
        cx.notify();
    }

    /// Hands the API what the fields hold now, ahead of their own delays:
    /// what was typed just before sending goes with it.
    pub(super) fn flush(&mut self, cx: &mut Context<Self>) {
        self.save_uri(cx);
        let (query, headers) = (self.query.read(cx).list(cx), self.headers.read(cx).list(cx));
        let (method, content_type) = {
            let request = self.api.read(cx).request();
            (request.method, request.content_type)
        };
        let body = match content_type {
            // A gRPC call's body is its message's, whatever the type.
            _ if method == HttpMethod::Grpc => Some(self.grpc_body(cx)),
            ContentType::GraphQl => Some(self.graphql_body(cx)),
            _ if content_type.is_key_value() => None,
            _ => Some(self.body.read(cx).value().to_string()),
        };
        let form = (content_type.is_key_value() && method != HttpMethod::Grpc).then(|| self.form.read(cx).list(cx));
        self.api.update(cx, |api, cx| {
            api.set_query(query, cx);
            api.set_headers(headers, cx);
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
        let unsent = unsent_body(&request).then_some(request.method);
        self.api.update(cx, |api, cx| api.replace_request(request, cx));
        // The fields read the request now, not once the event is handled:
        // Enter sends at once, and sending first saves what the fields hold
        // — the URL field still the command (#55).
        self.reload(window, cx);
        window.push_notification(Notification::success(t!("editor.curl_filled").to_string()), cx);
        if let Some(method) = unsent {
            super::tell_unsent_body(method, window, cx);
        }
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
        let rest = self.uri.read(cx).value().trim().to_string();
        // A `{{NAME}}` typed at the field's start is the prefix now, in
        // place of the one picked.
        let uri = if split_env_prefix(&rest).0.is_some() {
            rest
        } else {
            let prefix = split_env_prefix(&self.api.read(cx).request().uri)
                .0
                .map(|name| format!("{{{{{name}}}}}"))
                .unwrap_or_default();
            format!("{prefix}{rest}")
        };
        self.api.update(cx, |api, cx| api.set_uri(uri, cx));
    }

    /// F4.4: a `{{NAME}}` typed at the start of the field is the URI's
    /// prefix, which the picker beside it shows: the field keeps the rest.
    fn take_prefix(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.uri.read(cx).value().trim().to_string();
        let (Some(_), rest) = split_env_prefix(&text) else {
            return;
        };
        let rest = rest.to_string();
        self.save_uri(cx);
        self.uri.update(cx, |input, cx| input.set_value(rest, window, cx));
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

    /// F5.1: sends — after asking for what its `prompt`s (D1) and its
    /// `openFile` need, and in production whether it should (D6) — or
    /// cancels the request on its way.
    pub fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.flush(cx);
        let api = self.api.read(cx);
        // E1 / E2: on an open socket or call, the message goes. While it is
        // still connecting, this cancels it.
        if api.sending_since().is_some() && !api.socket_open() && !api.call_open() {
            self.api
                .update(cx, |api, cx| api.send_or_cancel(Answers::default(), cx));
            return;
        }
        // D6: what starts — not a message on what is open — in production.
        let workspace = self.workspace.read(cx);
        if api.sending_since().is_none() && workspace.asks_before(api.request(), cx) {
            let request = api.request();
            let what = match (request.content_type, request.method) {
                (ContentType::GraphQl, HttpMethod::Get | HttpMethod::Post) => format!("mutation {}", request.uri),
                (_, method) => format!("{} {}", method.as_str(), request.uri),
            };
            let environment = workspace
                .current_environment()
                .map(|environment| environment.name.clone())
                .unwrap_or_default();
            let (editor, workspace) = (cx.entity().downgrade(), self.workspace.downgrade());
            super::production_confirm::confirm_production(environment, what, window, cx, move |stop, window, cx| {
                if stop {
                    let _ = workspace.update(cx, |workspace, cx| workspace.stop_asking(cx));
                }
                let _ = editor.update(cx, |editor, cx| editor.send_unasked(window, cx));
            });
            return;
        }
        self.send_unasked(window, cx);
    }

    /// Sends, once whatever asks first (D6) has.
    fn send_unasked(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let api = self.api.read(cx);
        let workspace = self.workspace.read(cx);
        let inherited = workspace.inherited(api.api());
        let needs = send::needs(api.request(), &workspace.global_headers(), &inherited);
        let picks_file = needs.picks_file && this_build::reads_files();
        if needs.prompts.is_empty() {
            self.pick_and_send(picks_file, HashMap::new(), cx);
            return;
        }
        let typed = api.typed(cx).prompts;
        let editor = cx.entity().downgrade();
        open_prompt_dialog(window, cx, needs.prompts, &typed, move |prompts, _, cx| {
            let _ = editor.update(cx, |editor, cx| editor.pick_and_send(picks_file, prompts, cx));
        });
    }

    /// Sends with what was typed for its `prompt`s — after asking for the
    /// file an `openFile` needs, when it needs one.
    fn pick_and_send(&mut self, picks_file: bool, prompts: HashMap<String, String>, cx: &mut Context<Self>) {
        // E1: a message on the open WebSocket; files are not asked for.
        if self.api.read(cx).socket_open() {
            self.api
                .update(cx, |api, cx| api.send_message(Answers { file: None, prompts }, cx));
            return;
        }
        // E2: a message on the open gRPC call.
        if self.api.read(cx).call_open() {
            self.api
                .update(cx, |api, cx| api.send_grpc_message(Answers { file: None, prompts }, cx));
            return;
        }
        if !picks_file {
            self.api
                .update(cx, |api, cx| api.send_or_cancel(Answers { file: None, prompts }, cx));
            return;
        }
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        let api = self.api.downgrade();
        cx.spawn(async move |_, cx| {
            let file = match picked.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                _ => None,
            };
            let file = file.map(|path| path.display().to_string());
            let _ = api.update(cx, |api, cx| api.send_or_cancel(Answers { file, prompts }, cx));
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
        let streaming = self.api.read(cx).live().is_some();
        let (env, _) = split_env_prefix(&request.uri);
        let env = env.map(str::to_string);
        let mut environments: Vec<(String, String)> = self
            .workspace
            .read(cx)
            .environments()
            .into_iter()
            .map(|variable| (variable.name, variable.value))
            .collect();
        // D5: the folders' variables and the API's own stand in for the
        // environment's of the same names.
        let api_id = self.api.read(cx).api().clone();
        for (name, value) in self
            .workspace
            .read(cx)
            .inherited(&api_id)
            .values(&request.variables, &[])
        {
            environments.retain(|(given, _)| *given != name);
            environments.push((name, value));
        }
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
                    menu.item(item.checked(method == current).on_click(move |_, window, cx| {
                        let _ = this.update(cx, |this, cx| {
                            this.api.update(cx, |api, cx| api.set_method(method, cx));
                            this.uri.update(cx, |input, cx| {
                                input.set_placeholder(uri_placeholder(method), window, cx)
                            });
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
                        // E1: a WebSocket connects, and is closed or given up.
                        _ if request.method == HttpMethod::WebSocket => self.socket_button(sending, cx),
                        // E2: a method that streams from the client connects.
                        _ if request.method == HttpMethod::Grpc && GrpcBody::read(&request.body).client_streams() => {
                            self.grpc_call_button(sending, cx)
                        }
                        // E3: a GraphQL API written at a WebSocket address subscribes.
                        None if request.content_type == ContentType::GraphQl
                            && cyberapi_core::graphql_ws::written_as_socket(&request.uri) =>
                        {
                            self.graphql_subscribe_button(cx)
                        }
                        // An event stream coming is stopped, and kept.
                        Some(started) => Button::new("request-send")
                            .danger()
                            .small()
                            .label(format!(
                                "{} {:.1}s",
                                if streaming {
                                    t!("editor.stop_stream")
                                } else {
                                    t!("editor.cancel_send")
                                },
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
        let tabs = RequestTab::for_request(&request);
        // F4.13: how many entries are sent. A `//` line stays in the list and
        // is not counted (#43).
        let count = |list: &[cyberapi_core::model::KvParam]| list.iter().filter(|row| row.enabled).count();
        // A gRPC call's body is its message, its headers the metadata.
        let grpc = request.method == HttpMethod::Grpc;
        let label = |tab: &RequestTab| {
            let (name, count) = match tab {
                RequestTab::Body if grpc || request.method == HttpMethod::WebSocket => (t!("editor.grpc_message"), 0),
                RequestTab::Headers if grpc => (t!("editor.grpc_metadata"), count(&request.headers)),
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
                // #66: one when it signs in itself.
                RequestTab::Auth => (t!("editor.auth"), usize::from(request.auth.signs())),
                // A1: how many are on.
                RequestTab::Assertions => (
                    t!("editor.assertions"),
                    request.assertions.iter().filter(|assertion| assertion.enabled).count(),
                ),
                // #71: how many it sets of its own.
                RequestTab::Settings => (t!("editor.settings"), overridden(&request.settings)),
                // F3: how many examples it keeps.
                RequestTab::Docs => (t!("editor.docs"), request.examples.len()),
            };
            if count > 0 {
                format!("{name} ({count})")
            } else {
                name.to_string()
            }
        };
        let (workspace, api) = (self.workspace.downgrade(), self.api.clone());
        let owned: Vec<RequestTab> = tabs.to_vec();
        // F4.14: the column at 30%, 50% or 70% of what the tree leaves. One
        // button saying the share it is at, the presets in its menu: the
        // tabs beside it keep the room.
        let share = AppStore::global(cx).state().read(cx).prefs().columns.editor_ratio;
        let percent = |share: f32| format!("{:.0}%", share * 100.);
        let editor = cx.entity().downgrade();
        let share_menu = move |menu: gpui_kit::component::menu::PopupMenu,
                               _: &mut Window,
                               _: &mut Context<gpui_kit::component::menu::PopupMenu>| {
            EDITOR_SHARES.iter().fold(menu, |menu, preset| {
                let (editor, preset) = (editor.clone(), *preset);
                menu.item(
                    PopupMenuItem::new(percent(preset))
                        .checked((share - preset).abs() < 0.01)
                        .on_click(move |_, _, cx| {
                            let _ = editor.update(cx, |_, cx| cx.emit(RequestEditorEvent::EditorShare(preset)));
                        }),
                )
            })
        };
        let muted = cx.theme().muted_foreground;
        let presets = Button::new("editor-share")
            .ghost()
            .xsmall()
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .text_xs()
                    .text_color(muted)
                    .child(percent(share))
                    .child(Icon::new(IconName::ChevronDown).xsmall()),
            )
            .tooltip(t!("editor.share_tooltip", share = percent(share)).to_string())
            .dropdown_menu_with_anchor(Anchor::TopRight, share_menu);
        TabBar::new("request-tabs")
            .underline()
            .small()
            .px_2()
            .selected_index(tabs.iter().position(|t| *t == tab).unwrap_or(0))
            .children(tabs.iter().map(|tab| Tab::new().label(label(tab))))
            .suffix(presets)
            .on_click(move |index, _, cx| {
                if let Some(tab) = owned.get(*index) {
                    let _ = workspace.update(cx, |workspace, cx| workspace.set_tab(&api, *tab, cx));
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

    /// Asks the endpoint for its schema, once there is an address to ask.
    fn fetch_schema(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ready_to_ask(window, cx) {
            self.api.update(cx, |api, cx| api.ask_schema(cx));
        }
    }

    /// The GraphQL query indented (#70), and its variables, as the JSON
    /// body's Format does. A query not complete stays as it is, and says
    /// why.
    fn format_graphql(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.graphql.query.read(cx).value().to_string();
        match cyberapi_core::graphql_format::format_query(&query) {
            Ok(formatted) if formatted != query => {
                self.graphql.query.update(cx, |editor, cx| {
                    // #75: the caret stays by the same word, not at the start.
                    let caret = cyberapi_core::graphql_format::caret_after_format(&query, editor.cursor(), &formatted);
                    editor.set_value(formatted, window, cx);
                    let position = editor.text().offset_to_position(caret);
                    editor.set_cursor_position(position, window, cx);
                });
            }
            Ok(_) => {}
            Err(why) => {
                use cyberapi_core::graphql_format::Unformatted;
                let reason = match why {
                    Unformatted::StringNotClosed => t!("editor.graphql_string_not_closed"),
                    Unformatted::Brackets => t!("editor.graphql_brackets"),
                    Unformatted::NotGraphQl(ch) => t!("editor.graphql_not_graphql", ch = ch),
                };
                let message = t!("editor.graphql_query_incomplete", reason = reason).to_string();
                window.push_notification(Notification::warning(message), cx);
            }
        }
        let text = self.graphql.variables.read(cx).value().to_string();
        if !text.trim().is_empty() {
            match send::format_json(&text) {
                Ok(pretty) => {
                    self.graphql
                        .variables
                        .update(cx, |editor, cx| editor.set_value(pretty, window, cx));
                }
                Err(problem) => {
                    let message = t!("editor.graphql_variables_broken", reason = problem).to_string();
                    window.push_notification(Notification::error(message), cx);
                }
            }
        }
        self.save_graphql(cx);
    }

    /// E3: subscribe over WebSocket. A page's bridge relays none, as for a
    /// WebSocket API. Once it is running the button is the stream's Stop.
    fn graphql_subscribe_button(&self, cx: &mut Context<Self>) -> Button {
        Button::new("request-send")
            .primary()
            .small()
            .icon(gpui_kit::assets::IconName::Plug)
            .label(t!("editor.graphql_subscribe").to_string())
            .disabled(!this_build::opens_sockets())
            .tooltip(match this_build::opens_sockets() {
                true => t!("editor.graphql_subscribe_about").to_string(),
                false => t!("editor.graphql_ws_desktop_only").to_string(),
            })
            .on_click(cx.listener(|this, _, window, cx| this.send(window, cx)))
    }

    /// E2: a client or bidirectional call's button: connect, give up
    /// connecting, or close it. A page's bridge relays none.
    fn grpc_call_button(&self, sending: Option<web_time::Instant>, cx: &mut Context<Self>) -> Button {
        let open = self.api.read(cx).call_open();
        match sending {
            Some(started) => Button::new("request-send")
                .danger()
                .small()
                .label(format!(
                    "{} {:.1}s",
                    if open {
                        t!("editor.ws_disconnect")
                    } else {
                        t!("editor.cancel_send")
                    },
                    started.elapsed().as_secs_f32()
                ))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.api
                        .update(cx, |api, cx| api.send_or_cancel(Answers::default(), cx));
                })),
            None => Button::new("request-send")
                .primary()
                .small()
                .icon(gpui_kit::assets::IconName::Plug)
                .label(t!("editor.ws_connect").to_string())
                .disabled(!this_build::streams_bodies())
                .tooltip(match this_build::streams_bodies() {
                    true => t!("editor.grpc_connect_about").to_string(),
                    false => t!("editor.grpc_stream_desktop_only").to_string(),
                })
                .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
        }
    }

    /// E1: a WebSocket's button: connect, give up connecting, or close it.
    fn socket_button(&self, sending: Option<web_time::Instant>, cx: &mut Context<Self>) -> Button {
        let open = self.api.read(cx).socket_open();
        match sending {
            Some(started) => Button::new("request-send")
                .danger()
                .small()
                .label(format!(
                    "{} {:.1}s",
                    if open {
                        t!("editor.ws_disconnect")
                    } else {
                        t!("editor.cancel_send")
                    },
                    started.elapsed().as_secs_f32()
                ))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.api
                        .update(cx, |api, cx| api.send_or_cancel(Answers::default(), cx));
                })),
            None => Button::new("request-send")
                .primary()
                .small()
                .icon(gpui_kit::assets::IconName::Plug)
                .label(t!("editor.ws_connect").to_string())
                // A page's bridge relays no WebSocket.
                .disabled(!this_build::opens_sockets())
                .tooltip(match this_build::opens_sockets() {
                    true => t!("editor.ws_connect_about").to_string(),
                    false => t!("editor.ws_desktop_only").to_string(),
                })
                .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
        }
    }

    /// E1: a WebSocket's message: the text the next send sends, its
    /// templates rendered then, and the button that sends it once open.
    fn render_message(&self, cx: &mut Context<Self>) -> AnyElement {
        let request = self.api.read(cx).request().clone();
        let open = self.api.read(cx).socket_open();
        let muted = cx.theme().muted_foreground;
        let toolbar = h_flex()
            .gap_2()
            .px_2()
            .py_1()
            .items_center()
            .child(
                Button::new("ws-send-message")
                    .primary()
                    .xsmall()
                    .icon(gpui_kit::assets::IconName::Send)
                    .label(t!("editor.ws_send_message").to_string())
                    .disabled(!open)
                    .tooltip_with_action(t!("editor.ws_send_message").to_string(), &SendAction, Some(KEY_CONTEXT))
                    .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
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
            })
            .when(!open, |this| {
                this.child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .text_color(muted)
                        .child(t!("editor.ws_message_hint").to_string()),
                )
            });
        v_flex()
            .flex_1()
            .min_h_0()
            .child(toolbar)
            .child(
                div().flex_1().min_h_0().relative().child(
                    div()
                        .absolute()
                        .inset_0()
                        .child(Editor::new(&self.body).w_full().h_full().bordered(false)),
                ),
            )
            .into_any_element()
    }

    fn render_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let request = self.api.read(cx).request().clone();
        if request.method == HttpMethod::Grpc {
            return self.render_grpc(cx);
        }
        if request.method == HttpMethod::WebSocket {
            return self.render_message(cx);
        }
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
                Button::new("body-format-graphql")
                    .ghost()
                    .xsmall()
                    .icon(gpui_kit::assets::IconName::WandSparkles)
                    .label(t!("editor.format").to_string())
                    .tooltip(t!("editor.format_graphql_about").to_string())
                    .on_click(cx.listener(|this, _, window, cx| this.format_graphql(window, cx))),
            )
            .child(
                Button::new("body-graphql-schema")
                    .ghost()
                    .xsmall()
                    .icon(gpui_kit::assets::IconName::ListTree)
                    .label(label)
                    .loading(asking)
                    .tooltip(match schema {
                        Some(_) => t!("editor.graphql_schema_browse").to_string(),
                        None => t!("editor.graphql_schema_about").to_string(),
                    })
                    // #70: a schema here is browsed, and fetched again from
                    // there; none yet is asked for.
                    .on_click(cx.listener(|this, _, window, cx| match this.api.read(cx).schema(cx) {
                        Some(schema) => {
                            let editor = cx.entity().downgrade();
                            let fetch = move |window: &mut Window, cx: &mut App| {
                                let _ = editor.update(cx, |this, cx| this.fetch_schema(window, cx));
                            };
                            let query = this.graphql.query.clone();
                            super::schema_browser::open_schema_browser(schema, query, fetch, window, cx);
                        }
                        None => this.fetch_schema(window, cx),
                    })),
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

    /// F1: a scratch request, and — for someone who may edit the project —
    /// the way to put it in the tree.
    fn scratch_note(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let editable = self.workspace.read(cx).editable();
        let id = self.api.read(cx).api().clone();
        let name = self
            .workspace
            .read(cx)
            .scratch(&id)
            .map(|scratch| scratch.name.clone())
            .unwrap_or_default();
        let weak = self.workspace.downgrade();
        let theme = cx.theme();
        h_flex()
            .flex_none()
            .h(px(28.))
            .px_3()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .child(t!("workspace.scratch_hint").to_string()),
            )
            .when(editable, |row| {
                row.child(
                    Button::new("save-scratch")
                        .ghost()
                        .xsmall()
                        .flex_none()
                        .label(t!("workspace.save_scratch").to_string())
                        .on_click(move |_, window, cx| {
                            super::move_dialog::open_save_scratch_dialog(
                                weak.clone(),
                                id.clone(),
                                name.clone(),
                                window,
                                cx,
                            );
                        }),
                )
            })
    }
}

impl Render for RequestEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = self.workspace.read(cx).tab(&self.api, cx);
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
            RequestTab::Settings => self.settings.clone().into_any_element(),
            RequestTab::Assertions => self.assertions.clone().into_any_element(),
            RequestTab::Docs => self.docs.clone().into_any_element(),
            RequestTab::Auth => div()
                .id("request-auth")
                .size_full()
                .overflow_y_scrollbar()
                .px_3()
                .py_2()
                .child(self.auth.clone())
                .into_any_element(),
        };
        let scratch = self.api.read(cx).is_scratch();
        v_flex()
            .size_full()
            .child(self.render_bar(cx))
            .when(scratch, |column| column.child(self.scratch_note(cx)))
            .child(self.render_tabs(tab, cx))
            .child(v_flex().flex_1().min_h_0().child(content))
    }
}
