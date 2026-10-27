//! The selected API (M4, M5, M7): the request as the editor holds it, its
//! saving, sending it, and its responses.
//!
//! One per selected API, made by the workspace, so an edit can only ever be
//! saved to the API it was made on — the old app checked "the API the edit
//! was made on is still the selected one" before each save (F4.11); here the
//! state *is* the API's. Method and URI save at once (F4.2 / F4.3), the rest
//! 300 ms after the last change; whatever is still unsaved when the state goes
//! (another API selected, the project left) is saved then.
//!
//! Sending (F5.1): one request at a time; sending again while one is out
//! cancels it, and a cancelled request is dropped — its connection closes
//! (#2) — rather than left to finish unseen. A second click within 200 ms of
//! the first is taken as a double click and ignored.
//!
//! An answer that is an event stream (`cyberapi_core::sse`) shows as it
//! comes ([`LiveStream`]); sending again then stops it, and what came is
//! the answer, kept like any other.
//!
//! A gRPC call (`cyberapi_core::grpc`) is encoded with definitions: from the
//! server's reflection, or `.proto` files on the desktop. The project keeps
//! the ones loaded for an address, for every API sent there; the API keeps
//! the files its chosen service needs.
//!
//! What was typed for a request's `prompt`s (D1) is kept while the app runs,
//! by project, never saved: the next send's dialog starts with it, and the
//! request's code, schema and reflection take it.

use super::{Load, WorkspaceState, run_serial, run_store};
use crate::this_build;
use cyberapi_core::assertions::{self, Assertion, Extraction, Outcomes, Problem, Subject};
use cyberapi_core::auth::Auth;
use cyberapi_core::code::CodeTarget;
use cyberapi_core::graphql_schema::Schema;
use cyberapi_core::grpc::{self, Definitions, FROM_REFLECTION, GrpcBody, ReplyReader};
use cyberapi_core::model::{ApiSetting, ContentType, Example, HttpMethod, HttpRequest, KvParam};
use cyberapi_core::prefs::RequestSettings;
use cyberapi_core::response::{HttpResponse, StreamEnd};
use cyberapi_core::send::PrepareError;
use cyberapi_core::sse;
use cyberapi_core::template::{Answers, TemplateError};
use cyberapi_core::ws::{self, Frame};
use cyberapi_core::{CollectionId, ItemId};
use cyberapi_http::{CallIn, CallOut, Progress, SocketIn, SocketOut, Watch, stop_signal};
use cyberapi_service::graphql_ws::{self, Heard};
use cyberapi_service::grpc_stream;
use cyberapi_service::socket::{self, Connected, LiveSocket};
use cyberapi_service::{SendFailure, Sender};
use cyberapi_store::StoreError;
use futures::StreamExt as _;
use futures::channel::{mpsc, oneshot};
use gpui_kit::{App, AppContext as _, Context, EventEmitter, Global, Task, WeakEntity};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;
use web_time::Instant;

/// How long typing waits before it is saved (F4.11).
pub const SAVE_DELAY: Duration = Duration::from_millis(300);
/// A send this soon after the last one is the same click (F5.1).
const DOUBLE_CLICK: Duration = Duration::from_millis(200);

/// D1: what was typed for each `prompt`, by project, while the app runs.
#[derive(Default)]
struct Typed(HashMap<CollectionId, HashMap<String, String>>);

impl Global for Typed {}

/// Keeps what was typed for `collection`'s `prompt`s, for the next time
/// one asks (D1).
pub fn remember_typed(collection: &CollectionId, prompts: &HashMap<String, String>, cx: &mut App) {
    if prompts.is_empty() {
        return;
    }
    cx.default_global::<Typed>()
        .0
        .entry(collection.clone())
        .or_default()
        .extend(prompts.clone());
}

/// What was typed for the `prompt`s of `collection`'s requests while the app
/// runs, by label.
pub fn typed_prompts(collection: &CollectionId, cx: &App) -> HashMap<String, String> {
    cx.try_global::<Typed>()
        .and_then(|typed| typed.0.get(collection))
        .cloned()
        .unwrap_or_default()
}

/// Why nothing was sent, as the view hears it.
fn not_sent(failure: SendFailure) -> ApiEvent {
    match failure {
        SendFailure::Prepare {
            source: PrepareError::Environment { name, within },
        } => ApiEvent::NoEnvironment { name, within },
        SendFailure::Prepare {
            source: PrepareError::GraphQlVariables { message },
        } => ApiEvent::GraphQlVariables(message),
        SendFailure::Prepare {
            source: PrepareError::Template {
                source: TemplateError::EnvUnset { name },
            },
        } => ApiEvent::EnvUnset(name),
        other => ApiEvent::NotSent(other.to_string()),
    }
}

#[derive(Clone, Debug)]
pub enum ApiEvent {
    /// A change could not be saved, or the history could not be read.
    Failed(StoreError),
    /// Nothing was sent: the request could not be prepared (F5.7).
    NotSent(String),
    /// Nothing was sent: the URI starts with an environment variable no
    /// enabled one is named as (#50). Its name, and the named environment
    /// it was sent in (#61), for the view to say.
    NoEnvironment { name: String, within: Option<String> },
    /// Nothing was sent: `env(NAME)` (D2) names a variable this machine has
    /// not set.
    EnvUnset(String),
    /// Nothing was sent: a GraphQL body's variables are not a JSON object
    /// once their templates ran. Why, for the view to say.
    GraphQlVariables(String),
    /// The request was replaced from the history (F7.10): the editor's
    /// fields read it again.
    RequestReplaced,
    /// The code asked for (F7.8, #69), for the clipboard: what it is
    /// written as, and the code.
    Code(CodeTarget, String),
    /// A body was saved in the downloads folder, there (F7.5).
    Saved(String),
    /// A body could not be saved, or opened outside the app: why.
    NotSaved(String),
    /// A GraphQL endpoint gave no schema: why.
    NoSchema(String),
    /// gRPC definitions were loaded, with this many methods; the example
    /// message of the method a call without one took, for a message not
    /// written yet.
    GrpcDefinitions { methods: usize, example: Option<String> },
    /// No gRPC definitions: why.
    NoGrpcDefinitions(String),
}

/// gRPC definitions, and where they came from (`GrpcBody::source`).
pub struct GrpcDefinitions {
    pub definitions: Definitions,
    pub source: String,
}

/// A request on its way.
struct Sending {
    started: Instant,
    _task: Task<()>,
    /// What an event stream tells as it comes, taken in.
    _follow: Task<()>,
    /// Ends an event stream: what came is the answer.
    stop: Option<oneshot::Sender<()>>,
    /// Reads a gRPC call's streamed reply, when it makes one.
    replies: Option<ReplyReader>,
}

/// How a live stream's pieces become events.
enum Reader {
    Events(sse::Parser),
    Replies(ReplyReader),
    /// A WebSocket's messages, each an event as it goes or comes (E1).
    Socket,
}

/// E1: an open WebSocket: what the reader sends goes out on it, and what
/// went each way is kept, for the history once it ends.
struct OpenSocket {
    outgoing: mpsc::UnboundedSender<SocketOut>,
    messages: Vec<(bool, Frame, u64)>,
    /// When it opened: a message's time is after it.
    opened: Instant,
}

