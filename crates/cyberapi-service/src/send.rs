//! Sending an API's request (M5): gather what its template functions need,
//! prepare it, send it, and keep the answer in the API's history (F5.8).
//!
//! A request that cannot be prepared — a file that does not read, a URL that
//! is not one — is refused before anything goes out, and nothing is kept
//! (F5.7). A request that went out and failed is kept like any other answer,
//! with the reason as its error.
//!
//! When the reader follows redirects (F14.8), each hop is a request of its
//! own (`cyberapi_core::redirect`): its cookies are kept, the next one gets
//! the jar's for where it goes, and the last answer is the one kept, with
//! the address it came from and the time of every hop.
//!
//! A gRPC call (`cyberapi_core::grpc`) follows no redirect; its reply is
//! kept read as JSON, and its trailers after the headers. [`reflect`] asks a
//! server for the definitions its calls are encoded with.

use cyberapi_core::Timestamp;
use cyberapi_core::cookie::{cookie_header, parse_set_cookie};
use cyberapi_core::graphql::GraphQlBody;
use cyberapi_core::graphql_schema::{INTROSPECTION_QUERY, Schema};
use cyberapi_core::grpc::{Definitions, GrpcBody, GrpcStatus, reflection};
use cyberapi_core::model::{ContentType, HttpMethod, HttpRequest, Variable, VariableCategory};
use cyberapi_core::prefs::{SendPrefs, Timeouts};
use cyberapi_core::redirect::{self, MAX_REDIRECTS};
use cyberapi_core::response::{Exchange, HttpResponse};
use cyberapi_core::send::{self, PrepareError, PreparedRequest};
use cyberapi_core::template::{FileReader, Inputs};
use cyberapi_core::{CollectionId, ItemId};
use cyberapi_http::{HttpEngine, SendError};
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
    pub sending: SendPrefs,
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
    let mut prepared = prepare(&sender, &collection, &request, picked_file, None).await?;
    let grpc = (request.method == HttpMethod::Grpc).then(|| GrpcBody::read(&request.body));
    // The jar's cookies go to each hop's own address. A Cookie the request
    // names itself stays with the origin it was written for; past it, the
    // jar's are sent.
    let from_jar = prepared.header("cookie").is_none();
    if from_jar {
        add_cookies(&sender, &mut prepared).await?;
    }
    let started = Instant::now();
    let mut earlier = Duration::ZERO;
    let mut hops = 0;
    let (sent_to, answered) = loop {
        let sent_to = prepared.url.clone();
        let answered = sender.engine.send(prepared.clone()).await;
        let Ok(exchange) = &answered else {
            break (sent_to, answered);
        };
        keep_cookies(&sender, exchange, &sent_to).await;
        let next = (sender.sending.follow_redirects && grpc.is_none())
            .then(|| redirect::next(&prepared, exchange.status, &exchange.headers))
            .flatten();
        let Some(mut next) = next else {
            break (sent_to, answered);
        };
        if hops == MAX_REDIRECTS {
            break (sent_to, Err(SendError::Redirects { count: hops }));
        }
        hops += 1;
        earlier += exchange.stats.total;
        if from_jar {
            next.headers.retain(|(name, _)| !name.eq_ignore_ascii_case("cookie"));
        }
        if next.header("cookie").is_none() {
            add_cookies(&sender, &mut next).await?;
        }
        prepared = next;
    };
    // Never zero (F5.7).
    let floor = |latency: Duration| latency.max(Duration::from_millis(1));
    let response = match answered {
        Ok(exchange) => {
            let mut headers = exchange.headers;
            headers.extend(exchange.trailers);
            // A gRPC reply is read with the definitions it was asked with.
            let (body, notice) = match &grpc {
                Some(call) => match call.reply(&exchange.body) {
                    Ok(json) => (json.map(String::into_bytes).unwrap_or_default(), None),
                    Err(why) => (exchange.body, Some(why)),
                },
                None => (exchange.body, None),
            };
            HttpResponse {
                api: api.clone(),
                collection: collection.clone(),
                request,
                url: sent_to,
                status: Some(exchange.status),
                headers,
                body,
                notice,
                error: None,
                body_size: exchange.body_size,
                // Every hop's: the stats are the last one's alone.
                latency: floor(earlier + exchange.stats.total),
                stats: exchange.stats,
                truncated: exchange.truncated,
                ..Default::default()
            }
        }
        Err(error) => HttpResponse {
            api: api.clone(),
            collection: collection.clone(),
            request,
            url: sent_to,
            error: Some(error.to_string()),
            latency: floor(started.elapsed()),
            ..Default::default()
        },
    };
    // A body too big for the history still shows now, in full (#17).
    match sender.storage.history().add(response.for_history()).await {
        Ok(kept) => Ok(HttpResponse {
            id: kept.id,
            created_at: kept.created_at,
            ..response
        }),
        // The answer still shows, with an id and a time of its own so the
        // next is not taken for it; it is only not kept (an API only in a
        // page's draft is unknown to the server).
        Err(error) => {
            warn!(%error, "the response could not be kept");
            Ok(HttpResponse {
                id: unkept_id(),
                created_at: now(),
                ..response
            })
        }
    }
}

