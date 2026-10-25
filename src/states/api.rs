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

use super::{Load, WorkspaceState, run_store};
use crate::this_build;
use cyberapi_core::auth::Auth;
use cyberapi_core::code::CodeTarget;
use cyberapi_core::graphql_schema::Schema;
use cyberapi_core::grpc::{self, Definitions, FROM_REFLECTION, GrpcBody, ReplyReader};
use cyberapi_core::model::{ApiSetting, ContentType, HttpMethod, HttpRequest, KvParam};
use cyberapi_core::prefs::RequestSettings;
use cyberapi_core::response::{HISTORY_LIMIT, HttpResponse};
use cyberapi_core::send::PrepareError;
use cyberapi_core::sse;
use cyberapi_core::{CollectionId, ItemId};
use cyberapi_http::{Progress, Watch, stop_signal};
use cyberapi_service::{SendFailure, Sender};
use cyberapi_store::StoreError;
use futures::StreamExt as _;
use futures::channel::{mpsc, oneshot};
use gpui_kit::{App, AppContext as _, Context, EventEmitter, Task, WeakEntity};
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;
use web_time::Instant;

/// How long typing waits before it is saved (F4.11).
pub const SAVE_DELAY: Duration = Duration::from_millis(300);
/// A send this soon after the last one is the same click (F5.1).
const DOUBLE_CLICK: Duration = Duration::from_millis(200);

/// Why nothing was sent, as the view hears it.
fn not_sent(failure: SendFailure) -> ApiEvent {
    match failure {
        SendFailure::Prepare {
            source: PrepareError::Environment { name, within },
        } => ApiEvent::NoEnvironment { name, within },
        SendFailure::Prepare {
            source: PrepareError::GraphQlVariables { message },
        } => ApiEvent::GraphQlVariables(message),
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
        };
        self.events.extend(events);
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
    request: HttpRequest,
    /// What the store has, as far as this state knows; `None` after a save
    /// failed, so the next one goes out whatever it holds.
    saved: Option<HttpRequest>,
    save_task: Option<Task<()>>,
    sending: Option<Sending>,
    /// The event stream being answered, while it comes.
    live: Option<LiveStream>,
    last_send: Option<Instant>,
    /// Newest first (F7.9). Shared, so the panel takes one for each frame
    /// without copying its body.
    responses: Vec<Arc<HttpResponse>>,
    history: Load,
    /// The response the panel shows: the newest unless one was chosen.
    shown: Option<String>,
    /// A GraphQL endpoint's schema being asked for.
    asking_schema: Option<Task<()>>,
    /// gRPC definitions being loaded.
    loading_grpc: Option<Task<()>>,
    /// The gRPC definitions the body keeps, read once for the descriptors
    /// it keeps: what the view draws asks for them every frame.
    own_grpc: RefCell<Option<(String, Option<Arc<GrpcDefinitions>>)>>,
}

impl EventEmitter<ApiEvent> for ApiState {}

impl ApiState {
    pub fn new(
        sender: Sender,
        workspace: WeakEntity<WorkspaceState>,
        api: &ApiSetting,
        editable: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        // An edit not saved yet is saved as the state goes.
        cx.on_release(|this, cx| {
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
            request: api.request.clone(),
            saved: Some(api.request.clone()),
            save_task: None,
            sending: None,
            live: None,
            last_send: None,
            responses: Vec::new(),
            history: Load::Loading,
            shown: None,
            asking_schema: None,
            loading_grpc: None,
            own_grpc: RefCell::new(None),
        };
        state.load_history(cx);
        state
    }

    pub fn api(&self) -> &ItemId {
        &self.api
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
    /// it now (#52).
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
            let replaced = body != self.request.body;
            self.request.body = body;
            self.request.method = method;
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
    pub fn save_now(&mut self, cx: &mut Context<Self>) {
        self.save_task = None;
        cx.notify();
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
        run_store(cx, call, move |this, result, cx| match result {
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

    /// F5.1: sends, or cancels the request on its way. `picked_file` answers
    /// the `openFile` the request needs, if it needs one.
    pub fn send_or_cancel(&mut self, picked_file: Option<String>, cx: &mut Context<Self>) {
        if self.last_send.is_some_and(|last| last.elapsed() < DOUBLE_CLICK) {
            return;
        }
        self.last_send = Some(Instant::now());
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
            picked_file,
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
        match result {
            Ok(response) => {
                // F7.5: an answer that names its file is saved as it
                // arrives — never when chosen from the history.
                if response.error.is_none() && !response.body.is_empty() && response.attachment_name().is_some() {
                    self.save(response.file_name(), response.body.clone(), cx);
                }
                self.responses.insert(0, Arc::new(response));
                self.responses.truncate(HISTORY_LIMIT);
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
        let call = cyberapi_service::introspect(self.sender(cx), self.collection.clone(), self.request.clone());
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
        let call = cyberapi_service::reflect(self.sender(cx), self.collection.clone(), self.request.clone());
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
        let call = cyberapi_service::code(self.sender(cx), self.collection.clone(), self.request.clone(), target);
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