/// E2: an open gRPC call whose request streams. Messages go out on it one
/// at a time; what went each way is kept once it ends. Dropping `outgoing`
/// gives the call up, so it stays until the answer has been read.
struct OpenCall {
    outgoing: mpsc::UnboundedSender<CallOut>,
    /// `(sent, text, milliseconds after it opened)`.
    messages: Vec<(bool, String, u64)>,
    opened: Instant,
    /// The request is done (a client stream's "finish"). A bidirectional
    /// call never is, until it ends.
    finished: bool,
    bidi: bool,
    reader: Option<ReplyReader>,
    status: Option<u16>,
    headers: Vec<(String, String)>,
    trailers: Vec<(String, String)>,
    failed: Option<String>,
}

/// An event stream's answer, or a gRPC call's streamed reply, as it comes.
pub struct LiveStream {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub events: Vec<sse::Event>,
    reader: Reader,
    /// When the head came: an event's time is after it.
    head: Instant,
    /// The reader asked it to end; the answer is on its way.
    pub stopping: bool,
}

impl LiveStream {
    fn new(status: u16, headers: Vec<(String, String)>, replies: Option<ReplyReader>) -> Self {
        // An event stream says so; any other stream is the call's replies.
        let reader = match replies {
            Some(replies) if !sse::is_event_stream(&headers) => Reader::Replies(replies),
            _ => Reader::Events(sse::Parser::new()),
        };
        Self {
            status,
            headers,
            body: Vec::new(),
            events: Vec::new(),
            reader,
            head: Instant::now(),
            stopping: false,
        }
    }

    fn take(&mut self, piece: &[u8]) {
        self.body.extend_from_slice(piece);
        let at = Some(self.head.elapsed().as_millis() as u64);
        let events = match &mut self.reader {
            Reader::Events(parser) => parser.feed(piece, at),
            Reader::Replies(replies) => replies.feed(piece, at),
            Reader::Socket => Vec::new(),
        };
        self.events.extend(events);
    }

    /// E1: a WebSocket's, its messages added as they go and come.
    fn socket(status: u16, headers: Vec<(String, String)>) -> Self {
        Self {
            status,
            headers,
            body: Vec::new(),
            events: Vec::new(),
            reader: Reader::Socket,
            head: Instant::now(),
            stopping: false,
        }
    }

    /// E1: a message, `sent` or come, `at` milliseconds after it opened:
    /// a line of the body as it will be kept, and an event.
    fn message(&mut self, sent: bool, frame: &Frame, at: u64) {
        let (line, _) = ws::transcript(&[(sent, frame.clone(), at)]);
        self.body.extend(line);
        self.events.push(ws::event(sent, frame, at));
    }

    /// E3: a GraphQL subscription over a WebSocket, its results added as
    /// they come.
    fn graphql(status: u16, headers: Vec<(String, String)>) -> Self {
        Self {
            status,
            headers,
            body: Vec::new(),
            events: Vec::new(),
            reader: Reader::Socket,
            head: Instant::now(),
            stopping: false,
        }
    }

    /// E2: a gRPC call that streams from the client, before its headers.
    /// Status 0 is not an answer yet; the panel draws no status for it.
    fn call() -> Self {
        Self::socket(0, Vec::new())
    }

    /// E2: one turn, sent or received, as the history will keep it.
    fn grpc_turn(&mut self, sent: bool, text: &str, at: u64) {
        let (line, _) = grpc::transcript(&[(sent, text.to_string(), at)]);
        self.body.extend(line);
        self.events.push(grpc::turn_event(sent, text, Some(at)));
    }

    /// E3: one result. `at` is milliseconds after the connection opened.
    fn graphql_event(&mut self, heard: &Heard) {
        self.body
            .extend(cyberapi_core::graphql_ws::line(&heard.kind, &heard.data));
        self.events.push(sse::Event {
            event: heard.kind.clone(),
            data: heard.data.clone(),
            at: Some(heard.at),
            ..Default::default()
        });
    }
}

/// How long the live stream waits between redraws: pieces that come
/// faster are taken in together.
const LIVE_FRAME: Duration = Duration::from_millis(50);

pub struct ApiState {
    sender: Sender,
    workspace: WeakEntity<WorkspaceState>,
    collection: CollectionId,
    api: ItemId,
    /// A viewer's edits stay here: they send, and are never saved (§9.2).
    editable: bool,
    /// F1: a scratch request. Its edits are the reader's, in the workspace
    /// prefs, even a viewer's. Closing the tab drops them.
    scratch: bool,
    /// The workspace's pref saves, so a scratch written as the editor goes
    /// lands after the one already on its way.
    pref_saves: Arc<async_lock::Mutex<()>>,
    request: HttpRequest,
    /// What the store has, as far as this state knows; `None` after a save
    /// failed, so the next one goes out whatever it holds.
    saved: Option<HttpRequest>,
    save_task: Option<Task<()>>,
    /// The saves, one at a time in the order made: each writes the whole
    /// request, and one that landed after a later one would undo it.
    saves: Arc<async_lock::Mutex<()>>,
    sending: Option<Sending>,
    /// The event stream being answered, while it comes.
    live: Option<LiveStream>,
    /// E1: the WebSocket open, while it is.
    socket: Option<OpenSocket>,
    /// E2: the gRPC call open, while it is.
    call: Option<OpenCall>,
    last_send: Option<Instant>,
    /// Newest first (F7.9). Shared, so the panel takes one for each frame
    /// without copying its body.
    responses: Vec<Arc<HttpResponse>>,
    history: Load,
    /// The response the panel shows: the newest unless one was chosen.
    shown: Option<String>,
    /// D4: one asked for from the project's recent list before the history
    /// was read: chosen once it is.
    wanted: Option<String>,
    /// A GraphQL endpoint's schema being asked for.
    asking_schema: Option<Task<()>>,
    /// gRPC definitions being loaded.
    loading_grpc: Option<Task<()>>,
    /// The gRPC definitions the body keeps, read once for the descriptors
    /// it keeps: what the view draws asks for them every frame.
    own_grpc: RefCell<Option<(String, Option<Arc<GrpcDefinitions>>)>>,
    /// A1: the shown response's body read as JSON, once for each response:
    /// every keystroke in an assertion checks it again.
    checked_json: RefCell<Option<(String, Option<Arc<serde_json::Value>>)>>,
    /// A1: the last outcomes; the editor and the panel ask for them every
    /// frame.
    checks: RefCell<Option<Checks>>,
}

/// A1: outcomes, with the response and the assertions they came of.
type Checks = (String, Vec<Assertion>, Rc<Outcomes>);

impl EventEmitter<ApiEvent> for ApiState {}

