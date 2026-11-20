//! H3: the team's model (`docs/ai-assistant.md` §4). An administrator sets
//! its address, name and key on the page (`/v1/ai/settings/save`, kept in
//! the store's `bridge_settings`), or the bridge's options give them
//! (`--ai-url`, `--ai-model`, `--ai-protocol`, `CYBERAPI_BRIDGE_AI_KEY`);
//! the page's win. The key stays in this process and the store: no answer
//! carries it. `GET /v1/ai` says whether there is a model; `POST
//! /v1/ai/chat` takes a conversation (`cyberapi_core::ai::Chat`), asks the
//! model with the key added, and answers its stream as NDJSON, one
//! `AiEvent` a line. A page that leaves ends the call. Signed-in sessions
//! only, not MCP tokens; each account [`CHATS_PER_MINUTE`] a minute, its
//! turns counted with its sends.

use crate::Bridge;
use crate::auth::signed_in;
use axum::extract::{Json, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use cyberapi_core::ai::{AiEvent, Chat, KeptModel, ModelChange, ModelConfig, ModelSettings, Reader, refusal};
use cyberapi_core::model::HttpMethod;
use cyberapi_core::prefs::Timeouts;
use cyberapi_http::{PreparedRequest, Progress, Watch, stop_signal};
use futures::StreamExt as _;
use serde_json::json;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

/// The turns an account may ask the model for in a minute.
pub const CHATS_PER_MINUTE: usize = 20;

/// The name the page's model is kept under in `bridge_settings`.
const SETTING: &str = "ai";

/// The model in use.
pub struct Ai {
    config: ModelConfig,
    /// Set on the page, not by the bridge's options.
    on_page: bool,
}

impl Ai {
    pub fn new(config: ModelConfig, on_page: bool) -> Self {
        Self { config, on_page }
    }

    fn settings(&self) -> ModelSettings {
        ModelSettings {
            protocol: self.config.protocol,
            url: self.config.url.clone(),
            model: self.config.model.clone(),
            has_key: !self.config.key.is_empty(),
            on_page: self.on_page,
        }
    }
}

/// The page's model kept in the store, if an administrator set one.
pub async fn kept(bridge: &Bridge) -> Result<Option<ModelConfig>, String> {
    let Some(text) = bridge
        .inner
        .store
        .bridge_setting(SETTING)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Ok(None);
    };
    let kept: KeptModel =
        serde_json::from_str(&text).map_err(|error| format!("the kept model did not read: {error}"))?;
    Ok(Some(kept.config()))
}

fn current(bridge: &Bridge) -> Option<Arc<Ai>> {
    bridge.inner.ai.read().unwrap_or_else(PoisonError::into_inner).clone()
}

/// `{"model": "…"}`, or `null` when the bridge has none.
pub async fn info(State(bridge): State<Bridge>, headers: HeaderMap) -> Response {
    if let Err(status) = signed_in(&bridge.inner.store, &headers).await {
        return status.into_response();
    }
    match current(&bridge) {
        Some(ai) => Json(json!({ "model": ai.config.model })).into_response(),
        None => Json(serde_json::Value::Null).into_response(),
    }
}

/// An administrator's session, or the answer to anyone else.
async fn administrator(
    bridge: &Bridge,
    headers: &HeaderMap,
) -> Result<cyberapi_store_sql::Account, (StatusCode, &'static str)> {
    let account = signed_in(&bridge.inner.store, headers)
        .await
        .map_err(|status| (status, ""))?;
    if !account.admin {
        return Err((StatusCode::FORBIDDEN, ""));
    }
    if bridge.inner.ai_engine.is_none() {
        return Err((StatusCode::NOT_FOUND, "the bridge has no assistant"));
    }
    Ok(account)
}

/// The model in use, without its key, or `null`: administrators only.
pub async fn settings(State(bridge): State<Bridge>, headers: HeaderMap) -> Response {
    if let Err(answer) = administrator(&bridge, &headers).await {
        return answer.into_response();
    }
    Json(current(&bridge).map(|ai| ai.settings())).into_response()
}

