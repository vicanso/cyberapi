//! Sending an API's request (M5): gather what its template functions need,
//! prepare it, send it, and keep the answer in the API's history (F5.8).
//!
//! A request that cannot be prepared — a file that does not read, a URL that
//! is not one — is refused before anything goes out, and nothing is kept
//! (F5.7). A request that went out and failed is kept like any other answer,
//! with the reason as its error.

use cyberapi_core::Timestamp;
use cyberapi_core::cookie::{cookie_header, parse_set_cookie};
use cyberapi_core::model::{HttpRequest, Variable, VariableCategory};
use cyberapi_core::prefs::Timeouts;
use cyberapi_core::response::HttpResponse;
use cyberapi_core::send::{self, PrepareError, PreparedRequest};
use cyberapi_core::template::{FileReader, Inputs};
use cyberapi_core::{CollectionId, ItemId};
use cyberapi_http::HttpEngine;
use cyberapi_store::{Storage, StoreError};
use snafu::Snafu;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tracing::warn;
use web_time::{Instant, SystemTime, UNIX_EPOCH};

/// Why nothing was sent.
#[derive(Debug, Snafu, PartialEq, Eq)]
pub enum SendFailure {
    #[snafu(display("{source}"))]
    Prepare { source: PrepareError },
    /// The project's variables or the responses its functions read could
    /// not be loaded.
    #[snafu(display("{source}"))]
    Store { source: StoreError },
}

/// What sending needs besides the request: where to send it from, and the
/// reader's settings.
#[derive(Clone)]
pub struct Sender {
    pub storage: Storage,
    pub engine: Arc<dyn HttpEngine>,
    /// For `readFile` and multipart files; the browser build has none.
    pub files: Arc<dyn FileReader + Send + Sync>,
    pub user_agent: String,
    pub timeouts: Timeouts,
}

/// A boundary no body will contain by chance: the time and a counter.
fn boundary() -> String {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_nanos())
        .unwrap_or_default();
    format!(
        "----CyberAPIBoundary{nanos:x}{:x}",
        COUNT.fetch_add(1, Ordering::Relaxed)
    )
}

/// Sends `request` — the API's, as the editor holds it — and keeps the
/// answer. `picked_file` is the reader's answer to an `openFile` the request
/// needs (`cyberapi_core::send::needs` says so), asked for beforehand.
pub async fn send(
    sender: Sender,
    collection: CollectionId,
    api: ItemId,
    request: HttpRequest,
    picked_file: Option<String>,
) -> Result<HttpResponse, SendFailure> {
    let prepared = prepare(&sender, &collection, &request, picked_file).await?;
    let url = url::Url::parse(&prepared.url).ok();
    let started = Instant::now();
    let answered = sender.engine.send(prepared).await;
    // F9.2: the answer's cookies are kept, an expired one removing its own.
    if let (Ok(exchange), Some(url)) = (&answered, &url) {
        let cookies: Vec<_> = exchange
            .headers
            .iter()
            .filter(|(name, _)| name == "set-cookie")
            .filter_map(|(_, value)| parse_set_cookie(value, url, now()))
            .collect();
        if !cookies.is_empty()
            && let Err(error) = sender.storage.cookies().put(cookies).await
        {
            warn!(%error, "the response's cookies could not be kept");
        }
    }
    // Never zero (F5.7).
    let floor = |latency: Duration| latency.max(Duration::from_millis(1));
    let response = match answered {
        Ok(exchange) => HttpResponse {
            api: api.clone(),
            collection: collection.clone(),
            request,
            status: Some(exchange.status),
            headers: exchange.headers,
            body: exchange.body,
            error: None,
            body_size: exchange.body_size,
            latency: floor(exchange.stats.total),
            stats: exchange.stats,
            ..Default::default()
        },
        Err(error) => HttpResponse {
            api: api.clone(),
            collection: collection.clone(),
            request,
            error: Some(error.to_string()),
            latency: floor(started.elapsed()),
            ..Default::default()
        },
    };
    match sender.storage.history().add(response.clone()).await {
        Ok(kept) => Ok(kept),
        // The answer still shows; it is only not kept.
        Err(error) => {
            warn!(%error, "the response could not be kept");
            Ok(response)
        }
    }
}

