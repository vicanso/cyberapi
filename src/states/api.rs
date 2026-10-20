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

use super::{Load, WorkspaceState, run_store};
use cyberapi_core::model::{ApiSetting, ContentType, HttpMethod, HttpRequest, KvParam};
use cyberapi_core::response::{HISTORY_LIMIT, HttpResponse};
use cyberapi_core::{CollectionId, ItemId};
use cyberapi_service::{SendFailure, Sender};
use cyberapi_store::StoreError;
use gpui_kit::{AppContext as _, Context, EventEmitter, Task, WeakEntity};
use std::time::Duration;
use tracing::warn;
use web_time::Instant;

/// How long typing waits before it is saved (F4.11).
pub const SAVE_DELAY: Duration = Duration::from_millis(300);
/// A send this soon after the last one is the same click (F5.1).
const DOUBLE_CLICK: Duration = Duration::from_millis(200);

#[derive(Clone, Debug)]
pub enum ApiEvent {
    /// A change could not be saved, or the history could not be read.
    Failed(StoreError),
    /// Nothing was sent: the request could not be prepared (F5.7).
    NotSent(String),
    /// The request was replaced from the history (F7.10): the editor's
    /// fields read it again.
    RequestReplaced,
    /// The cURL command asked for (F7.8), for the clipboard.
    Curl(String),
}

/// A request on its way.
struct Sending {
    started: Instant,
    _task: Task<()>,
}

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
    last_send: Option<Instant>,
    /// Newest first (F7.9).
    responses: Vec<HttpResponse>,
    history: Load,
    /// The response the panel shows: the newest unless one was chosen.
    shown: Option<String>,
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
            last_send: None,
            responses: Vec::new(),
            history: Load::Loading,
            shown: None,
        };
        state.load_history(cx);
        state
    }

    pub fn api(&self) -> &ItemId {
        &self.api
    }

    pub fn request(&self) -> &HttpRequest {
        &self.request
    }

    pub fn responses(&self) -> &[HttpResponse] {
        &self.responses
    }

    pub fn history(&self) -> &Load {
        &self.history
    }

    /// F7.12: the newest response, or the one chosen from the history.
    pub fn shown_response(&self) -> Option<&HttpResponse> {
        match &self.shown {
            Some(id) => self.responses.iter().find(|response| &response.id == id),
            None => self.responses.first(),
        }
    }

    /// When the request on its way was sent, if one is.
    pub fn sending_since(&self) -> Option<Instant> {
        self.sending.as_ref().map(|sending| sending.started)
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
                    this.responses = responses;
                    this.history = Load::Loaded;
                }
                Err(error) => {
                    this.history = Load::Failed(error.clone());
                    cx.emit(ApiEvent::Failed(error));
                }
            }
            cx.notify();
        });
    }

    /// F4.2: saved at once.
    pub fn set_method(&mut self, method: HttpMethod, cx: &mut Context<Self>) {
        if self.request.method != method {
            self.request.method = method;
            self.save_now(cx);
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

    /// F4.8: a form or multipart body's fields.
    pub fn set_form(&mut self, fields: Vec<KvParam>, cx: &mut Context<Self>) {
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

    pub fn set_auth(&mut self, auth: Vec<KvParam>, cx: &mut Context<Self>) {
        let auth = kept(auth);
        if self.request.auth != auth {
            self.request.auth = auth;
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
        self.saved = Some(request);
        run_store(cx, call, |this, result, cx| match result {
            Ok(setting) => {
                let _ = this
                    .workspace
                    .update(cx, |workspace, cx| workspace.api_saved(setting, cx));
            }
            Err(error) => {
                // Not saved after all: the next save goes out.
                this.saved = None;
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
        if self.sending.take().is_some() {
            cx.notify();
            return;
        }
        self.save_now(cx);
        let call = cyberapi_service::send(
            self.sender.clone(),
            self.collection.clone(),
            self.api.clone(),
            self.request.clone(),
            picked_file,
        );
        let task = cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(call).await;
            let _ = this.update(cx, |this, cx| this.answered(result, cx));
        });
        self.sending = Some(Sending {
            started: Instant::now(),
            _task: task,
        });
        cx.notify();
    }

    fn answered(&mut self, result: Result<HttpResponse, SendFailure>, cx: &mut Context<Self>) {
        self.sending = None;
        match result {
            Ok(response) => {
                self.responses.insert(0, response);
                self.responses.truncate(HISTORY_LIMIT);
                self.shown = None;
            }
            Err(failure) => cx.emit(ApiEvent::NotSent(failure.to_string())),
        }
        cx.notify();
    }

    /// F7.10: shows a response of the history and puts its request back in
    /// the editor, saved with the next change.
    pub fn choose_response(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(response) = self.responses.iter().find(|response| response.id == id) else {
            return;
        };
        self.shown = Some(response.id.clone());
        if self.request != response.request {
            self.request = response.request.clone();
            self.save_task = None;
            cx.emit(ApiEvent::RequestReplaced);
        }
        cx.notify();
    }

    /// F7.8: the command of the request as the editor holds it.
    pub fn copy_curl(&mut self, cx: &mut Context<Self>) {
        let call = cyberapi_service::curl(self.sender.clone(), self.collection.clone(), self.request.clone());
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(call).await;
            let _ = this.update(cx, |_, cx| match result {
                Ok(command) => cx.emit(ApiEvent::Curl(command)),
                Err(failure) => cx.emit(ApiEvent::NotSent(failure.to_string())),
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