/// Sets the page's model, or with `null` puts the bridge's options back;
/// answers the model now in use as [`settings`] does.
pub async fn save(
    State(bridge): State<Bridge>,
    headers: HeaderMap,
    Json(change): Json<Option<ModelChange>>,
) -> Response {
    let account = match administrator(&bridge, &headers).await {
        Ok(account) => account,
        Err(answer) => return answer.into_response(),
    };
    let store = &bridge.inner.store;
    let now = match change {
        Some(change) => {
            // A blank key keeps the page's; never copies the options' one in.
            let kept_key = current(&bridge).filter(|ai| ai.on_page).map(|ai| ai.config.key.clone());
            let kept = match change.applied(kept_key.as_deref()) {
                Ok(kept) => kept,
                Err(reason) => return (StatusCode::UNPROCESSABLE_ENTITY, reason).into_response(),
            };
            let text = serde_json::to_string(&kept).unwrap_or_default();
            if let Err(error) = store.set_bridge_setting(SETTING, Some(&text)).await {
                tracing::error!(%error, "the model could not be kept");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
            Some(Arc::new(Ai::new(kept.config(), true)))
        }
        None => {
            if let Err(error) = store.set_bridge_setting(SETTING, None).await {
                tracing::error!(%error, "the kept model could not be removed");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
            bridge
                .inner
                .ai_options
                .clone()
                .map(|config| Arc::new(Ai::new(config, false)))
        }
    };
    tracing::info!(
        account = %account.id,
        url = now.as_ref().map(|ai| ai.config.url.as_str()).unwrap_or(""),
        model = now.as_ref().map(|ai| ai.config.model.as_str()).unwrap_or(""),
        on_page = now.as_ref().is_some_and(|ai| ai.on_page),
        "the assistant's model was set"
    );
    let settings = now.as_ref().map(|ai| ai.settings());
    *bridge.inner.ai.write().unwrap_or_else(PoisonError::into_inner) = now;
    Json(settings).into_response()
}

fn line(event: &AiEvent) -> Vec<u8> {
    let mut line = serde_json::to_vec(event).unwrap_or_default();
    line.push(b'\n');
    line
}

fn failed(message: impl Into<String>) -> Response {
    crate::ndjson(axum::body::Body::from(line(&AiEvent::Error {
        message: message.into(),
    })))
}

pub async fn chat(State(bridge): State<Bridge>, headers: HeaderMap, Json(chat): Json<Chat>) -> Response {
    let (Some(ai), Some(engine)) = (current(&bridge), bridge.inner.ai_engine.clone()) else {
        return (StatusCode::NOT_FOUND, "the bridge has no model").into_response();
    };
    let account = match signed_in(&bridge.inner.store, &headers).await {
        Ok(account) => account,
        Err(status) => return status.into_response(),
    };
    if !bridge
        .inner
        .ai_calls
        .allow_up_to(account.id.as_str(), Instant::now(), CHATS_PER_MINUTE)
    {
        return failed(format!(
            "more than {CHATS_PER_MINUTE} questions in a minute: wait a little and try again"
        ));
    }
    let sending = match bridge.inner.in_flight.start(&account.id) {
        Ok(sending) => sending,
        Err(_) => return failed("too many requests on their way: wait for one to finish"),
    };
    let asked = ai.config.request(&chat);
    let request = PreparedRequest {
        method: HttpMethod::Post,
        url: asked.url,
        headers: asked.headers,
        body: asked.body,
        timeouts: Timeouts::default().effective(),
        ..Default::default()
    };
    let (lines, answer) = futures::channel::mpsc::unbounded::<Vec<u8>>();
    let reader = Arc::new(Mutex::new(Some(Reader::new(ai.config.protocol))));
    let told = (lines.clone(), reader.clone());
    let (keep_going, stop) = stop_signal();
    let watch = Watch {
        progress: Arc::new(move |progress| {
            let Progress::Body(piece) = progress else {
                return;
            };
            let mut reader = told.1.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(reader) = reader.as_mut() {
                for event in reader.feed(&piece) {
                    let _ = told.0.unbounded_send(line(&event));
                }
            }
        }),
        stop,
    };
    let (model, account_id) = (ai.config.model.clone(), account.id.clone());
    let task = tokio::spawn(async move {
        let (_sending, _keep_going) = (sending, keep_going);
        let started = Instant::now();
        let ending = match engine.send_watched(request, watch).await {
            // A stream: what the reader still holds, and its end.
            Ok(exchange) if exchange.stream.is_some() => reader
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
                .map(Reader::finish)
                .unwrap_or_default(),
            // Refused: an error's body comes whole.
            Ok(exchange) => vec![AiEvent::Error {
                message: refusal(exchange.status, &exchange.body),
            }],
            Err(error) => vec![AiEvent::Error {
                message: format!("the model could not be reached: {error}"),
            }],
        };
        let (input_tokens, output_tokens) = ending
            .iter()
            .find_map(|event| match event {
                AiEvent::Done {
                    input_tokens,
                    output_tokens,
                    ..
                } => Some((*input_tokens, *output_tokens)),
                _ => None,
            })
            .unwrap_or_default();
        let ok = !ending.iter().any(|event| matches!(event, AiEvent::Error { .. }));
        tracing::info!(
            account = %account_id,
            %model,
            ms = started.elapsed().as_millis() as u64,
            ?input_tokens,
            ?output_tokens,
            ok,
            "ai chat"
        );
        for event in ending {
            let _ = lines.unbounded_send(line(&event));
        }
    });
    // Goes with the answer: a page that leaves ends the call.
    let call = crate::AbortOnDrop(task.abort_handle());
    let body = answer.map(move |line| {
        let _ = &call;
        Ok::<_, std::convert::Infallible>(line)
    });
    crate::ndjson(axum::body::Body::from_stream(body))
}