/// A GraphQL endpoint's schema, asked of it with the introspection query:
/// the request as it is — its URL, headers, auth, the jar's cookies — with
/// that query in place of its body, as a POST. Nothing goes into the
/// history; a redirect is not followed. `Err` says why there is none.
pub async fn introspect(sender: Sender, collection: CollectionId, request: HttpRequest) -> Result<Schema, String> {
    let asking = HttpRequest {
        method: HttpMethod::Post,
        content_type: ContentType::GraphQl,
        body: GraphQlBody {
            query: INTROSPECTION_QUERY.into(),
            ..Default::default()
        }
        .to_body(),
        ..request
    };
    let mut prepared = prepare(&sender, &collection, &asking, None, None)
        .await
        .map_err(|failure| failure.to_string())?;
    if prepared.header("cookie").is_none() {
        add_cookies(&sender, &mut prepared)
            .await
            .map_err(|failure| failure.to_string())?;
    }
    let exchange = sender.engine.send(prepared).await.map_err(|error| error.to_string())?;
    match Schema::from_introspection(&exchange.body) {
        Ok(schema) => Ok(schema),
        Err(why) if !(200..300).contains(&exchange.status) => Err(format!("{} ({why})", exchange.status)),
        Err(why) => Err(why),
    }
}

/// F7.8: the cURL command of `request`, prepared as sending would prepare
/// it. An `openFile` in it is not asked: its parameter stands.
pub async fn curl(sender: Sender, collection: CollectionId, request: HttpRequest) -> Result<String, SendFailure> {
    let mut prepared = prepare(&sender, &collection, &request, None, None).await?;
    if prepared.header("cookie").is_none() {
        add_cookies(&sender, &mut prepared).await?;
    }
    Ok(cyberapi_core::curl::curl(
        &prepared,
        &request,
        &[],
        sender.sending.follow_redirects,
    ))
}

/// Why reflection at one path gave no definitions.
enum Unreflected {
    /// The server has no reflection service there: the older path may.
    Absent(String),
    Failed(String),
}

/// A gRPC server's definitions, asked of its reflection service with the
/// request's address and metadata: version 1, or `v1alpha` where the
/// server has only that. Nothing goes into the history. `Err` says why
/// there are none.
pub async fn reflect(sender: Sender, collection: CollectionId, request: HttpRequest) -> Result<Definitions, String> {
    let mut absent = String::new();
    for path in reflection::PATHS {
        match reflect_at(&sender, &collection, &request, path).await {
            Ok(definitions) => return Ok(definitions),
            Err(Unreflected::Absent(why)) => absent = why,
            Err(Unreflected::Failed(why)) => return Err(why),
        }
    }
    Err(format!("the server offers no reflection: {absent}"))
}

async fn reflect_at(
    sender: &Sender,
    collection: &CollectionId,
    request: &HttpRequest,
    path: &str,
) -> Result<Definitions, Unreflected> {
    let services = ask_reflection(sender, collection, request, path, reflection::list_services())
        .await?
        .into_iter()
        .flat_map(|reply| match reply {
            reflection::Reply::Services(names) => names,
            _ => Vec::new(),
        })
        .collect();
    let services = reflection::wanted(services);
    let mut collected = reflection::Collected::default();
    let mut body = reflection::files_containing(&services);
    // A server that answers with a file alone is asked for what it
    // imports, a few rounds at most.
    for _ in 0..4 {
        if body.is_empty() {
            break;
        }
        for reply in ask_reflection(sender, collection, request, path, body).await? {
            match reply {
                reflection::Reply::Files(files) => {
                    for file in files {
                        collected.add(&file).map_err(Unreflected::Failed)?;
                    }
                }
                reflection::Reply::Error(why) => return Err(Unreflected::Failed(why)),
                reflection::Reply::Services(_) => {}
            }
        }
        body = reflection::files_named(&collected.missing());
    }
    collected.definitions().map_err(Unreflected::Failed)
}