/// F7.8: the cURL command of `request`, prepared as sending would prepare
/// it. An `openFile` in it is not asked: its parameter stands.
pub async fn curl(sender: Sender, collection: CollectionId, request: HttpRequest) -> Result<String, SendFailure> {
    let prepared = prepare(&sender, &collection, &request, None).await?;
    Ok(cyberapi_core::curl::curl(&prepared, &request, &[]))
}

fn now() -> Timestamp {
    Timestamp::from_unix(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|since| since.as_secs() as i64)
            .unwrap_or_default(),
    )
}

/// Everything before the engine: the variables, the responses the template
/// functions read, then `cyberapi_core::send::prepare`, and the cookies that
/// go with it (F9.3) — unless the request names its own `Cookie`.
async fn prepare(
    sender: &Sender,
    collection: &CollectionId,
    request: &HttpRequest,
    picked_file: Option<String>,
) -> Result<PreparedRequest, SendFailure> {
    let store = |source| SendFailure::Store { source };
    let mut variables: Vec<Variable> = Vec::new();
    for category in [
        VariableCategory::Environment,
        VariableCategory::Custom,
        VariableCategory::GlobalRequestHeader,
    ] {
        variables.extend(
            sender
                .storage
                .variables()
                .list(collection.clone(), category)
                .await
                .map_err(store)?,
        );
    }

    let needs = send::needs(request, &variables);
    let responses = if needs.responses.is_empty() {
        Vec::new()
    } else {
        sender
            .storage
            .history()
            .latest(
                collection.clone(),
                needs.responses.iter().map(|id| ItemId::new(id.as_str())).collect(),
            )
            .await
            .map_err(store)?
    };
    let inputs = Inputs {
        now: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|since| since.as_secs() as i64)
            .unwrap_or_default(),
        custom: variables
            .iter()
            .filter(|v| v.enabled && v.category == VariableCategory::Custom)
            .map(|v| (v.name.clone(), v.value.clone()))
            .collect(),
        responses: responses
            .iter()
            .filter_map(|response| Some((response.api.to_string(), response.body_json()?)))
            .collect(),
        picked_file,
    };
    let mut prepared = send::prepare(
        request,
        &send::Context {
            variables: &variables,
            template: &inputs,
            files: sender.files.as_ref(),
            user_agent: &sender.user_agent,
            timeouts: sender.timeouts,
            boundary: &boundary(),
        },
    )
    .map_err(|source| SendFailure::Prepare { source })?;
    if prepared.header("cookie").is_none()
        && let Ok(url) = url::Url::parse(&prepared.url)
    {
        let jar = sender.storage.cookies().list().await.map_err(store)?;
        if let Some(cookie) = cookie_header(&jar, &url, now()) {
            prepared.headers.push(("Cookie".into(), cookie));
        }
    }
    Ok(prepared)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cyberapi_core::model::{ContentType, HttpMethod, KvParam};
    use cyberapi_core::response::Exchange;
    use cyberapi_core::template::{BaseDir, TemplateError};
    use cyberapi_http::{SendError, SendFuture};
    use cyberapi_store::{MemoryStore, NewCollection, NewVariable};
    use pretty_assertions::assert_eq;
    use std::sync::Mutex;

    /// Answers each request with `answer` and keeps what it was sent.
    struct Engine {
        answer: Result<Exchange, SendError>,
        sent: Mutex<Vec<PreparedRequest>>,
    }

    impl HttpEngine for Engine {
        fn send(&self, request: PreparedRequest) -> SendFuture {
            if let Ok(mut sent) = self.sent.lock() {
                sent.push(request);
            }
            let answer = self.answer.clone();
            Box::pin(async move { answer })
        }
    }

    struct Files;

    impl FileReader for Files {
        fn read(&self, file: &str, _: Option<BaseDir>) -> Result<Vec<u8>, String> {
            match file {
                "token.txt" => Ok(b"from-file".to_vec()),
                _ => Err("no such file".into()),
            }
        }
    }

    fn kv(key: &str, value: &str) -> KvParam {
        KvParam {
            key: key.into(),
            value: value.into(),
            enabled: true,
        }
    }

    struct Setup {
        storage: Storage,
        engine: Arc<Engine>,
        sender: Sender,
        collection: CollectionId,
    }

    fn setup(answer: Result<Exchange, SendError>) -> Setup {
        let storage = MemoryStore::new().storage();
        let engine = Arc::new(Engine {
            answer,
            sent: Mutex::new(Vec::new()),
        });
        let collection = pollster::block_on(storage.collections().create(NewCollection {
            name: "p".into(),
            ..Default::default()
        }))
        .expect("create")
        .id;
        let sender = Sender {
            storage: storage.clone(),
            engine: engine.clone(),
            files: Arc::new(Files),
            user_agent: "CyberAPI/test".into(),
            timeouts: Timeouts::default(),
        };
        Setup {
            storage,
            engine,
            sender,
            collection,
        }
    }

    fn variable(setup: &Setup, category: VariableCategory, name: &str, value: &str) {
        pollster::block_on(setup.storage.variables().create(NewVariable {
            collection: setup.collection.clone(),
            category,
            name: name.into(),
            value: value.into(),
            enabled: true,
        }))
        .expect("create a variable");
    }

    fn new_api(setup: &Setup, name: &str) -> ItemId {
        pollster::block_on(
            setup
                .storage
                .tree()
                .create_api(setup.collection.clone(), None, name.into()),
        )
        .expect("create an API")
        .item
        .id
    }

    fn ok(status: u16, body: &str) -> Result<Exchange, SendError> {
        Ok(Exchange {
            status,
            headers: vec![("content-type".into(), "application/json".into())],
            body: body.as_bytes().to_vec(),
            body_size: body.len() as u64,
            ..Default::default()
        })
    }

    #[test]
    fn variables_functions_and_history_come_together() {
        let setup = setup(ok(200, r#"{"token": "t-9"}"#));
        variable(&setup, VariableCategory::Environment, "HOST", "https://api.test");
        variable(&setup, VariableCategory::Custom, "tenant", "acme");
        variable(
            &setup,
            VariableCategory::GlobalRequestHeader,
            "X-Tenant",
            "{{value(tenant)}}",
        );
        let login = new_api(&setup, "login");
        let me = new_api(&setup, "me");

        // The first API's answer is what the second reads.
        let first = pollster::block_on(send(
            setup.sender.clone(),
            setup.collection.clone(),
            login.clone(),
            HttpRequest {
                uri: "{{HOST}}/login".into(),
                ..Default::default()
            },
            None,
        ))
        .expect("sent");
        assert_eq!(first.status, Some(200));
        assert!(!first.id.is_empty(), "kept in the history");

        let request = HttpRequest {
            method: HttpMethod::Post,
            uri: "{{HOST}}/me".into(),
            content_type: ContentType::Json,
            body: r#"{"file": "{{rtf(token.txt)}}", "picked": "{{of()}}"}"#.into(),
            headers: vec![kv("Authorization", &format!("Bearer {{{{get({login}, token)}}}}"))],
            ..Default::default()
        };
        let second = pollster::block_on(send(
            setup.sender.clone(),
            setup.collection.clone(),
            me.clone(),
            request.clone(),
            Some("/tmp/a.png".into()),
        ))
        .expect("sent");

        let sent = setup.engine.sent.lock().expect("sent").clone();
        let prepared = &sent[1];
        assert_eq!(prepared.url, "https://api.test/me");
        assert_eq!(prepared.header("Authorization"), Some("Bearer t-9"));
        assert_eq!(prepared.header("X-Tenant"), Some("acme"));
        assert_eq!(prepared.header("User-Agent"), Some("CyberAPI/test"));
        assert_eq!(
            String::from_utf8_lossy(&prepared.body),
            r#"{"file": "from-file", "picked": "/tmp/a.png"}"#
        );

        // The history keeps the request as the API held it.
        assert_eq!(second.request, request);
        assert!(second.latency >= Duration::from_millis(1));
        let history = pollster::block_on(setup.storage.history().list(setup.collection.clone(), me)).expect("list");
        assert_eq!(history, [second]);
    }

    #[test]
    fn cookies_come_back_with_later_requests() {
        let setup = setup(Ok(Exchange {
            status: 200,
            headers: vec![
                ("set-cookie".into(), "sid=s-1; Path=/; HttpOnly".into()),
                ("set-cookie".into(), "other=o; Domain=elsewhere.test".into()),
            ],
            ..Default::default()
        }));
        let api = new_api(&setup, "login");
        let request = |uri: &str, headers: Vec<KvParam>| HttpRequest {
            uri: uri.into(),
            headers,
            ..Default::default()
        };
        let send_one = |request: HttpRequest| {
            pollster::block_on(send(
                setup.sender.clone(),
                setup.collection.clone(),
                api.clone(),
                request,
                None,
            ))
            .expect("sent")
        };
        send_one(request("https://api.test/login", vec![]));
        let jar = pollster::block_on(setup.storage.cookies().list()).expect("cookies");
        assert_eq!(jar.len(), 1, "a cookie for another site is not kept: {jar:?}");
        assert_eq!((jar[0].name.as_str(), jar[0].domain.as_str()), ("sid", "api.test"));

        send_one(request("https://api.test/me", vec![]));
        send_one(request("https://other.test/", vec![]));
        send_one(request("https://api.test/me", vec![kv("Cookie", "mine=1")]));
        let sent = setup.engine.sent.lock().expect("sent").clone();
        assert_eq!(sent[1].header("cookie"), Some("sid=s-1"));
        assert_eq!(sent[2].header("cookie"), None);
        assert_eq!(sent[3].header("cookie"), Some("mine=1"), "the request's own wins");
    }

    #[test]
    fn curl_is_what_sending_would_send() {
        let setup = setup(ok(200, "{}"));
        variable(&setup, VariableCategory::Environment, "HOST", "https://api.test");
        let command = pollster::block_on(curl(
            setup.sender.clone(),
            setup.collection.clone(),
            HttpRequest {
                uri: "{{HOST}}/me".into(),
                query: vec![kv("v", "{{md5(a)}}")],
                ..Default::default()
            },
        ))
        .expect("a command");
        assert_eq!(
            command,
            "curl -v -XGET -H 'User-Agent: CyberAPI/test' 'https://api.test/me?v=0cc175b9c0f1b6a831c399e269772661'"
        );
        assert!(setup.engine.sent.lock().expect("sent").is_empty(), "nothing is sent");
    }

    #[test]
    fn a_failed_exchange_is_kept_with_its_reason() {
        let setup = setup(Err(SendError::Connect {
            message: "refused".into(),
        }));
        let api = new_api(&setup, "down");
        let response = pollster::block_on(send(
            setup.sender.clone(),
            setup.collection.clone(),
            api.clone(),
            HttpRequest {
                uri: "http://down.test".into(),
                ..Default::default()
            },
            None,
        ))
        .expect("an answer, of a kind");
        assert_eq!(response.status, None);
        assert_eq!(response.error.as_deref(), Some("could not connect: refused"));
        assert!(response.latency >= Duration::from_millis(1));
        let history = pollster::block_on(setup.storage.history().list(setup.collection.clone(), api)).expect("list");
        assert_eq!(history.len(), 1);
    }

    #[test]
    fn what_cannot_be_prepared_is_not_sent_or_kept() {
        let setup = setup(ok(200, "{}"));
        let api = new_api(&setup, "files");
        let failure = pollster::block_on(send(
            setup.sender.clone(),
            setup.collection.clone(),
            api.clone(),
            HttpRequest {
                method: HttpMethod::Post,
                uri: "http://a.test".into(),
                body: "{{rf(missing.bin)}}".into(),
                ..Default::default()
            },
            None,
        ));
        assert_eq!(
            failure,
            Err(SendFailure::Prepare {
                source: PrepareError::Template {
                    source: TemplateError::ReadFile {
                        file: "missing.bin".into(),
                        message: "no such file".into()
                    }
                }
            })
        );
        assert!(setup.engine.sent.lock().expect("sent").is_empty());
        let history = pollster::block_on(setup.storage.history().list(setup.collection.clone(), api)).expect("list");
        assert!(history.is_empty());
    }
}