impl ApiState {
    pub fn new(
        sender: Sender,
        workspace: WeakEntity<WorkspaceState>,
        api: &ApiSetting,
        editable: bool,
        scratch: bool,
        pref_saves: Arc<async_lock::Mutex<()>>,
        cx: &mut Context<Self>,
    ) -> Self {
        // An edit not saved yet is saved as the state goes.
        cx.on_release(|this, cx| {
            if this.scratch {
                // Closing the tab marks it saved and drops it. What is left
                // unsaved — the project was left — is written after the
                // prefs save already on its way, and only while the scratch
                // is still there.
                if this.saved.as_ref() == Some(&this.request) {
                    return;
                }
                let (saves, storage, collection) = (
                    this.pref_saves.clone(),
                    this.sender.storage.clone(),
                    this.collection.clone(),
                );
                let (id, request) = (this.api.clone(), this.request.clone());
                cx.background_spawn(async move {
                    let _turn = saves.lock().await;
                    let Ok(mut prefs) = storage.prefs().workspace(collection.clone()).await else {
                        return;
                    };
                    let Some(scratch) = prefs.scratches.iter_mut().find(|scratch| scratch.id == id) else {
                        return;
                    };
                    scratch.request = request;
                    if let Err(error) = storage.prefs().set_workspace(collection, prefs).await {
                        warn!(%error, "the scratch request could not be saved");
                    }
                })
                .detach();
                return;
            }
            if this.editable && this.saved.as_ref() != Some(&this.request) {
                let call = this.sender.storage.tree().save_request(
                    this.collection.clone(),
                    this.api.clone(),
                    this.request.clone(),
                );
                cx.background_spawn(async move {
                    if let Err(error) = call.await {
                        warn!(%error, "the request could not be saved");
                    }
                })
                .detach();
            }
        })
        .detach();
        let mut state = Self {
            sender,
            workspace,
            collection: api.collection.clone(),
            api: api.id.clone(),
            editable,
            scratch,
            pref_saves,
            request: api.request.clone(),
            saved: Some(api.request.clone()),
            save_task: None,
            saves: Arc::default(),
            sending: None,
            live: None,
            socket: None,
            call: None,
            last_send: None,
            responses: Vec::new(),
            history: Load::Loading,
            shown: None,
            wanted: None,
            asking_schema: None,
            loading_grpc: None,
            own_grpc: RefCell::new(None),
            checked_json: RefCell::new(None),
            checks: RefCell::new(None),
        };
        state.load_history(cx);
        state
    }

    pub fn api(&self) -> &ItemId {
        &self.api
    }

    /// F1: this editor holds a scratch request, not an API in the tree.
    pub fn is_scratch(&self) -> bool {
        self.scratch
    }

    /// Whether the reader may change the project's requests: a viewer's
    /// edits stay their own (§9.2).
    pub fn editable(&self) -> bool {
        self.editable
    }

    /// The project it is of, for what lists the project's (D4).
    pub fn workspace(&self) -> &WeakEntity<WorkspaceState> {
        &self.workspace
    }

    /// Whether the editor holds a change the store has not had.
    pub fn has_unsaved(&self) -> bool {
        self.editable && self.saved.as_ref() != Some(&self.request)
    }

    /// Drops the change not saved yet, so the state goes without saving it:
    /// what it would be saved into is being replaced (`docs/server-sync.md`
    /// §3.3, the server's over the reader's).
    pub fn forget_unsaved(&mut self) {
        self.save_task = None;
        self.saved = Some(self.request.clone());
    }

    pub fn request(&self) -> &HttpRequest {
        &self.request
    }

    pub fn responses(&self) -> &[Arc<HttpResponse>] {
        &self.responses
    }

    pub fn history(&self) -> &Load {
        &self.history
    }

    /// F7.12: the newest response, or the one chosen from the history.
    pub fn shown_response(&self) -> Option<&Arc<HttpResponse>> {
        match &self.shown {
            Some(id) => self.responses.iter().find(|response| &response.id == id),
            None => self.responses.first(),
        }
    }

    /// When the request on its way was sent, if one is.
    pub fn sending_since(&self) -> Option<Instant> {
        self.sending.as_ref().map(|sending| sending.started)
    }

    /// The history read again: answers the page kept until a sync are in
    /// it now (#52), or a run sent the API (A3).
    pub fn reload_history(&mut self, cx: &mut Context<Self>) {
        self.load_history(cx);
    }

    /// Whether this API is kept local, out of the sync (server-sync §3.4).
    pub fn local_only(&self, cx: &App) -> bool {
        self.workspace
            .upgrade()
            .is_some_and(|workspace| workspace.read(cx).local_members().contains(&self.api))
    }

    fn load_history(&mut self, cx: &mut Context<Self>) {
        let call = self
            .sender
            .storage
            .history()
            .list(self.collection.clone(), self.api.clone());
        run_store(cx, call, |this, result, cx| {
            match result {
                Ok(responses) => {
                    this.responses = responses.into_iter().map(Arc::new).collect();
                    this.history = Load::Loaded;
                    if let Some(id) = this.wanted.take() {
                        this.choose_response(&id, cx);
                    }
                    // One chosen that is gone (kept again under a new id)
                    // gives way to the newest.
                    if this
                        .shown
                        .as_ref()
                        .is_some_and(|id| !this.responses.iter().any(|response| &response.id == id))
                    {
                        this.shown = None;
                    }
                }
                Err(error) => {
                    this.history = Load::Failed(error.clone());
                    cx.emit(ApiEvent::Failed(error));
                }
            }
            cx.notify();
        });
    }

    /// F4.2: saved at once. To or from gRPC, the body becomes the call's
    /// message, or the message the body; the editor reads it again.
    pub fn set_method(&mut self, method: HttpMethod, cx: &mut Context<Self>) {
        if self.request.method != method {
            let body = grpc::switched_body(&self.request.body, self.request.method, method);
            let mut replaced = body != self.request.body;
            self.request.body = body;
            self.request.method = method;
            // E1: a WebSocket's message is text; a form or a GraphQL body
            // reads as the text it is kept as.
            if method == HttpMethod::WebSocket
                && (self.request.content_type.is_key_value() || self.request.content_type == ContentType::GraphQl)
            {
                self.request.content_type = ContentType::Json;
                replaced = true;
            }
            self.save_now(cx);
            let api = self.api.clone();
            let _ = self
                .workspace
                .update(cx, |workspace, cx| workspace.method_changed(&api, cx));
            if replaced {
                cx.emit(ApiEvent::RequestReplaced);
            }
        }
    }

    /// F4.3: called as the URI field is left; saved at once, trimmed.
    pub fn set_uri(&mut self, uri: String, cx: &mut Context<Self>) {
        let uri = uri.trim().to_string();
        if self.request.uri != uri {
            self.request.uri = uri;
            self.save_now(cx);
        }
    }

    /// F4.6: a new type starts an empty body (the view asks first when the
    /// body is not empty).
    pub fn set_content_type(&mut self, content_type: ContentType, cx: &mut Context<Self>) {
        if self.request.content_type != content_type {
            self.request.content_type = content_type;
            self.request.body.clear();
            self.save_now(cx);
        }
    }

    /// F4.7: text bodies are kept trimmed; form bodies as their list's JSON.
    pub fn set_body(&mut self, body: String, cx: &mut Context<Self>) {
        let body = if self.request.content_type.is_key_value() {
            body
        } else {
            body.trim().to_string()
        };
        if self.request.body != body {
            self.request.body = body;
            self.save_later(cx);
        }
    }

    /// The whole request at once (a cURL command pasted as the URI), saved
    /// at once; the editor reads it back.
    pub fn replace_request(&mut self, request: HttpRequest, cx: &mut Context<Self>) {
        if self.request == request {
            return;
        }
        let method_changed = self.request.method != request.method;
        self.request = request;
        self.save_now(cx);
        if method_changed {
            let api = self.api.clone();
            let _ = self
                .workspace
                .update(cx, |workspace, cx| workspace.method_changed(&api, cx));
        }
        cx.emit(ApiEvent::RequestReplaced);
    }