/// One reflection call: the answers in its reply.
async fn ask_reflection(
    sender: &Sender,
    collection: &CollectionId,
    request: &HttpRequest,
    path: &str,
    body: Vec<u8>,
) -> Result<Vec<reflection::Reply>, Unreflected> {
    let failed = |why: String| Unreflected::Failed(why);
    let mut prepared = prepare(sender, collection, request, None, Some((path, body)))
        .await
        .map_err(|failure| failed(failure.to_string()))?;
    if prepared.header("cookie").is_none() {
        add_cookies(sender, &mut prepared)
            .await
            .map_err(|failure| failed(failure.to_string()))?;
    }
    let exchange = sender
        .engine
        .send(prepared)
        .await
        .map_err(|error| failed(error.to_string()))?;
    let mut headers = exchange.headers;
    headers.extend(exchange.trailers);
    match GrpcStatus::read(&headers) {
        // UNIMPLEMENTED: no such service.
        Some(status) if status.code == 12 => Err(Unreflected::Absent(format!("{} {}", status.name(), status.message))),
        Some(status) if !status.is_ok() => Err(failed(format!("{} {}", status.name(), status.message))),
        None if exchange.status != 200 => Err(failed(format!("HTTP {}", exchange.status))),
        _ => reflection::read(&exchange.body).map_err(failed),
    }
}

/// F9.2: the answer's cookies are kept, an expired one removing its own.
async fn keep_cookies(sender: &Sender, exchange: &Exchange, sent_to: &str) {
    let Ok(url) = url::Url::parse(sent_to) else {
        return;
    };
    let cookies: Vec<_> = exchange
        .headers
        .iter()
        .filter(|(name, _)| name == "set-cookie")
        .filter_map(|(_, value)| parse_set_cookie(value, &url, now()))
        .collect();
    if !cookies.is_empty()
        && let Err(error) = sender.storage.cookies().put(cookies).await
    {
        warn!(%error, "the response's cookies could not be kept");
    }
}

/// F9.3: the jar's cookies for where `prepared` goes.
async fn add_cookies(sender: &Sender, prepared: &mut PreparedRequest) -> Result<(), SendFailure> {
    let Ok(url) = url::Url::parse(&prepared.url) else {
        return Ok(());
    };
    let jar = sender
        .storage
        .cookies()
        .list()
        .await
        .map_err(|source| SendFailure::Store { source })?;
    if let Some(cookie) = cookie_header(&jar, &url, now()) {
        prepared.headers.push(("Cookie".into(), cookie));
    }
    Ok(())
}

/// An id for an answer the history did not keep, different for each.
fn unkept_id() -> String {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    HttpResponse::unkept_id(COUNT.fetch_add(1, Ordering::Relaxed))
}

fn now() -> Timestamp {
    Timestamp::from_unix(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|since| since.as_secs() as i64)
            .unwrap_or_default(),
    )
}

/// Everything before the engine but the cookies: the variables, the
/// responses the template functions read, then
/// `cyberapi_core::send::prepare` — or, given a path and its body, the gRPC
/// call of that path at the request's address (`send::grpc_call`).
async fn prepare(
    sender: &Sender,
    collection: &CollectionId,
    request: &HttpRequest,
    picked_file: Option<String>,
    grpc_call: Option<(&str, Vec<u8>)>,
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
    let boundary = boundary();
    let cx = send::Context {
        variables: &variables,
        template: &inputs,
        files: sender.files.as_ref(),
        user_agent: &sender.user_agent,
        timeouts: sender.timeouts,
        accept_invalid_certs: sender.sending.accept_invalid_certs,
        http_version: sender.sending.http_version,
        boundary: &boundary,
    };
    match grpc_call {
        Some((path, body)) => send::grpc_call(request, &cx, path, body),
        None => send::prepare(request, &cx),
    }
    .map_err(|source| SendFailure::Prepare { source })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cyberapi_core::grpc;
    use cyberapi_core::model::{ContentType, HttpMethod, KvParam};
    use cyberapi_core::response::HttpStats;
    use cyberapi_core::template::{BaseDir, TemplateError};
    use cyberapi_http::SendFuture;
    use cyberapi_store::{MemoryStore, NewCollection, NewVariable};
    use pretty_assertions::assert_eq;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// Answers a request with its URL's route, else with `answer`, and
    /// keeps what it was sent.
    struct Engine {
        answer: Result<Exchange, SendError>,
        routes: Mutex<HashMap<String, Exchange>>,
        sent: Mutex<Vec<PreparedRequest>>,
    }

    impl HttpEngine for Engine {
        fn send(&self, request: PreparedRequest) -> SendFuture {
            let routed = self.routes.lock().expect("routes").get(&request.url).cloned();
            if let Ok(mut sent) = self.sent.lock() {
                sent.push(request);
            }
            let answer = routed.map(Ok).unwrap_or_else(|| self.answer.clone());
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
            routes: Mutex::default(),
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
            sending: SendPrefs::default(),
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

    /// The schema is asked of the API's endpoint as the API is sent — its
    /// environment, its headers — and kept nowhere.
    #[test]
    fn a_schema_is_asked_for_as_the_api_would_be_sent() {
        let answer = r#"{"data": {"__schema": {"queryType": {"name": "Query"}, "types": [
            {"kind": "OBJECT", "name": "Query", "fields": [{"name": "me", "args": [], "type": {"kind": "SCALAR", "name": "ID"}}]}
        ]}}}"#;
        let setup = setup(ok(200, answer));
        variable(&setup, VariableCategory::Environment, "HOST", "https://api.test");
        let api = new_api(&setup, "me");
        let request = HttpRequest {
            method: HttpMethod::Get,
            uri: "{{HOST}}/graphql".into(),
            headers: vec![kv("Authorization", "Bearer t")],
            content_type: ContentType::GraphQl,
            body: GraphQlBody {
                query: "{ me }".into(),
                ..Default::default()
            }
            .to_body(),
            ..Default::default()
        };
        let schema =
            pollster::block_on(introspect(setup.sender.clone(), setup.collection.clone(), request)).expect("a schema");
        assert_eq!(schema.query.as_deref(), Some("Query"));
        let sent = setup.engine.sent.lock().expect("sent").clone();
        assert_eq!(
            (sent[0].method, sent[0].url.as_str()),
            (HttpMethod::Post, "https://api.test/graphql")
        );
        assert_eq!(sent[0].header("authorization"), Some("Bearer t"));
        let body: serde_json::Value = serde_json::from_slice(&sent[0].body).expect("JSON");
        assert_eq!(body["query"], serde_json::json!(INTROSPECTION_QUERY));
        let history = pollster::block_on(setup.storage.history().list(setup.collection.clone(), api)).expect("list");
        assert!(history.is_empty(), "not kept");

        let refused = setup_refusing();
        let request = HttpRequest {
            uri: "https://api.test/graphql".into(),
            ..Default::default()
        };
        let why = pollster::block_on(introspect(refused.sender.clone(), refused.collection.clone(), request));
        assert_eq!(why, Err("introspection is off".into()));
    }

    /// `package shop; message Ask { string id = 1; }
    /// service Store { rpc Get(Ask) returns (Ask); }`, encoded.
    fn shop_proto() -> Vec<u8> {
        use prost::Message as _;
        use prost_types::field_descriptor_proto::{Label, Type};
        prost_types::FileDescriptorProto {
            name: Some("shop.proto".into()),
            package: Some("shop".into()),
            syntax: Some("proto3".into()),
            message_type: vec![prost_types::DescriptorProto {
                name: Some("Ask".into()),
                field: vec![prost_types::FieldDescriptorProto {
                    name: Some("id".into()),
                    number: Some(1),
                    r#type: Some(Type::String as i32),
                    label: Some(Label::Optional as i32),
                    json_name: Some("id".into()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            service: vec![prost_types::ServiceDescriptorProto {
                name: Some("Store".into()),
                method: vec![prost_types::MethodDescriptorProto {
                    name: Some("Get".into()),
                    input_type: Some(".shop.Ask".into()),
                    output_type: Some(".shop.Ask".into()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }
        .encode_to_vec()
    }

    /// A length-delimited field `tag` holding `inner`.
    fn field(tag: u32, inner: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        prost::encoding::encode_key(tag, prost::encoding::WireType::LengthDelimited, &mut out);
        prost::encoding::encode_varint(inner.len() as u64, &mut out);
        out.extend_from_slice(inner);
        out
    }

    /// A gRPC answer: `body` and the trailers with `status`.
    fn grpc_answer(body: Vec<u8>, status: &str, message: &str) -> Exchange {
        Exchange {
            status: 200,
            headers: vec![("content-type".into(), "application/grpc".into())],
            body_size: body.len() as u64,
            body,
            trailers: vec![
                ("grpc-status".into(), status.into()),
                ("grpc-message".into(), message.into()),
            ],
            ..Default::default()
        }
    }

    /// Reflection is asked at v1, then at v1alpha where the server has only
    /// that; the definitions it gives encode a call, sent as HTTP/2 to the
    /// method's path, whose reply is kept read as JSON.
    #[test]
    fn a_grpc_call_is_made_with_the_definitions_reflection_gives() {
        let setup = setup(Ok(grpc_answer(Vec::new(), "14", "down")));
        variable(&setup, VariableCategory::Environment, "SHOP", "grpc://shop.test:50051");
        let request = HttpRequest {
            method: HttpMethod::Grpc,
            uri: "{{SHOP}}".into(),
            headers: vec![kv("authorization", "Bearer t")],
            ..Default::default()
        };
        let [v1, v1alpha] = reflection::PATHS.map(|path| format!("http://shop.test:50051{path}"));
        let services = grpc::frame(&field(6, &field(1, &field(1, b"shop.Store"))));
        let files = grpc::frame(&field(4, &field(1, &shop_proto())));
        {
            let mut routes = setup.engine.routes.lock().expect("routes");
            routes.insert(v1.clone(), grpc_answer(Vec::new(), "12", "unknown service"));
            routes.insert(v1alpha.clone(), grpc_answer([services, files].concat(), "0", ""));
        }
        let definitions = pollster::block_on(reflect(setup.sender.clone(), setup.collection.clone(), request.clone()))
            .expect("definitions");
        assert_eq!(
            definitions
                .methods()
                .iter()
                .map(|m| m.method.as_str())
                .collect::<Vec<_>>(),
            ["Get"]
        );
        let asked: Vec<_> = setup.engine.sent.lock().expect("sent").drain(..).collect();
        assert_eq!(
            asked.iter().map(|sent| sent.url.as_str()).collect::<Vec<_>>(),
            [v1.as_str(), v1alpha.as_str(), v1alpha.as_str()],
            "v1, then v1alpha's services and their files"
        );
        assert!(asked.iter().all(|sent| sent.header("authorization") == Some("Bearer t")
            && sent.http_version == cyberapi_core::prefs::HttpVersion::Http2));

        // The call, and its reply read as JSON.
        let api = new_api(&setup, "get");
        let call = HttpRequest {
            body: GrpcBody {
                service: "shop.Store".into(),
                method: "Get".into(),
                message: r#"{"id": "7"}"#.into(),
                descriptors: definitions.kept_for("shop.Store").expect("kept"),
                source: grpc::FROM_REFLECTION.into(),
            }
            .to_body(),
            ..request
        };
        let get = "http://shop.test:50051/shop.Store/Get".to_string();
        let reply = grpc::frame(&field(1, b"8"));
        setup
            .engine
            .routes
            .lock()
            .expect("routes")
            .insert(get.clone(), grpc_answer(reply, "0", ""));
        let send_it = |call: &HttpRequest| {
            pollster::block_on(send(
                setup.sender.clone(),
                setup.collection.clone(),
                api.clone(),
                call.clone(),
                None,
            ))
            .expect("sent")
        };
        let response = send_it(&call);
        assert_eq!(String::from_utf8_lossy(&response.body), "{\n    \"id\": \"8\"\n}");
        assert_eq!(response.body_kind(), cyberapi_core::response::BodyKind::Json);
        assert_eq!(response.grpc_status().map(|status| status.name()), Some("OK"));
        assert_eq!(
            (response.size(), response.compression()),
            (8, None),
            "as it came: read, not inflated"
        );
        let sent = setup.engine.sent.lock().expect("sent").pop().expect("sent");
        assert_eq!((sent.method, sent.url.as_str()), (HttpMethod::Post, get.as_str()));
        assert_eq!(sent.body, grpc::frame(&field(1, b"7")));

        // A call that failed: its status, no message.
        setup.engine.routes.lock().expect("routes").remove(&get);
        let failed = send_it(&call);
        let status = failed.grpc_status().expect("a status");
        assert_eq!((status.name(), status.message.as_str()), ("UNAVAILABLE", "down"));
        assert!(failed.body.is_empty() && failed.notice.is_none());

        // A reply the definitions do not read is kept as it came, with why.
        setup
            .engine
            .routes
            .lock()
            .expect("routes")
            .insert(get, grpc_answer(grpc::frame(&[0xff]), "0", ""));
        let unread = send_it(&call);
        assert_eq!(unread.body, grpc::frame(&[0xff]));
        assert!(unread.notice.is_some_and(|why| why.contains("not a shop.Ask")));
    }

    fn setup_refusing() -> Setup {
        setup(ok(200, r#"{"errors": [{"message": "introspection is off"}]}"#))
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
        assert!(first.kept());

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

        // The history keeps the request as the API held it, and where it
        // went.
        assert_eq!(second.request, request);
        assert_eq!(second.url, "https://api.test/me");
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

    fn redirect(to: &str, cookie: Option<&str>) -> Exchange {
        let mut headers = vec![("location".to_string(), to.to_string())];
        headers.extend(cookie.map(|cookie| ("set-cookie".to_string(), cookie.to_string())));
        Exchange {
            status: 302,
            headers,
            stats: HttpStats {
                total: Duration::from_millis(30),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn route(setup: &Setup, url: &str, exchange: Exchange) {
        setup.engine.routes.lock().expect("routes").insert(url.into(), exchange);
    }

    fn send_to(setup: &Setup, method: HttpMethod, uri: &str) -> HttpResponse {
        let api = new_api(setup, "a");
        pollster::block_on(send(
            setup.sender.clone(),
            setup.collection.clone(),
            api,
            HttpRequest {
                method,
                uri: uri.into(),
                content_type: ContentType::Json,
                body: "{}".into(),
                ..Default::default()
            },
            None,
        ))
        .expect("sent")
    }

    #[test]
    fn redirects_are_answers_unless_followed() {
        let mut setup = setup(Ok(Exchange {
            status: 200,
            body: b"me".to_vec(),
            stats: HttpStats {
                total: Duration::from_millis(20),
                ..Default::default()
            },
            ..Default::default()
        }));
        route(&setup, "https://api.test/login", redirect("/me", Some("sid=1; Path=/")));
        let answer = send_to(&setup, HttpMethod::Post, "https://api.test/login");
        assert_eq!(answer.status, Some(302), "not followed by default");
        assert_eq!(setup.engine.sent.lock().expect("sent").len(), 1);

        setup.sender.sending.follow_redirects = true;
        let answer = send_to(&setup, HttpMethod::Post, "https://api.test/login");
        assert_eq!(answer.status, Some(200));
        assert_eq!(answer.body, b"me".to_vec());
        assert_eq!(answer.url, "https://api.test/me", "where the answer came from");
        assert_eq!(answer.request.method, HttpMethod::Post, "the request kept is the API's");
        assert_eq!(answer.latency, Duration::from_millis(50), "every hop's time");
        let sent = setup.engine.sent.lock().expect("sent").clone();
        let me = &sent[2];
        assert_eq!((me.method, me.body.is_empty()), (HttpMethod::Get, true));
        assert_eq!(
            me.header("cookie"),
            Some("sid=1"),
            "the first hop's cookie goes to the next"
        );
        let kept = pollster::block_on(
            setup
                .storage
                .history()
                .list(setup.collection.clone(), answer.api.clone()),
        )
        .expect("history");
        assert_eq!(kept.len(), 1, "one answer per send, not one per hop");
    }

    #[test]
    fn each_hop_gets_the_jars_cookies_for_where_it_goes() {
        let mut setup = setup(ok(200, "{}"));
        setup.sender.sending.follow_redirects = true;
        pollster::block_on(setup.storage.cookies().put(vec![
            parse_set_cookie("a=1", &url::Url::parse("https://a.test/").expect("url"), now()).expect("cookie"),
            parse_set_cookie("b=2", &url::Url::parse("https://b.test/").expect("url"), now()).expect("cookie"),
        ]))
        .expect("cookies");
        route(&setup, "https://a.test/", redirect("https://b.test/", None));
        route(&setup, "https://a.test/mine", redirect("/", None));
        send_to(&setup, HttpMethod::Get, "https://a.test/");
        let sent = setup.engine.sent.lock().expect("sent").clone();
        assert_eq!(sent[0].header("cookie"), Some("a=1"));
        assert_eq!(sent[1].header("cookie"), Some("b=2"));

        // The request's own Cookie stays with its origin, then the jar's go.
        let api = new_api(&setup, "mine");
        pollster::block_on(send(
            setup.sender.clone(),
            setup.collection.clone(),
            api,
            HttpRequest {
                uri: "https://a.test/mine".into(),
                headers: vec![kv("Cookie", "mine=1")],
                ..Default::default()
            },
            None,
        ))
        .expect("sent");
        let sent = setup.engine.sent.lock().expect("sent").clone();
        let cookies: Vec<_> = sent[2..].iter().map(|request| request.header("cookie")).collect();
        assert_eq!(cookies, vec![Some("mine=1"), Some("mine=1"), Some("b=2")]);
    }

    #[test]
    fn a_redirect_loop_stops() {
        let mut setup = setup(ok(200, "{}"));
        setup.sender.sending.follow_redirects = true;
        route(&setup, "https://api.test/a", redirect("/b", None));
        route(&setup, "https://api.test/b", redirect("/a", None));
        let answer = send_to(&setup, HttpMethod::Get, "https://api.test/a");
        assert_eq!(answer.status, None);
        assert_eq!(answer.error.as_deref(), Some("stopped after 10 redirects"));
        assert_eq!(setup.engine.sent.lock().expect("sent").len(), MAX_REDIRECTS + 1);
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
        assert_eq!(response.url, "http://down.test/", "where it was going");
        assert!(response.latency >= Duration::from_millis(1));
        let history = pollster::block_on(setup.storage.history().list(setup.collection.clone(), api)).expect("list");
        assert_eq!(history.len(), 1);
    }

    /// #17: a body too big for the history shows in full, and the history
    /// keeps its size.
    #[test]
    fn a_big_answer_shows_whole_and_is_kept_without_its_body() {
        let big = "x".repeat(cyberapi_core::response::HISTORY_BODY_LIMIT + 10);
        let setup = setup(ok(200, &big));
        let api = new_api(&setup, "export");
        let response = pollster::block_on(send(
            setup.sender.clone(),
            setup.collection.clone(),
            api.clone(),
            HttpRequest {
                uri: "http://big.test".into(),
                ..Default::default()
            },
            None,
        ))
        .expect("an answer");
        assert_eq!(response.body.len(), big.len());
        assert!(!response.id.is_empty(), "the id the history gave");
        let history = pollster::block_on(setup.storage.history().list(setup.collection.clone(), api)).expect("list");
        assert_eq!(history[0].id, response.id);
        assert!(history[0].body.is_empty());
        assert_eq!(history[0].body_omitted, Some(big.len() as u64));
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

    #[test]
    fn an_answer_the_history_cannot_keep_is_still_one_of_its_own() {
        // An API the store does not know, as one only in a page's draft is
        // to the server.
        let setup = setup(ok(200, "{}"));
        let before = now();
        let answer = || {
            pollster::block_on(send(
                setup.sender.clone(),
                setup.collection.clone(),
                "only-in-a-draft".into(),
                HttpRequest {
                    uri: "http://a.test".into(),
                    ..Default::default()
                },
                None,
            ))
            .expect("sent")
        };
        let (first, second) = (answer(), answer());
        // Told apart, as the panel and the history menu tell them; and
        // known not to be kept, as the panel says.
        assert!(!first.id.is_empty());
        assert_ne!(first.id, second.id);
        assert!(!first.kept() && !second.kept());
        assert!(first.created_at >= before, "{:?}", first.created_at);
    }
}