    /// F4.8: a form or multipart body's fields. A gRPC call's body is its
    /// own: the hidden list writes nothing there.
    pub fn set_form(&mut self, fields: Vec<KvParam>, cx: &mut Context<Self>) {
        if self.request.method == HttpMethod::Grpc {
            return;
        }
        self.set_body(serde_json::to_string(&kept(fields)).unwrap_or_default(), cx);
    }

    pub fn set_query(&mut self, query: Vec<KvParam>, cx: &mut Context<Self>) {
        let query = kept(query);
        if self.request.query != query {
            self.request.query = query;
            self.save_later(cx);
        }
    }

    pub fn set_headers(&mut self, headers: Vec<KvParam>, cx: &mut Context<Self>) {
        let headers = kept(headers);
        if self.request.headers != headers {
            self.request.headers = headers;
            self.save_later(cx);
        }
    }

    /// #66: how the API signs in.
    pub fn set_auth(&mut self, auth: Auth, cx: &mut Context<Self>) {
        if self.request.auth != auth {
            self.request.auth = auth;
            self.save_later(cx);
        }
    }

    /// A1: what the API's answers are checked against.
    pub fn set_assertions(&mut self, assertions: Vec<Assertion>, cx: &mut Context<Self>) {
        if self.request.assertions != assertions {
            self.request.assertions = assertions;
            self.save_later(cx);
        }
    }

    /// A1: how the shown response holds up to the API's assertions as they
    /// are now. `None` with no response, while one streams in, or with no
    /// assertion on.
    pub fn checks(&self) -> Option<Rc<Outcomes>> {
        if self.live.is_some() {
            return None;
        }
        let response = self.shown_response()?;
        let assertions = &self.request.assertions;
        if !assertions.iter().any(|assertion| assertion.enabled) {
            return None;
        }
        if let Some((id, of, outcomes)) = &*self.checks.borrow()
            && id == &response.id
            && of == assertions
        {
            return Some(outcomes.clone());
        }
        let json = if assertions::reads_json(assertions) {
            self.json_of(response)
        } else {
            None
        };
        let outcomes = Rc::new(assertions::check_all(assertions, response, json.as_deref()));
        *self.checks.borrow_mut() = Some((response.id.clone(), assertions.clone(), outcomes.clone()));
        Some(outcomes)
    }

    /// F3: what the API is for, as the editor has it.
    pub fn set_description(&mut self, description: String, cx: &mut Context<Self>) {
        if self.request.description != description {
            self.request.description = description;
            self.save_later(cx);
        }
    }

    /// F3: the response shown kept as an example named `name`; `false` when
    /// it cannot be one — no status, no text, or too long.
    pub fn keep_example(&mut self, name: &str, cx: &mut Context<Self>) -> bool {
        let Some(example) = self.shown_response().and_then(|shown| Example::of(shown, name)) else {
            return false;
        };
        self.request.examples.push(example);
        self.save_later(cx);
        true
    }

    /// F3: the example at `at` no longer kept.
    pub fn remove_example(&mut self, at: usize, cx: &mut Context<Self>) {
        if at < self.request.examples.len() {
            self.request.examples.remove(at);
            self.save_later(cx);
        }
    }

    /// A2: the API's extractions, as the editor has them.
    pub fn set_extractions(&mut self, extractions: Vec<Extraction>, cx: &mut Context<Self>) {
        if self.request.extractions != extractions {
            self.request.extractions = extractions;
            self.save_later(cx);
        }
    }

    /// A2: what `extraction` would take from the shown response, for the
    /// editor to show; `None` with no response, or while one streams in.
    pub fn taken(&self, extraction: &Extraction) -> Option<Result<Option<String>, Problem>> {
        if self.live.is_some() {
            return None;
        }
        let response = self.shown_response()?;
        let json = if extraction.subject == Subject::Json {
            self.json_of(response)
        } else {
            None
        };
        Some(assertions::extract(extraction, response, json.as_deref()))
    }

    fn json_of(&self, response: &HttpResponse) -> Option<Arc<serde_json::Value>> {
        if let Some((id, json)) = &*self.checked_json.borrow()
            && id == &response.id
        {
            return json.clone();
        }
        let json = response.body_json().map(Arc::new);
        *self.checked_json.borrow_mut() = Some((response.id.clone(), json.clone()));
        json
    }

    /// #71: the API's own sending settings.
    pub fn set_settings(&mut self, settings: RequestSettings, cx: &mut Context<Self>) {
        if self.request.settings != settings {
            self.request.settings = settings;
            self.save_later(cx);
        }
    }

    fn save_later(&mut self, cx: &mut Context<Self>) {
        cx.notify();
        self.save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            let _ = this.update(cx, |this, cx| this.save_now(cx));
        }));
    }

    /// Saves what the editor holds, unless the store has it already.
    /// A scratch request is kept in the reader's prefs, a viewer's too.
    pub fn save_now(&mut self, cx: &mut Context<Self>) {
        self.save_task = None;
        cx.notify();
        if self.scratch {
            if self.saved.as_ref() == Some(&self.request) {
                return;
            }
            let request = self.request.clone();
            self.saved = Some(request.clone());
            let id = self.api.clone();
            let workspace = self.workspace.clone();
            // Leaving a tab, and a push, call this while the workspace is
            // already being updated. The scratch is written once that returns.
            cx.defer(move |cx| {
                let _ = workspace.update(cx, |workspace, cx| workspace.keep_scratch(&id, request, cx));
            });
            return;
        }
        if !self.editable || self.saved.as_ref() == Some(&self.request) {
            return;
        }
        let request = self.request.clone();
        let call = self
            .sender
            .storage
            .tree()
            .save_request(self.collection.clone(), self.api.clone(), request.clone());
        // Marked saved now, so a change while the call is out saves again.
        self.saved = Some(request.clone());
        run_serial(cx, &self.saves, call, move |this, result, cx| match result {
            Ok(setting) => {
                let _ = this
                    .workspace
                    .update(cx, |workspace, cx| workspace.api_saved(setting, cx));
            }
            Err(error) => {
                // Not saved after all: the next save goes out — unless the
                // editor has moved on to a request of the history meanwhile.
                if this.saved.as_ref() == Some(&request) {
                    this.saved = None;
                }
                cx.emit(ApiEvent::Failed(error));
            }
        });
    }

    /// F5.1: sends, or cancels the request on its way. `answers` are the
    /// reader's to the `openFile` and the `prompt`s the request needs; what
    /// was typed is kept for the next time.
    pub fn send_or_cancel(&mut self, answers: Answers, cx: &mut Context<Self>) {
        if self.last_send.is_some_and(|last| last.elapsed() < DOUBLE_CLICK) {
            return;
        }
        self.last_send = Some(Instant::now());
        // E1: an open WebSocket is closed — the server is told, and what
        // went each way is kept once it answers.
        if let Some(socket) = &self.socket {
            let _ = socket.outgoing.unbounded_send(SocketOut::Close);
            if let Some(live) = self.live.as_mut() {
                live.stopping = true;
            }
            cx.notify();
            return;
        }
        // E2: an open call is given up. This is before the generic branch:
        // the call's task has no stop signal, so that branch would do nothing.
        if self.call.is_some() {
            if let Some(call) = self.call.as_mut() {
                let _ = call.outgoing.unbounded_send(CallOut::Cancel);
            }
            if let Some(live) = self.live.as_mut() {
                live.stopping = true;
            }
            cx.notify();
            return;
        }
        if let Some(sending) = self.sending.as_mut() {
            // An event stream that has begun ends, and is kept; any other
            // request is dropped.
            match self.live.as_mut() {
                Some(live) => {
                    if let Some(stop) = sending.stop.take() {
                        let _ = stop.send(());
                        live.stopping = true;
                    }
                }
                None => self.sending = None,
            }
            cx.notify();
            return;
        }
        self.save_now(cx);
        remember_typed(&self.collection, &answers.prompts, cx);
        if self.request.method == HttpMethod::WebSocket {
            return self.connect(answers, cx);
        }
        // E2: a method that streams from the client is connected to. A page
        // reaches the engine's refusal and keeps it, as a WebSocket does.
        if self.request.method == HttpMethod::Grpc && GrpcBody::read(&self.request.body).client_streams() {
            return self.connect_grpc(answers, cx);
        }
        // E3: a GraphQL API whose address is (or may be) a WebSocket
        // subscribes instead of sending. An address that renders as HTTP
        // still sends.
        if self.request.content_type == ContentType::GraphQl
            && cyberapi_core::graphql_ws::might_subscribe(&self.request.uri)
        {
            return self.subscribe_graphql(answers, cx);
        }
        let (told, mut telling) = mpsc::unbounded::<Progress>();
        let (stop, stopped) = stop_signal();
        let watch = Watch {
            progress: Arc::new(move |progress| {
                let _ = told.unbounded_send(progress);
            }),
            stop: stopped,
        };
        let replies = (self.request.method == HttpMethod::Grpc)
            .then(|| GrpcBody::read(&self.request.body).reply_reader())
            .flatten();
        let call = cyberapi_service::send_watched(
            self.sender(cx),
            self.collection.clone(),
            self.api.clone(),
            self.request.clone(),
            answers,
            Some(watch),
        );
        let task = cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(call).await;
            let _ = this.update(cx, |this, cx| this.answered(result, cx));
        });
        // What comes in a frame's time is taken in together.
        let follow = cx.spawn(async move |this, cx| {
            while let Some(first) = telling.next().await {
                let mut told = vec![first];
                while let Ok(more) = telling.try_recv() {
                    told.push(more);
                }
                if this.update(cx, |this, cx| this.told(told, cx)).is_err() {
                    return;
                }
                cx.background_executor().timer(LIVE_FRAME).await;
            }
        });
        self.sending = Some(Sending {
            started: Instant::now(),
            _task: task,
            _follow: follow,
            stop: Some(stop),
            replies,
        });
        cx.notify();
    }

    /// The event stream being answered, while it comes.
    pub fn live(&self) -> Option<&LiveStream> {
        self.live.as_ref()
    }

    /// E3: subscribes when the rendered address is a WebSocket, and sends
    /// otherwise. Results show as they come; stopping sends `complete`.
    fn subscribe_graphql(&mut self, answers: Answers, cx: &mut Context<Self>) {
        let (told, mut telling) = mpsc::unbounded::<Progress>();
        let (stop, stopped) = stop_signal();
        let watch = Watch {
            progress: Arc::new(move |progress| {
                let _ = told.unbounded_send(progress);
            }),
            stop: stopped,
        };
        let sender = self.sender(cx);
        let collection = self.collection.clone();
        let api = self.api.clone();
        let request = self.request.clone();
        let call = graphql_ws::subscribe(
            sender.clone(),
            collection.clone(),
            api.clone(),
            request.clone(),
            answers.clone(),
        );
        let task = cx.spawn(async move |this, cx| match cx.background_spawn(call).await {
            Ok(graphql_ws::Started::Http) => {
                let result = cx
                    .background_spawn(cyberapi_service::send_watched(
                        sender,
                        collection,
                        api,
                        request,
                        answers,
                        Some(watch),
                    ))
                    .await;
                let _ = this.update(cx, |this, cx| this.answered(result, cx));
            }
            Ok(graphql_ws::Started::Kept(response)) => {
                let _ = this.update(cx, |this, cx| this.answered(Ok(response), cx));
            }
            Err(failure) => {
                let _ = this.update(cx, |this, cx| this.answered(Err(failure), cx));
            }
            Ok(graphql_ws::Started::Open(subscription)) => {
                drop(watch);
                let status = subscription.status;
                let headers = subscription.headers.clone();
                let (events_tx, mut heard) = mpsc::unbounded();
                let (stop_tx, stop_rx) = oneshot::channel();
                // `cx` here is the task's, whose spawn takes the app alone.
                let listening = this.clone();
                let follow = cx.spawn(async move |cx| {
                    while let Some(event) = heard.next().await {
                        if listening
                            .update(cx, |this, cx| {
                                if let Some(live) = this.live.as_mut() {
                                    live.graphql_event(&event);
                                }
                                cx.notify();
                            })
                            .is_err()
                        {
                            return;
                        }
                    }
                });
                if this
                    .update(cx, |this, cx| {
                        this.live = Some(LiveStream::graphql(status, headers));
                        if let Some(sending) = this.sending.as_mut() {
                            sending._follow = follow;
                            sending.stop = Some(stop_tx);
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }
                let response = cx
                    .background_spawn(graphql_ws::drive(sender, subscription, events_tx, stop_rx))
                    .await;
                let _ = this.update(cx, |this, cx| this.answered(Ok(response), cx));
            }
        });
        let follow = cx.spawn(async move |this, cx| {
            while let Some(first) = telling.next().await {
                let mut batch = vec![first];
                while let Ok(more) = telling.try_recv() {
                    batch.push(more);
                }
                if this.update(cx, |this, cx| this.told(batch, cx)).is_err() {
                    return;
                }
                cx.background_executor().timer(LIVE_FRAME).await;
            }
        });
        self.sending = Some(Sending {
            started: Instant::now(),
            _task: task,
            _follow: follow,
            stop: Some(stop),
            replies: None,
        });
        cx.notify();
    }

    /// E1: opens the WebSocket. Once open, what comes is heard till it
    /// ends; then what went each way is kept, and shown as its answer.
    fn connect(&mut self, answers: Answers, cx: &mut Context<Self>) {
        let sender = self.sender(cx);
        let call = socket::connect(
            sender.clone(),
            self.collection.clone(),
            self.api.clone(),
            self.request.clone(),
            answers,
        );
        let task = cx.spawn(async move |this, cx| {
            let opened = match cx.background_spawn(call).await {
                Ok(Connected::Open(opened)) => opened,
                Ok(Connected::Kept(response)) => {
                    let _ = this.update(cx, |this, cx| this.answered(Ok(response), cx));
                    return;
                }
                Err(failure) => {
                    let _ = this.update(cx, |this, cx| this.answered(Err(failure), cx));
                    return;
                }
            };
            let LiveSocket {
                status,
                headers,
                outgoing,
                mut incoming,
                head,
            } = opened;
            if this
                .update(cx, |this, cx| {
                    this.live = Some(LiveStream::socket(status, headers));
                    this.socket = Some(OpenSocket {
                        outgoing,
                        messages: Vec::new(),
                        opened: Instant::now(),
                    });
                    cx.notify();
                })
                .is_err()
            {
                return;
            }
            let mut last = SocketIn::Closed {
                code: None,
                reason: String::new(),
            };
            while let Some((came, at)) = incoming.next().await {
                let SocketIn::Message(frame) = came else {
                    last = came;
                    break;
                };
                let heard = this.update(cx, |this, cx| {
                    if let Some(socket) = this.socket.as_mut() {
                        socket.messages.push((false, frame.clone(), at));
                    }
                    if let Some(live) = this.live.as_mut() {
                        live.message(false, &frame, at);
                    }
                    cx.notify();
                });
                if heard.is_err() {
                    return;
                }
            }
            let Ok((messages, stopping)) = this.update(cx, |this, _| {
                let messages = this.socket.take().map(|socket| socket.messages).unwrap_or_default();
                (messages, this.live.as_ref().is_some_and(|live| live.stopping))
            }) else {
                return;
            };
            let ended = socket::ending(&last, stopping);
            let kept = cx.background_spawn(socket::keep(sender, head, messages, ended)).await;
            let _ = this.update(cx, |this, cx| this.answered(Ok(kept), cx));
        });
        self.sending = Some(Sending {
            started: Instant::now(),
            _task: task,
            _follow: Task::ready(()),
            stop: None,
            replies: None,
        });
        cx.notify();
    }

    /// E1: whether the WebSocket is open, so a message can go.
    pub fn socket_open(&self) -> bool {
        self.socket.is_some()
    }

    /// E1: sends the body as it is now, its templates rendered, on the
    /// open WebSocket. `answers` are the reader's to its `prompt`s.
    pub fn send_message(&mut self, answers: Answers, cx: &mut Context<Self>) {
        if self.socket.is_none() {
            return;
        }
        self.save_now(cx);
        remember_typed(&self.collection, &answers.prompts, cx);
        let call = socket::message(self.sender(cx), self.collection.clone(), self.request.clone(), answers);
        cx.spawn(async move |this, cx| {
            let rendered = cx.background_spawn(call).await;
            let _ = this.update(cx, |this, cx| match rendered {
                Ok(text) => this.say(text, cx),
                Err(failure) => cx.emit(not_sent(failure)),
            });
        })
        .detach();
    }

    fn say(&mut self, text: String, cx: &mut Context<Self>) {
        let Some(socket) = self.socket.as_mut() else { return };
        let frame = Frame::Text(text);
        let at = socket.opened.elapsed().as_millis() as u64;
        if socket.outgoing.unbounded_send(SocketOut::Send(frame.clone())).is_ok() {
            socket.messages.push((true, frame.clone(), at));
            if let Some(live) = self.live.as_mut() {
                live.message(true, &frame, at);
            }
        }
        cx.notify();
    }

    /// E2: opens a gRPC call that streams from the client. The editor's
    /// message is not sent with it; each one goes out after, framed.
    /// Headers may arrive only once the request is finished, so the panel
    /// shows the turns from the moment it opens.
    fn connect_grpc(&mut self, answers: Answers, cx: &mut Context<Self>) {
        let sender = self.sender(cx);
        let call = grpc_stream::connect(
            sender.clone(),
            self.collection.clone(),
            self.api.clone(),
            self.request.clone(),
            answers,
        );
        let task = cx.spawn(async move |this, cx| {
            let opened = match cx.background_spawn(call).await {
                Ok(grpc_stream::Connected::Open(opened)) => opened,
                Ok(grpc_stream::Connected::Kept(response)) => {
                    let _ = this.update(cx, |this, cx| this.answered(Ok(response), cx));
                    return;
                }
                Err(failure) => {
                    let _ = this.update(cx, |this, cx| this.answered(Err(failure), cx));
                    return;
                }
            };
            let grpc_stream::OpenedCall {
                outgoing,
                mut incoming,
                reader,
                bidi,
                head,
            } = opened;
            if this
                .update(cx, |this, cx| {
                    this.live = Some(LiveStream::call());
                    this.call = Some(OpenCall {
                        outgoing,
                        messages: Vec::new(),
                        opened: Instant::now(),
                        finished: false,
                        bidi,
                        reader,
                        status: None,
                        headers: Vec::new(),
                        trailers: Vec::new(),
                        failed: None,
                    });
                    cx.notify();
                })
                .is_err()
            {
                return;
            }
            while let Some(came) = incoming.next().await {
                let ended = this.update(cx, |this, cx| this.heard(came, cx)).unwrap_or(true);
                if ended {
                    break;
                }
            }
            // `outgoing` is dropped only now: dropping it earlier would
            // reset a call whose request was finished and whose reply is
            // still on its way.
            let Ok((call, stopping)) = this.update(cx, |this, _| {
                let stopping = this.live.as_ref().is_some_and(|live| live.stopping);
                (this.call.take(), stopping)
            }) else {
                return;
            };
            let Some(call) = call else { return };
            let OpenCall {
                messages,
                status,
                headers,
                trailers,
                failed,
                ..
            } = call;
            let ended = if stopping {
                StreamEnd::Stopped
            } else if let Some(message) = failed {
                StreamEnd::Failed { message }
            } else {
                StreamEnd::Closed
            };
            let kept = cx
                .background_spawn(grpc_stream::keep(
                    sender, head, status, headers, trailers, messages, ended,
                ))
                .await;
            let _ = this.update(cx, |this, cx| this.answered(Ok(kept), cx));
        });
        self.sending = Some(Sending {
            started: Instant::now(),
            _task: task,
            _follow: Task::ready(()),
            stop: None,
            replies: None,
        });
        cx.notify();
    }

    /// One thing the call said. `true` when the call has ended.
    fn heard(&mut self, came: CallIn, cx: &mut Context<Self>) -> bool {
        let at = self
            .call
            .as_ref()
            .map(|call| call.opened.elapsed().as_millis() as u64)
            .unwrap_or(0);
        match came {
            CallIn::Head { status, headers } => {
                if let Some(call) = self.call.as_mut() {
                    call.status = Some(status);
                    call.headers = headers.clone();
                }
                if let Some(live) = self.live.as_mut() {
                    live.status = status;
                    live.headers = headers;
                }
                cx.notify();
                false
            }
            CallIn::Data(bytes) => {
                let texts = {
                    let Some(call) = self.call.as_mut() else { return true };
                    match call.reader.as_mut() {
                        Some(reader) => reader
                            .feed(&bytes, Some(at))
                            .into_iter()
                            .map(|event| event.data)
                            .collect(),
                        None => vec![format!("binary, {} bytes", bytes.len())],
                    }
                };
                for text in texts {
                    self.turn(false, text, at, cx);
                }
                false
            }
            CallIn::Trailers(fields) => {
                if let Some(call) = self.call.as_mut() {
                    call.trailers.extend(fields);
                }
                false
            }
            CallIn::Ended => true,
            CallIn::Failed(message) => {
                if let Some(call) = self.call.as_mut() {
                    call.failed = Some(message);
                }
                true
            }
        }
    }

    /// A turn of the open call, shown now and kept when it ends.
    fn turn(&mut self, sent: bool, text: String, at: u64, cx: &mut Context<Self>) {
        if let Some(call) = self.call.as_mut() {
            call.messages.push((sent, text.clone(), at));
        }
        if let Some(live) = self.live.as_mut() {
            live.grpc_turn(sent, &text, at);
        }
        cx.notify();
    }

    /// E2: whether the call is open, so a message can go.
    pub fn call_open(&self) -> bool {
        self.call.is_some()
    }

    /// E2: the request of a client stream has been finished. Nothing more
    /// is sent; the reply may still be coming.
    pub fn call_finished(&self) -> bool {
        self.call.as_ref().is_some_and(|call| call.finished)
    }

    /// E2: sends the message as it is now, its templates rendered, framed,
    /// on the open call.
    pub fn send_grpc_message(&mut self, answers: Answers, cx: &mut Context<Self>) {
        if self.call.as_ref().is_none_or(|call| call.finished) {
            return;
        }
        self.save_now(cx);
        remember_typed(&self.collection, &answers.prompts, cx);
        let call = grpc_stream::message(self.sender(cx), self.collection.clone(), self.request.clone(), answers);
        cx.spawn(async move |this, cx| {
            let rendered = cx.background_spawn(call).await;
            let _ = this.update(cx, |this, cx| match rendered {
                Ok((text, framed)) => this.say_grpc(text, framed, cx),
                Err(failure) => cx.emit(not_sent(failure)),
            });
        })
        .detach();
    }

    fn say_grpc(&mut self, text: String, framed: Vec<u8>, cx: &mut Context<Self>) {
        let Some(call) = self.call.as_mut() else { return };
        if call.finished {
            return;
        }
        let at = call.opened.elapsed().as_millis() as u64;
        if call.outgoing.unbounded_send(CallOut::Data(framed)).is_ok() {
            let text = if text.trim().is_empty() { "{}".into() } else { text };
            call.messages.push((true, text.clone(), at));
            if let Some(live) = self.live.as_mut() {
                live.grpc_turn(true, &text, at);
            }
        }
        cx.notify();
    }

    /// E2: the request of a client stream is done. The one reply comes after.
    pub fn finish_grpc(&mut self, cx: &mut Context<Self>) {
        let Some(call) = self.call.as_mut() else { return };
        if call.bidi || call.finished {
            return;
        }
        let _ = call.outgoing.unbounded_send(CallOut::Finish);
        call.finished = true;
        cx.notify();
    }

    fn told(&mut self, told: Vec<Progress>, cx: &mut Context<Self>) {
        // Told after the answer came: it is shown already.
        if self.sending.is_none() {
            return;
        }
        for progress in told {
            match progress {
                Progress::Head { status, headers } => {
                    let replies = self.sending.as_mut().and_then(|sending| sending.replies.take());
                    self.live = Some(LiveStream::new(status, headers, replies));
                }
                Progress::Body(piece) => {
                    if let Some(live) = self.live.as_mut() {
                        live.take(&piece);
                    }
                }
            }
        }
        cx.notify();
    }

    fn answered(&mut self, result: Result<HttpResponse, SendFailure>, cx: &mut Context<Self>) {
        self.sending = None;
        self.live = None;
        self.socket = None;
        self.call = None;
        match result {
            Ok(response) => {
                // F7.5: an answer that names its file is saved as it
                // arrives — never when chosen from the history.
                if response.error.is_none() && !response.body.is_empty() && response.attachment_name().is_some() {
                    self.save(response.file_name(), response.body.clone(), cx);
                }
                self.responses.insert(0, Arc::new(response));
                self.responses.truncate(self.sender.sending.history_size());
                self.shown = None;
                // What a `get` path's fields were read from is not the last.
                let api = self.api.clone();
                let _ = self.workspace.update(cx, |workspace, _| workspace.forget_answer(&api));
            }
            Err(failure) => cx.emit(not_sent(failure)),
        }
        cx.notify();
    }

    /// The shown response's body, whole, in the downloads folder.
    pub fn save_body(&mut self, cx: &mut Context<Self>) {
        if let Some(response) = self.shown_response() {
            let (name, body) = (response.file_name(), response.body.clone());
            self.save(name, body, cx);
        }
    }

    /// F7.4: the shown body opened outside the app.
    pub fn view_body(&mut self, cx: &mut Context<Self>) {
        let Some(response) = self.shown_response() else {
            return;
        };
        if let Err(message) =
            this_build::view_outside(&response.file_name(), &response.media_type(), &response.body, cx)
        {
            cx.emit(ApiEvent::NotSaved(message));
        }
    }

    fn save(&self, name: String, body: Vec<u8>, cx: &mut Context<Self>) {
        let saving = cx.background_spawn(async move { this_build::save_to_downloads(&name, &body) });
        cx.spawn(async move |this, cx| {
            let event = match saving.await {
                Ok(path) => ApiEvent::Saved(path),
                Err(message) => ApiEvent::NotSaved(message),
            };
            let _ = this.update(cx, |_, cx| cx.emit(event));
        })
        .detach();
    }

    /// D4: a response from the project's recent list, chosen as one from
    /// the API's history is — now, or once the history is read.
    pub fn show_response(&mut self, id: String, cx: &mut Context<Self>) {
        match self.history {
            Load::Loading => self.wanted = Some(id),
            _ if self.responses.iter().any(|response| response.id == id) => self.choose_response(&id, cx),
            // F1: an editor kept open since before the response was: its
            // history read again first.
            _ => {
                self.wanted = Some(id);
                self.load_history(cx);
            }
        }
    }

    /// D4: two responses of the history, by id, to compare.
    pub fn response(&self, id: &str) -> Option<Arc<HttpResponse>> {
        self.responses.iter().find(|response| response.id == id).cloned()
    }

    /// F7.10: shows a response of the history and puts its request back in
    /// the editor, saved with the next change.
    pub fn choose_response(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(response) = self.responses.iter().find(|response| response.id == id) else {
            return;
        };
        self.shown = Some(response.id.clone());
        if self.request != response.request {
            let restored = response.request.clone();
            // An edit still waiting is kept first.
            self.save_now(cx);
            self.request = restored;
            // F7.10: the restored request is saved by the next edit, and not
            // before — leaving the API or sending it writes nothing.
            self.saved = Some(self.request.clone());
            cx.emit(ApiEvent::RequestReplaced);
        }
        cx.notify();
    }

    /// Timeouts and how a request goes out are read when it goes out. The
    /// sender was built when this API was opened, so a change in settings
    /// would otherwise wait for the next launch (#42). A workspace opened on
    /// its own, as the state tests do, has no app state and keeps the
    /// settings it set.
    /// What was typed for the project's `prompt`s while the app runs.
    pub fn typed(&self, cx: &App) -> Answers {
        Answers {
            file: None,
            prompts: typed_prompts(&self.collection, cx),
        }
    }

    fn sender(&mut self, cx: &App) -> Sender {
        if let Some(store) = cx.try_global::<super::AppStore>() {
            let prefs = store.state().read(cx).prefs();
            self.sender.timeouts = prefs.timeouts;
            self.sender.sending = prefs.sending.clone();
        }
        // #66, #61: what the API takes from its folders and project now,
        // and the reader's environment.
        if let Some(workspace) = self.workspace.upgrade() {
            let workspace = workspace.read(cx);
            self.sender.inherited = workspace.inherited(&self.api);
            self.sender.environment = workspace.current_environment().cloned();
            // D3: the reader's values of the secret ones.
            self.sender.secrets = workspace.secrets();
        }
        self.sender.clone()
    }

    /// The schema of the GraphQL endpoint the request goes to, as this API
    /// or another sent there asked for it.
    pub fn schema(&self, cx: &App) -> Option<Arc<Schema>> {
        self.workspace.upgrade()?.read(cx).schema(&self.request.uri)
    }

    pub fn asking_schema(&self) -> bool {
        self.asking_schema.is_some()
    }

    /// Asks the endpoint for its schema with the introspection query, sent
    /// as the request would be; the project keeps the answer for every API
    /// sent there. Nothing goes into the history.
    pub fn ask_schema(&mut self, cx: &mut Context<Self>) {
        if self.asking_schema.is_some() {
            return;
        }
        let uri = self.request.uri.clone();
        let call = cyberapi_service::introspect(
            self.sender(cx),
            self.collection.clone(),
            self.request.clone(),
            self.typed(cx),
        );
        self.asking_schema = Some(cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(call).await;
            let _ = this.update(cx, |this, cx| {
                this.asking_schema = None;
                match result {
                    Ok(schema) => {
                        let schema = Arc::new(schema);
                        let _ = this
                            .workspace
                            .update(cx, |workspace, cx| workspace.keep_schema(&uri, schema, cx));
                    }
                    Err(why) => cx.emit(ApiEvent::NoSchema(why)),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// The gRPC definitions the call's method is chosen from: the ones the
    /// project keeps for its address — loaded by this API or another sent
    /// there — or else the ones its body keeps.
    pub fn grpc_definitions(&self, cx: &App) -> Option<Arc<GrpcDefinitions>> {
        let known = self
            .workspace
            .upgrade()
            .and_then(|workspace| workspace.read(cx).grpc_definitions(&self.request.uri));
        known.or_else(|| self.own_grpc_definitions())
    }

    fn own_grpc_definitions(&self) -> Option<Arc<GrpcDefinitions>> {
        let body = GrpcBody::read(&self.request.body);
        let mut memo = self.own_grpc.borrow_mut();
        if let Some((descriptors, read)) = memo.as_ref()
            && *descriptors == body.descriptors
        {
            return read.clone();
        }
        let read = body.definitions().and_then(Result::ok).map(|definitions| {
            Arc::new(GrpcDefinitions {
                definitions,
                source: body.source.clone(),
            })
        });
        *memo = Some((body.descriptors, read.clone()));
        read
    }

    pub fn loading_grpc(&self) -> bool {
        self.loading_grpc.is_some()
    }

    /// Asks the server's reflection for its definitions, with the request's
    /// address and metadata.
    pub fn load_grpc_reflection(&mut self, cx: &mut Context<Self>) {
        let call = cyberapi_service::reflect(
            self.sender(cx),
            self.collection.clone(),
            self.request.clone(),
            self.typed(cx),
        );
        self.load_grpc(call, FROM_REFLECTION.to_string(), cx);
    }

    /// Compiles `.proto` files for the definitions (the desktop).
    pub fn import_protos(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let names: Vec<String> = paths
            .iter()
            .filter_map(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .collect();
        let compiling = async move { this_build::compile_protos(&paths) };
        self.load_grpc(compiling, names.join(", "), cx);
    }

    fn load_grpc(
        &mut self,
        loading: impl Future<Output = Result<Definitions, String>> + Send + 'static,
        source: String,
        cx: &mut Context<Self>,
    ) {
        if self.loading_grpc.is_some() {
            return;
        }
        let uri = self.request.uri.clone();
        self.loading_grpc = Some(cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(loading).await;
            let _ = this.update(cx, |this, cx| this.grpc_loaded(result, &uri, source, cx));
        }));
        cx.notify();
    }

    fn grpc_loaded(&mut self, result: Result<Definitions, String>, uri: &str, source: String, cx: &mut Context<Self>) {
        self.loading_grpc = None;
        match result {
            Ok(definitions) => {
                let known = Arc::new(GrpcDefinitions { definitions, source });
                let _ = self.workspace.update(cx, |workspace, cx| {
                    workspace.keep_grpc_definitions(uri, known.clone(), cx)
                });
                let methods = known.definitions.methods();
                // A call with no method yet takes the first unary one, else
                // the first it can make.
                let example = if GrpcBody::read(&self.request.body).service.is_empty() {
                    methods
                        .iter()
                        .find(|method| !method.client_streaming && !method.server_streaming)
                        .or_else(|| methods.iter().find(|method| !method.client_streaming))
                        .and_then(|method| self.choose_grpc_method_in(&known, &method.service, &method.method, cx))
                } else {
                    None
                };
                cx.emit(ApiEvent::GrpcDefinitions {
                    methods: methods.len(),
                    example,
                });
            }
            Err(why) => cx.emit(ApiEvent::NoGrpcDefinitions(why)),
        }
        cx.notify();
    }

    /// Chooses the method the call makes, keeping the definitions its
    /// service needs. Answers the example of its message, for the view to
    /// put in a message not written yet.
    pub fn choose_grpc_method(&mut self, service: &str, method: &str, cx: &mut Context<Self>) -> Option<String> {
        let known = self.grpc_definitions(cx)?;
        self.choose_grpc_method_in(&known, service, method, cx)
    }

    fn choose_grpc_method_in(
        &mut self,
        known: &GrpcDefinitions,
        service: &str,
        method: &str,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let mut body = GrpcBody::read(&self.request.body);
        body.descriptors = known.definitions.kept_for(service)?;
        body.service = service.to_string();
        body.method = method.to_string();
        body.source = known.source.clone();
        self.request.body = body.to_body();
        self.save_now(cx);
        known.definitions.example(service, method)
    }

    /// The example message of the method the call makes.
    pub fn grpc_example(&self, cx: &App) -> Option<String> {
        let body = GrpcBody::read(&self.request.body);
        self.grpc_definitions(cx)?
            .definitions
            .example(&body.service, &body.method)
    }

    /// F7.8 / #69: the request as the editor holds it, as code.
    pub fn copy_code(&mut self, target: CodeTarget, cx: &mut Context<Self>) {
        let call = cyberapi_service::code(
            self.sender(cx),
            self.collection.clone(),
            self.request.clone(),
            self.typed(cx),
            target,
        );
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(call).await;
            let _ = this.update(cx, |_, cx| match result {
                Ok(code) => cx.emit(ApiEvent::Code(target, code)),
                Err(failure) => cx.emit(not_sent(failure)),
            });
        })
        .detach();
    }

    /// F7.11.
    pub fn clear_history(&mut self, cx: &mut Context<Self>) {
        let call = self
            .sender
            .storage
            .history()
            .clear(self.collection.clone(), self.api.clone());
        run_store(cx, call, |this, result, cx| {
            match result {
                Ok(()) => {
                    this.responses.clear();
                    this.shown = None;
                }
                Err(error) => cx.emit(ApiEvent::Failed(error)),
            }
            cx.notify();
        });
    }
}

/// F4.10: rows with neither a key nor a value are not kept.
fn kept(list: Vec<KvParam>) -> Vec<KvParam> {
    list.into_iter()
        .filter(|kv| !kv.key.trim().is_empty() || !kv.value.trim().is_empty())
        .collect()
}
