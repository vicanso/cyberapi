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

use crate::oauth::{self, Signed, Tokens};
use cyberapi_core::Timestamp;
use cyberapi_core::certificates;
use cyberapi_core::code::CodeTarget;
use cyberapi_core::cookie::{cookie_header, parse_set_cookie};
use cyberapi_core::digest::{self, Unanswered as DigestUnanswered};
use cyberapi_core::environments;
use cyberapi_core::graphql::GraphQlBody;
use cyberapi_core::graphql_schema::{INTROSPECTION_QUERY, Schema};
use cyberapi_core::grpc::{Definitions, GrpcBody, GrpcStatus, reflection};
use cyberapi_core::inherit::Inherited;
use cyberapi_core::model::{ContentType, Environment, HttpMethod, HttpRequest, Variable, VariableCategory};
use cyberapi_core::prefs::{SendPrefs, Timeouts};
use cyberapi_core::redirect::{self, MAX_REDIRECTS};
use cyberapi_core::response::{Exchange, HttpResponse};
use cyberapi_core::secrets::{self, SecretValues};
use cyberapi_core::send::{self, PrepareError, PreparedRequest};
use cyberapi_core::template::{Answers, Draws, Env, EnvReader, FileReader, Inputs};
use cyberapi_core::{CollectionId, ItemId};
use cyberapi_http::{HttpEngine, SendError, Watch};
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
    /// D1: a `prompt` nothing was typed for — an AI assistant's send, which
    /// no one is asked — and the request does not go with a stand-in.
    #[snafu(display("it asks for {} as it is sent, and nothing was typed for it", labels.join(", ")))]
    Unanswered { labels: Vec<String> },
    /// E1: a WebSocket API is connected to (`crate::socket`), not sent: a
    /// run, the command line and an AI assistant pass it by.
    #[snafu(display("a WebSocket API is connected to in the desktop app, not sent"))]
    WebSocket,
    /// E3: a GraphQL subscription whose address is `ws` or `wss` is
    /// connected to (`crate::graphql_ws`), not sent.
    #[snafu(display("a GraphQL subscription over WebSocket is connected to in the desktop app, not sent"))]
    GraphQlSocket,
    /// E2: a gRPC call that streams from the client is connected to
    /// (`crate::grpc_stream`), not sent. A run, the command line and an
    /// assistant pass it by.
    #[snafu(display("a gRPC call that streams from the client is connected to in the desktop app, not sent"))]
    GrpcStream,
}

/// What a `prompt` nothing was typed for does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unanswered {
    /// Nothing goes out.
    Stops,
    /// Its start, or else its label, stands: in the request's code.
    Stands,
}

/// What sending needs besides the request: where to send it from, and the
/// reader's settings.
#[derive(Clone)]
pub struct Sender {
    pub storage: Storage,
    pub engine: Arc<dyn HttpEngine>,
    /// For `readFile` and multipart files; the browser build has none.
    pub files: Arc<dyn FileReader + Send + Sync>,
    /// For `env` (D2): the desktop's variables; a page and the bridge have
    /// none.
    pub env: Arc<dyn EnvReader + Send + Sync>,
    pub user_agent: String,
    pub timeouts: Timeouts,
    pub sending: SendPrefs,
    /// What the API sent takes from its folders and its project (#66). The
    /// caller fills it from the tree it holds (`inherit::inherited`):
    /// reading the tree here for every send would cost the page a trip to
    /// the bridge.
    pub inherited: Inherited,
    /// The named environment it is sent in (#61), the reader's: its
    /// variables stand in for the shared ones of the same name. The caller
    /// fills it, as `inherited`.
    pub environment: Option<Environment>,
    /// C4: the OAuth tokens had so far, shared by every send of the app
    /// (or of the run, the call) that holds them.
    pub tokens: Arc<Tokens>,
    /// D3: the reader's values of the project's secret variables, filled
    /// in as it is sent and masked in what the history keeps. The caller
    /// fills it, as `inherited`.
    pub secrets: SecretValues,
}

impl Sender {
    /// The sender of `request`: its own sending settings in place of the
    /// settings' where it has them (#71). Every way a request goes out
    /// starts here.
    pub fn for_request(&self, request: &HttpRequest) -> Self {
        Self {
            timeouts: request.settings.timeouts_over(self.timeouts),
            sending: request.settings.sending_over(&self.sending),
            ..self.clone()
        }
    }
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
/// answer. `answers` are the reader's to the `openFile` and the `prompt`s
/// the request needs (`cyberapi_core::send::needs` says so), asked for
/// beforehand: a prompt not answered stops it.
pub async fn send(
    sender: Sender,
    collection: CollectionId,
    api: ItemId,
    request: HttpRequest,
    answers: Answers,
) -> Result<HttpResponse, SendFailure> {
    send_watched(sender, collection, api, request, answers, None).await
}

/// As [`send`]; an answer that is an event stream is told to `watch` as it
/// comes (`HttpEngine::send_watched`), and kept with how it came once it
/// ends or is stopped.
pub async fn send_watched(
    sender: Sender,
    collection: CollectionId,
    api: ItemId,
    request: HttpRequest,
    answers: Answers,
    watch: Option<Watch>,
) -> Result<HttpResponse, SendFailure> {
    send_with(sender, collection, api, request, answers, watch, &[]).await
}

/// As [`send_watched`]; a run's `values` (A3, `cyberapi_core::run`) stand
/// for the project's variables of the same names.
pub(crate) async fn send_with(
    sender: Sender,
    collection: CollectionId,
    api: ItemId,
    request: HttpRequest,
    answers: Answers,
    watch: Option<Watch>,
    values: &[(String, String)],
) -> Result<HttpResponse, SendFailure> {
    if request.method == HttpMethod::WebSocket {
        return Err(SendFailure::WebSocket);
    }
    // E2: a client or bidirectional stream is opened by the desktop, one
    // message at a time. A run, the command line and an assistant are told so.
    if request.method == HttpMethod::Grpc && GrpcBody::read(&request.body).client_streams() {
        return Err(SendFailure::GrpcStream);
    }
    let sender = sender.for_request(&request);
    let (mut prepared, globals) = prepare(
        &sender,
        &collection,
        &request,
        &answers,
        Unanswered::Stops,
        None,
        values,
    )
    .await?;
    // E3: a subscription whose address is a WebSocket is opened by the
    // desktop, not sent from here. A run, the command line and an assistant
    // are told so.
    if request.content_type == ContentType::GraphQl && cyberapi_core::graphql_ws::is_socket_url(&prepared.url) {
        return Err(SendFailure::GraphQlSocket);
    }
    let grpc = (request.method == HttpMethod::Grpc).then(|| GrpcBody::read(&request.body));
    // The jar's cookies go to each hop's own address. A Cookie the request
    // names itself stays with the origin it was written for; past it, the
    // jar's are sent.
    let from_jar = prepared.header("cookie").is_none();
    let own_cookie = prepared.header("cookie").map(str::to_string);
    if from_jar {
        add_cookies(&sender, &mut prepared).await?;
    }
    // C4: no token, no request: why is kept as its answer.
    let (mut oauth_signed, mut no_token) = match oauth::sign(&sender, &mut prepared, false).await {
        Ok(signed) => (signed, None),
        Err(why) => (Signed::No, Some(why)),
    };
    let started = Instant::now();
    let mut earlier = Duration::ZERO;
    let mut hops = 0;
    // C2: this hop's Authorization is the answer to its Digest challenge;
    // why one was not answered.
    let mut answered_digest = false;
    let mut digest_notice = None;
    let (sent, answered) = loop {
        if let Some(why) = no_token.take() {
            break (prepared, Err(SendError::Http { message: why }));
        }
        let sent_to = prepared.url.clone();
        let answered = match &watch {
            Some(watch) => sender.engine.send_watched(prepared.clone(), watch.clone()).await,
            None => sender.engine.send(prepared.clone()).await,
        };
        let Ok(exchange) = &answered else {
            break (prepared, answered);
        };
        keep_cookies(&sender, exchange, &sent_to).await;
        // C4: a kept token the API no longer takes: once more, with a new
        // one.
        if exchange.status == 401 && oauth_signed == Signed::Kept {
            match oauth::sign(&sender, &mut prepared, true).await {
                Ok(signed) => {
                    oauth_signed = signed;
                    earlier += exchange.stats.total;
                    continue;
                }
                Err(why) => break (prepared, Err(SendError::Http { message: why })),
            }
        }
        match answer_digest(&prepared, exchange) {
            Some(Ok(signed)) => {
                answered_digest = true;
                earlier += exchange.stats.total;
                prepared = signed;
                continue;
            }
            Some(Err(why)) => {
                digest_notice = Some(why);
                break (prepared, answered);
            }
            None => {}
        }
        let next = (sender.sending.follow_redirects && grpc.is_none())
            .then(|| redirect::next(&prepared, exchange.status, &exchange.headers))
            .flatten();
        let Some(mut next) = next else {
            break (prepared, answered);
        };
        if hops == MAX_REDIRECTS {
            break (prepared, Err(SendError::Redirects { count: hops }));
        }
        hops += 1;
        earlier += exchange.stats.total;
        // Its nonce and its URI were this hop's: the next is asked anew.
        if answered_digest {
            next.headers
                .retain(|(name, _)| !name.eq_ignore_ascii_case("authorization"));
            answered_digest = false;
        }
        // C5: the certificate for where it goes now.
        next.tls = certificates::tls_for(&sender.sending.certificates, &next.url);
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
    // #58: the last hop as it went. Its cookies are the jar's unless they
    // are the ones the request names itself.
    let version = answered
        .as_ref()
        .map(|exchange| exchange.stats.version.as_str())
        .unwrap_or_default();
    let jar = sent.header("cookie").is_some() && sent.header("cookie") != own_cookie.as_deref();
    // D3: kept with the reader's secret values masked.
    let sent_request = secrets::mask_sent(
        send::sent_request(&sent, &request, &sender.inherited, &globals, jar, version),
        &sender.secrets,
    );
    let sent_to = secrets::mask(&sent.url, &sender.secrets);
    let response = match answered {
        Ok(exchange) => {
            let mut headers = exchange.headers;
            headers.extend(exchange.trailers);
            // A gRPC reply is read with the definitions it was asked with: a
            // streamed one a message a line, with when each came.
            let (body, notice, stream) = match &grpc {
                Some(call) if call.server_streams() => {
                    match call.stream_reply(&exchange.body, exchange.stream.as_ref()) {
                        Ok((body, stream)) => (body, None, stream),
                        Err(why) => (exchange.body, Some(why), exchange.stream),
                    }
                }
                Some(call) => match call.reply(&exchange.body) {
                    Ok(json) => (json.map(String::into_bytes).unwrap_or_default(), None, exchange.stream),
                    Err(why) => (exchange.body, Some(why), exchange.stream),
                },
                None => (exchange.body, digest_notice, exchange.stream),
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
                stream,
                sent: Some(sent_request),
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
            sent: Some(sent_request),
            ..Default::default()
        },
    };
    Ok(kept(&sender, response).await)
}

/// `response` in the API's history, with the id and time it was kept
/// under. A body too big for the history still shows now, in full (#17).
pub(crate) async fn kept(sender: &Sender, response: HttpResponse) -> HttpResponse {
    match sender
        .storage
        .history()
        .add(response.for_history(), sender.sending.history_size())
        .await
    {
        Ok(kept) => HttpResponse {
            id: kept.id,
            created_at: kept.created_at,
            ..response
        },
        // The answer still shows, with an id and a time of its own so the
        // next is not taken for it; it is only not kept (an API only in a
        // page's draft is unknown to the server).
        Err(error) => {
            warn!(%error, "the response could not be kept");
            HttpResponse {
                id: unkept_id(),
                created_at: now(),
                ..response
            }
        }
    }
}

/// A GraphQL endpoint's schema, asked of it with the introspection query:
/// the request as it is — its URL, headers, auth, the jar's cookies — with
/// that query in place of its body, as a POST. Nothing goes into the
/// history; a redirect is not followed. `answers` are what the reader typed
/// for its `prompt`s when it was last sent. `Err` says why there is none.
pub async fn introspect(
    sender: Sender,
    collection: CollectionId,
    request: HttpRequest,
    answers: Answers,
) -> Result<Schema, String> {
    let sender = sender.for_request(&request);
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
    let (mut prepared, _) = prepare(&sender, &collection, &asking, &answers, Unanswered::Stops, None, &[])
        .await
        .map_err(|failure| failure.to_string())?;
    if prepared.header("cookie").is_none() {
        add_cookies(&sender, &mut prepared)
            .await
            .map_err(|failure| failure.to_string())?;
    }
    oauth::sign(&sender, &mut prepared, false).await?;
    let mut exchange = sender
        .engine
        .send(prepared.clone())
        .await
        .map_err(|error| error.to_string())?;
    if let Some(Ok(signed)) = answer_digest(&prepared, &exchange) {
        exchange = sender.engine.send(signed).await.map_err(|error| error.to_string())?;
    }
    match Schema::from_introspection(&exchange.body) {
        Ok(schema) => Ok(schema),
        Err(why) if !(200..300).contains(&exchange.status) => Err(format!("{} ({why})", exchange.status)),
        Err(why) => Err(why),
    }
}

/// B3: what `url` answers to a GET, as text, for an import: with the
/// settings' timeouts, certificate check and proxy, redirects followed
/// whatever the settings say, no cookies, nothing kept. `Err` says why there
/// is none: no address, a failure, an answer that is no 2xx, or one too big
/// to read whole.
pub async fn fetch_text(sender: Sender, url: String) -> Result<String, String> {
    let url = url.trim().to_string();
    if !matches!(url::Url::parse(&url), Ok(parsed) if matches!(parsed.scheme(), "http" | "https")) {
        return Err(format!("{url} is no http or https address"));
    }
    let mut prepared = PreparedRequest {
        method: HttpMethod::Get,
        url,
        headers: vec![
            ("User-Agent".into(), sender.user_agent.clone()),
            ("Accept".into(), "*/*".into()),
        ],
        timeouts: sender.timeouts.effective(),
        accept_invalid_certs: sender.sending.accept_invalid_certs,
        http_version: sender.sending.http_version,
        proxy: sender.sending.proxy.clone(),
        ..Default::default()
    };
    for _ in 0..=MAX_REDIRECTS {
        prepared.tls = certificates::tls_for(&sender.sending.certificates, &prepared.url);
        let exchange = sender
            .engine
            .send(prepared.clone())
            .await
            .map_err(|error| error.to_string())?;
        if let Some(next) = redirect::next(&prepared, exchange.status, &exchange.headers) {
            prepared = next;
            continue;
        }
        if !(200..300).contains(&exchange.status) {
            return Err(format!("{} answered {}", prepared.url, exchange.status));
        }
        if exchange.truncated {
            return Err(format!("{} is larger than can be read", prepared.url));
        }
        return Ok(String::from_utf8_lossy(&exchange.body).into_owned());
    }
    Err(format!("stopped after {MAX_REDIRECTS} redirects"))
}

/// F7.8 / #69: `request` as code — a cURL command, or the code of
/// another client — prepared as sending would prepare it. An `openFile` in
/// it is not asked: its parameter stands. A `prompt` takes what was typed
/// for it last (`answers`), or else its start or its label stands.
pub async fn code(
    sender: Sender,
    collection: CollectionId,
    request: HttpRequest,
    answers: Answers,
    target: CodeTarget,
) -> Result<String, SendFailure> {
    let sender = sender.for_request(&request);
    let (mut prepared, _) = prepare(&sender, &collection, &request, &answers, Unanswered::Stands, None, &[]).await?;
    if prepared.header("cookie").is_none() {
        add_cookies(&sender, &mut prepared).await?;
    }
    // C4: no token is asked for to write code.
    oauth::sign_kept(&sender, &mut prepared).await;
    Ok(cyberapi_core::code::code(
        target,
        &prepared,
        &request,
        sender.sending.follow_redirects,
    ))
}

/// C2: `prepared` again, with the answer to the Digest challenge of
/// `exchange` — a 401 to a request that signs in with Digest and has not
/// answered one yet. `None` when there is none to answer; `Err` says why a
/// challenge is not answered.
fn answer_digest(prepared: &PreparedRequest, exchange: &Exchange) -> Option<Result<PreparedRequest, String>> {
    let login = prepared.digest.as_ref()?;
    if exchange.status != 401 || prepared.header("authorization").is_some() {
        return None;
    }
    let challenge = match digest::challenge(&exchange.headers) {
        Ok(challenge) => challenge,
        Err(DigestUnanswered::NotAsked) => return None,
        Err(DigestUnanswered::Unsupported(what)) => {
            return Some(Err(format!(
                "The server asks for Digest with {what}, which CyberAPI does not answer."
            )));
        }
    };
    let url = url::Url::parse(&prepared.url).ok()?;
    let uri = match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().to_string(),
    };
    let cnonce = format!(
        "{:016x}{:016x}",
        getrandom::u64().unwrap_or_default(),
        getrandom::u64().unwrap_or_else(|_| now_nanos())
    );
    let mut signed = prepared.clone();
    signed.headers.push((
        "Authorization".into(),
        digest::authorization(&challenge, prepared.method.as_str(), &uri, login, &cnonce),
    ));
    Some(Ok(signed))
}

fn now_nanos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_nanos() as u64)
        .unwrap_or_default()
}

/// Why reflection at one path gave no definitions.
enum Unreflected {
    /// The server has no reflection service there: the older path may.
    Absent(String),
    Failed(String),
}

/// A gRPC server's definitions, asked of its reflection service with the
/// request's address and metadata: version 1, or `v1alpha` where the
/// server has only that. Nothing goes into the history. `answers` are what
/// the reader typed for its `prompt`s when it was last sent. `Err` says why
/// there are none.
pub async fn reflect(
    sender: Sender,
    collection: CollectionId,
    request: HttpRequest,
    answers: Answers,
) -> Result<Definitions, String> {
    let sender = sender.for_request(&request);
    let mut absent = String::new();
    for path in reflection::PATHS {
        match reflect_at(&sender, &collection, &request, &answers, path).await {
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
    answers: &Answers,
    path: &str,
) -> Result<Definitions, Unreflected> {
    let services = ask_reflection(sender, collection, request, answers, path, reflection::list_services())
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
        for reply in ask_reflection(sender, collection, request, answers, path, body).await? {
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
    answers: &Answers,
    path: &str,
    body: Vec<u8>,
) -> Result<Vec<reflection::Reply>, Unreflected> {
    let failed = |why: String| Unreflected::Failed(why);
    let (mut prepared, _) = prepare(
        sender,
        collection,
        request,
        answers,
        Unanswered::Stops,
        Some((path, body)),
        &[],
    )
    .await
    .map_err(|failure| failed(failure.to_string()))?;
    if prepared.header("cookie").is_none() {
        add_cookies(sender, &mut prepared)
            .await
            .map_err(|failure| failed(failure.to_string()))?;
    }
    oauth::sign(sender, &mut prepared, false).await.map_err(failed)?;
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
pub(crate) async fn keep_cookies(sender: &Sender, exchange: &Exchange, sent_to: &str) {
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
pub(crate) async fn add_cookies(sender: &Sender, prepared: &mut PreparedRequest) -> Result<(), SendFailure> {
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
pub(crate) async fn prepare(
    sender: &Sender,
    collection: &CollectionId,
    request: &HttpRequest,
    answers: &Answers,
    unanswered: Unanswered,
    grpc_call: Option<(&str, Vec<u8>)>,
    values: &[(String, String)],
) -> Result<(PreparedRequest, Vec<String>), SendFailure> {
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

    // D3: the reader's values of the secret ones; #61: the shared
    // variables, and the environment's in their place; D5: the folders'
    // and the API's own in theirs; A3: a run's in all of theirs.
    let variables = secrets::fill(variables, &sender.secrets);
    let variables = environments::effective(&variables, sender.environment.as_ref().map(|found| &found.id));
    let values = sender.inherited.values(&request.variables, values);
    let variables = if values.is_empty() {
        variables
    } else {
        cyberapi_core::run::with_values(&variables, &values)
    };
    let needs = send::needs(request, &variables, &sender.inherited);
    let missing = answers.unanswered(&needs.prompts);
    if unanswered == Unanswered::Stops && !missing.is_empty() {
        return Err(SendFailure::Unanswered {
            labels: missing.into_iter().map(str::to_string).collect(),
        });
    }
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
    let since_epoch = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let seed = getrandom::u64().unwrap_or(since_epoch.as_nanos() as u64);
    let inputs = Inputs {
        now: since_epoch.as_secs() as i64,
        custom: variables
            .iter()
            .filter(|v| v.enabled && v.category == VariableCategory::Custom)
            .map(|v| (v.name.clone(), v.value.clone()))
            .collect(),
        responses: responses
            .iter()
            .filter_map(|response| Some((response.api.to_string(), response.body_json()?)))
            .collect(),
        answers: answers.clone(),
        env: if needs.env.is_empty() {
            Env::default()
        } else {
            sender.env.read(&needs.env)
        },
        utc_offset: chrono::Local::now().offset().local_minus_utc(),
        // The clock, should the system have no randomness to give.
        draws: Draws::new(seed),
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
        proxy: &sender.sending.proxy,
        boundary: &boundary,
        inherited: &sender.inherited,
        environment: sender.environment.as_ref().map(|found| found.name.as_str()),
    };
    let mut prepared = match grpc_call {
        Some((path, body)) => send::grpc_call(request, &cx, path, body),
        None => send::prepare(request, &cx),
    }
    .map_err(|source| SendFailure::Prepare { source })?;
    prepared.tls = certificates::tls_for(&sender.sending.certificates, &prepared.url);
    Ok((prepared, send::global_header_names(&variables)))
}

/// A7b: what a load test gathers once before its sends, so dynamic template
/// functions (`uuid`, `random`, `randomString`, `now`, `timestamp`) can be
/// evaluated afresh for each send without querying storage again.
pub(crate) struct LoadTemplate {
    pub variables: Vec<Variable>,
    pub inputs: Inputs,
    pub dynamic: bool,
    pub seed: u64,
}

impl LoadTemplate {
    pub fn prepare_again(
        &self,
        sender: &Sender,
        request: &HttpRequest,
        turn: u32,
    ) -> Result<PreparedRequest, SendFailure> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let seed = self
            .seed
            .wrapping_add(u64::from(turn).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let inputs = Inputs {
            now,
            draws: Draws::new(seed),
            ..self.inputs.clone()
        };
        let boundary = boundary();
        let cx = send::Context {
            variables: &self.variables,
            template: &inputs,
            files: sender.files.as_ref(),
            user_agent: &sender.user_agent,
            timeouts: sender.timeouts,
            accept_invalid_certs: sender.sending.accept_invalid_certs,
            http_version: sender.sending.http_version,
            proxy: &sender.sending.proxy,
            boundary: &boundary,
            inherited: &sender.inherited,
            environment: sender.environment.as_ref().map(|found| found.name.as_str()),
        };
        let mut prepared = send::prepare(request, &cx).map_err(|source| SendFailure::Prepare { source })?;
        prepared.tls = certificates::tls_for(&sender.sending.certificates, &prepared.url);
        Ok(prepared)
    }
}

pub(crate) async fn prepare_load(
    sender: &Sender,
    collection: &CollectionId,
    request: &HttpRequest,
    answers: &Answers,
) -> Result<(PreparedRequest, LoadTemplate), SendFailure> {
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
    let variables = secrets::fill(variables, &sender.secrets);
    let variables = environments::effective(&variables, sender.environment.as_ref().map(|found| &found.id));
    let values = sender.inherited.values(&request.variables, &[]);
    let variables = if values.is_empty() {
        variables
    } else {
        cyberapi_core::run::with_values(&variables, &values)
    };
    let needs = send::needs(request, &variables, &sender.inherited);
    let missing = answers.unanswered(&needs.prompts);
    if !missing.is_empty() {
        return Err(SendFailure::Unanswered {
            labels: missing.into_iter().map(str::to_string).collect(),
        });
    }
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
    let since_epoch = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let seed = getrandom::u64().unwrap_or(since_epoch.as_nanos() as u64);
    let inputs = Inputs {
        now: since_epoch.as_secs() as i64,
        custom: variables
            .iter()
            .filter(|v| v.enabled && v.category == VariableCategory::Custom)
            .map(|v| (v.name.clone(), v.value.clone()))
            .collect(),
        responses: responses
            .iter()
            .filter_map(|response| Some((response.api.to_string(), response.body_json()?)))
            .collect(),
        answers: answers.clone(),
        env: if needs.env.is_empty() {
            Env::default()
        } else {
            sender.env.read(&needs.env)
        },
        utc_offset: chrono::Local::now().offset().local_minus_utc(),
        draws: Draws::new(seed),
    };
    let dynamic = send::has_dynamic_templates(request, &variables, &sender.inherited);
    let template = LoadTemplate {
        variables,
        inputs,
        dynamic,
        seed,
    };
    let prepared = template.prepare_again(sender, request, 0)?;
    Ok((prepared, template))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cyberapi_core::grpc;
    use cyberapi_core::model::{ContentType, HttpMethod, KvParam};
    use cyberapi_core::response::{HeaderSource, HttpStats};
    use cyberapi_core::response::{StreamEnd, StreamInfo};
    use cyberapi_core::template::{BaseDir, NoEnv, TemplateError};
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
            ..Default::default()
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
            env: Arc::new(NoEnv("no environment in tests")),
            user_agent: "CyberAPI/test".into(),
            timeouts: Timeouts::default(),
            sending: SendPrefs::default(),
            inherited: Inherited::default(),
            environment: None,
            tokens: Default::default(),
            secrets: Default::default(),
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
            environment: None,
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

    /// A7: every send of a load test told, none kept; one that ends by its
    /// count sends that many. A WebSocket is not load tested.
    #[test]
    fn a_load_test_sends_so_many_and_keeps_none() {
        use cyberapi_core::load::{Limit, Plan};
        let setup = setup(ok(200, "ok"));
        variable(&setup, VariableCategory::Environment, "HOST", "https://api.test");
        let api = new_api(&setup, "ping");
        let samples = Arc::new(Mutex::new(Vec::new()));
        let told = samples.clone();
        let plan = Plan {
            senders: 4,
            limit: Limit::Count(10),
        };
        pollster::block_on(crate::load::load(
            setup.sender.clone(),
            setup.collection.clone(),
            HttpRequest {
                uri: "{{HOST}}/ping".into(),
                ..Default::default()
            },
            Answers::default(),
            plan,
            Arc::new(move |sample| told.lock().expect("samples").push(sample)),
        ))
        .expect("sent");
        let samples = samples.lock().expect("samples");
        assert_eq!(samples.len(), 10);
        assert!(
            samples
                .iter()
                .all(|sample| sample.status == Some(200) && sample.size == 2 && !sample.failed())
        );
        let sent = setup.engine.sent.lock().expect("sent");
        assert_eq!(sent.len(), 10);
        assert!(sent.iter().all(|request| request.url == "https://api.test/ping"));
        let kept = pollster::block_on(setup.storage.history().list(setup.collection.clone(), api)).expect("list");
        assert!(kept.is_empty(), "nothing kept");

        let socket = pollster::block_on(crate::load::load(
            setup.sender.clone(),
            setup.collection.clone(),
            HttpRequest {
                method: HttpMethod::WebSocket,
                uri: "wss://api.test".into(),
                ..Default::default()
            },
            Answers::default(),
            plan,
            Arc::new(|_| {}),
        ));
        assert!(socket.is_err());
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
        let schema = pollster::block_on(introspect(
            setup.sender.clone(),
            setup.collection.clone(),
            request,
            Answers::default(),
        ))
        .expect("a schema");
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
        let why = pollster::block_on(introspect(
            refused.sender.clone(),
            refused.collection.clone(),
            request,
            Answers::default(),
        ));
        assert_eq!(why, Err("introspection is off".into()));
    }

    /// `package shop; message Ask { string id = 1; }
    /// service Store { rpc Get(Ask) returns (Ask); rpc Watch(Ask) returns (stream Ask); }`,
    /// encoded.
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
                method: vec![
                    prost_types::MethodDescriptorProto {
                        name: Some("Get".into()),
                        input_type: Some(".shop.Ask".into()),
                        output_type: Some(".shop.Ask".into()),
                        ..Default::default()
                    },
                    prost_types::MethodDescriptorProto {
                        name: Some("Watch".into()),
                        input_type: Some(".shop.Ask".into()),
                        output_type: Some(".shop.Ask".into()),
                        server_streaming: Some(true),
                        ..Default::default()
                    },
                    prost_types::MethodDescriptorProto {
                        name: Some("Upload".into()),
                        input_type: Some(".shop.Ask".into()),
                        output_type: Some(".shop.Ask".into()),
                        client_streaming: Some(true),
                        ..Default::default()
                    },
                ],
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
        let definitions = pollster::block_on(reflect(
            setup.sender.clone(),
            setup.collection.clone(),
            request.clone(),
            Answers::default(),
        ))
        .expect("definitions");
        assert_eq!(
            definitions
                .methods()
                .iter()
                .map(|m| m.method.as_str())
                .collect::<Vec<_>>(),
            ["Get", "Watch", "Upload"]
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
                Answers::default(),
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

        // E2: a method that streams from the client is not sent from here.
        let upload = HttpRequest {
            body: GrpcBody {
                method: "Upload".into(),
                ..GrpcBody::read(&call.body)
            }
            .to_body(),
            ..call.clone()
        };
        let refused = pollster::block_on(send(
            setup.sender.clone(),
            setup.collection.clone(),
            api.clone(),
            upload,
            Answers::default(),
        ));
        assert_eq!(refused.map(|response| response.status), Err(SendFailure::GrpcStream));

        // A call that failed: its status, no message.
        setup.engine.routes.lock().expect("routes").remove(&get);
        let failed = send_it(&call);
        let status = failed.grpc_status().expect("a status");
        assert_eq!((status.name(), status.message.as_str()), ("UNAVAILABLE", "down"));
        assert!(failed.body.is_empty() && failed.notice.is_none());

        // BUG-8: load tested, a call that fails so is a failure, though
        // its HTTP status is 200.
        let samples = Arc::new(Mutex::new(Vec::new()));
        let told = samples.clone();
        pollster::block_on(crate::load::load(
            setup.sender.clone(),
            setup.collection.clone(),
            call.clone(),
            Answers::default(),
            cyberapi_core::load::Plan {
                senders: 1,
                limit: cyberapi_core::load::Limit::Count(2),
            },
            Arc::new(move |sample| told.lock().expect("samples").push(sample)),
        ))
        .expect("sent");
        let samples = samples.lock().expect("samples");
        assert_eq!(samples.len(), 2);
        assert!(
            samples
                .iter()
                .all(|sample| sample.status == Some(200) && sample.grpc == Some(14) && sample.failed())
        );

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

        // A streamed reply: the call says so, and the replies are kept a
        // line each with when each came.
        let watch = HttpRequest {
            body: GrpcBody {
                method: "Watch".into(),
                ..GrpcBody::read(&call.body)
            }
            .to_body(),
            ..call
        };
        let replies = [grpc::frame(&field(1, b"1")), grpc::frame(&field(1, b"2"))].concat();
        let streamed = Exchange {
            stream: Some(StreamInfo {
                arrivals: vec![(8, 3), (16, 900)],
                ended: StreamEnd::Stopped,
            }),
            ..grpc_answer(replies, "0", "")
        };
        setup
            .engine
            .routes
            .lock()
            .expect("routes")
            .insert("http://shop.test:50051/shop.Store/Watch".into(), streamed);
        let response = send_it(&watch);
        assert!(
            setup
                .engine
                .sent
                .lock()
                .expect("sent")
                .pop()
                .is_some_and(|sent| sent.streams)
        );
        assert_eq!(
            String::from_utf8_lossy(&response.body),
            "{\"id\":\"1\"}\n{\"id\":\"2\"}\n"
        );
        let stream = response.stream.clone().expect("a stream");
        assert_eq!(
            (stream.arrivals, stream.ended),
            (vec![(11, 3), (22, 900)], StreamEnd::Stopped)
        );
        assert!(response.is_stream());
        assert_eq!(response.stream_events()[1].kind(), "Ask");
    }

    /// An event stream's arrivals and end are kept with its answer.
    #[test]
    fn a_streams_arrivals_and_end_are_kept() {
        let stream = StreamInfo {
            arrivals: vec![(9, 4), (18, 60)],
            ended: StreamEnd::Stopped,
        };
        let setup = setup(Ok(Exchange {
            status: 200,
            headers: vec![("content-type".into(), "text/event-stream".into())],
            body: b"data: 0\n\ndata: 1\n\n".to_vec(),
            stream: Some(stream.clone()),
            ..Default::default()
        }));
        let api = new_api(&setup, "events");
        let (_stop, stopped) = cyberapi_http::stop_signal();
        let watch = Watch {
            progress: Arc::new(|_| {}),
            stop: stopped,
        };
        let request = HttpRequest {
            uri: "https://api.test/events".into(),
            ..Default::default()
        };
        let response = pollster::block_on(send_watched(
            setup.sender.clone(),
            setup.collection.clone(),
            api.clone(),
            request,
            Answers::default(),
            Some(watch),
        ))
        .expect("sent");
        assert_eq!(response.stream.as_ref(), Some(&stream));
        let kept = pollster::block_on(setup.storage.history().list(setup.collection.clone(), api)).expect("list");
        assert_eq!(kept[0].stream.as_ref(), Some(&stream));
    }

    fn setup_refusing() -> Setup {
        setup(ok(200, r#"{"errors": [{"message": "introspection is off"}]}"#))
    }

    /// A3: each pass sends the APIs in order; a row's values and what one
    /// API takes from its answer (A2) are read by the requests after it,
    /// in place of the project's; every send is kept in the history.
    #[test]
    fn a_run_hands_what_one_takes_to_the_next() {
        use cyberapi_core::assertions::{Assertion, Extraction, Op, Subject};
        let setup = setup(ok(200, "{}"));
        variable(&setup, VariableCategory::Environment, "HOST", "https://api.test");
        variable(&setup, VariableCategory::Custom, "user", "from the project");
        let login = new_api(&setup, "login");
        let me = new_api(&setup, "me");
        setup.engine.routes.lock().expect("routes").insert(
            "https://api.test/login".into(),
            ok(200, r#"{"token": "t-9"}"#).expect("an answer"),
        );
        let steps = vec![
            crate::RunStep {
                api: login.clone(),
                request: HttpRequest {
                    method: HttpMethod::Post,
                    uri: "{{HOST}}/login".into(),
                    content_type: ContentType::Json,
                    body: r#"{"user": "{{value(user)}}"}"#.into(),
                    extractions: vec![Extraction {
                        name: "token".into(),
                        subject: Subject::Json,
                        target: "$.token".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                inherited: Inherited::default(),
            },
            crate::RunStep {
                api: me.clone(),
                request: HttpRequest {
                    uri: "{{HOST}}/me".into(),
                    headers: vec![kv("Authorization", "Bearer {{value(token)}}")],
                    assertions: vec![Assertion {
                        subject: Subject::Status,
                        op: Op::Equals,
                        expected: "201".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                inherited: Inherited::default(),
            },
        ];
        let rows = vec![
            vec![("user".to_string(), "ann".to_string())],
            vec![("user".to_string(), "bob".to_string())],
        ];
        let told = Arc::new(Mutex::new(Vec::new()));
        let telling = told.clone();
        pollster::block_on(crate::run(
            setup.sender.clone(),
            setup.collection.clone(),
            steps,
            rows,
            Answers::default(),
            Arc::new(move |result| telling.lock().expect("told").push(result)),
        ));
        let sent = setup.engine.sent.lock().expect("sent").clone();
        let bodies: Vec<String> = sent
            .iter()
            .map(|prepared| String::from_utf8_lossy(&prepared.body).into_owned())
            .collect();
        assert_eq!(
            bodies,
            [r#"{"user": "ann"}"#, "", r#"{"user": "bob"}"#, ""],
            "each pass in order, the row's value read"
        );
        assert_eq!(sent[1].header("Authorization"), Some("Bearer t-9"), "the token taken");
        let told = told.lock().expect("told").clone();
        let places: Vec<(usize, usize)> = told.iter().map(|result| (result.pass, result.step)).collect();
        assert_eq!(places, [(0, 0), (0, 1), (1, 0), (1, 1)]);
        assert_eq!(told[0].extracted, [("token".to_string(), "t-9".to_string())]);
        assert!(told[0].held(), "answered, nothing to check");
        assert!(!told[1].held(), "a 200 where 201 was asked");
        assert!(
            told[0].response.as_ref().expect("answered").body.is_empty(),
            "told as a list shows it"
        );
        let kept = pollster::block_on(setup.storage.history().list(setup.collection.clone(), login)).expect("list");
        assert_eq!(kept.len(), 2, "every send kept");
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
            Answers::default(),
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
            Answers {
                file: Some("/tmp/a.png".into()),
                ..Default::default()
            },
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
    fn a_prompt_goes_out_as_answered_and_never_unanswered() {
        let setup = setup(ok(200, "{}"));
        variable(
            &setup,
            VariableCategory::GlobalRequestHeader,
            "X-Tenant",
            "{{prompt(tenant, acme)}}",
        );
        let api = new_api(&setup, "verify");
        let request = HttpRequest {
            uri: "https://api.test/verify".into(),
            headers: vec![kv("X-Code", "{{prompt(code)}}")],
            ..Default::default()
        };
        let send_with = |prompts: &[(&str, &str)]| {
            pollster::block_on(send(
                setup.sender.clone(),
                setup.collection.clone(),
                api.clone(),
                request.clone(),
                Answers {
                    file: None,
                    prompts: prompts.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
                },
            ))
        };
        // An AI assistant's send asks no one: nothing goes out.
        assert_eq!(
            send_with(&[("code", "1")]),
            Err(SendFailure::Unanswered {
                labels: vec!["tenant".into()]
            })
        );
        assert!(setup.engine.sent.lock().expect("sent").is_empty());

        send_with(&[("code", "123456"), ("tenant", "beta")]).expect("sent");
        let sent = setup.engine.sent.lock().expect("sent").pop().expect("sent");
        assert_eq!(sent.header("X-Code"), Some("123456"));
        assert_eq!(sent.header("X-Tenant"), Some("beta"));

        // Its code takes what was typed, and the start or the label stands.
        let curl = pollster::block_on(code(
            setup.sender.clone(),
            setup.collection.clone(),
            request.clone(),
            Answers::default(),
            CodeTarget::Curl,
        ))
        .expect("code");
        assert!(
            curl.contains("X-Code: code") && curl.contains("X-Tenant: acme"),
            "{curl}"
        );
    }

    #[test]
    fn env_is_read_for_the_names_written_and_an_unset_one_stops_the_send() {
        struct Machine(Mutex<Vec<Vec<String>>>);
        impl EnvReader for Machine {
            fn read(&self, names: &[String]) -> Env {
                self.0.lock().expect("asked").push(names.to_vec());
                Env::Read(
                    names
                        .iter()
                        .map(|name| (name.clone(), (name == "TOKEN").then(|| "t-env".to_string())))
                        .collect(),
                )
            }
        }
        let mut setup = setup(ok(200, "{}"));
        let machine = Arc::new(Machine(Mutex::default()));
        setup.sender.env = machine.clone();
        let api = new_api(&setup, "me");
        let send_with = |headers: Vec<KvParam>| {
            pollster::block_on(send(
                setup.sender.clone(),
                setup.collection.clone(),
                api.clone(),
                HttpRequest {
                    uri: "https://api.test/me".into(),
                    headers,
                    ..Default::default()
                },
                Answers::default(),
            ))
        };
        send_with(vec![kv("Authorization", "Bearer {{env(TOKEN)}}")]).expect("sent");
        let sent = setup.engine.sent.lock().expect("sent").pop().expect("sent");
        assert_eq!(sent.header("Authorization"), Some("Bearer t-env"));

        assert_eq!(
            send_with(vec![kv("X-Key", "{{env(MISSING)}}")]),
            Err(SendFailure::Prepare {
                source: PrepareError::Template {
                    source: TemplateError::EnvUnset { name: "MISSING".into() }
                }
            })
        );
        assert!(setup.engine.sent.lock().expect("sent").is_empty(), "not sent");

        // A request without env asks the machine nothing.
        send_with(vec![kv("X-Plain", "1")]).expect("sent");
        assert_eq!(
            *machine.0.lock().expect("asked"),
            [vec!["TOKEN".to_string()], vec!["MISSING".to_string()]]
        );
    }

    /// C2: a 401 that asks for Digest is answered once, and the answer is
    /// what the history keeps; a request with its own Authorization, or a
    /// challenge not answered, ends at the 401.
    #[test]
    fn a_digest_challenge_is_answered_once() {
        struct Camera {
            qop: &'static str,
            asked: Mutex<Vec<PreparedRequest>>,
        }
        impl HttpEngine for Camera {
            fn send(&self, request: PreparedRequest) -> SendFuture {
                self.asked.lock().expect("asked").push(request.clone());
                let answer = match request.header("authorization") {
                    Some(signed) if signed.starts_with("Digest ") => ok(200, "{}"),
                    _ => Ok(Exchange {
                        status: 401,
                        headers: vec![(
                            "www-authenticate".into(),
                            format!(
                                r#"Digest realm="cam", nonce="n1", qop="{}", algorithm=SHA-256"#,
                                self.qop
                            ),
                        )],
                        ..Default::default()
                    }),
                };
                Box::pin(async move { answer })
            }
        }
        let setup = setup(ok(200, "{}"));
        let api = new_api(&setup, "camera");
        let send_to = |setup: &Setup, qop: &'static str, headers: Vec<KvParam>| {
            let camera = Arc::new(Camera {
                qop,
                asked: Mutex::default(),
            });
            let sender = Sender {
                engine: camera.clone(),
                ..setup.sender.clone()
            };
            let response = pollster::block_on(send(
                sender,
                setup.collection.clone(),
                api.clone(),
                HttpRequest {
                    uri: "http://cam.test/ISAPI/info?x=1".into(),
                    headers,
                    auth: cyberapi_core::auth::Auth::Digest {
                        username: "admin".into(),
                        password: "secret".into(),
                    },
                    ..Default::default()
                },
                Answers::default(),
            ))
            .expect("sent");
            let asked = camera.asked.lock().expect("asked").clone();
            (response, asked)
        };

        let (response, asked) = send_to(&setup, "auth", Vec::new());
        assert_eq!(response.status, Some(200));
        assert_eq!(asked.len(), 2);
        assert_eq!(asked[0].header("authorization"), None, "asked first without");
        let signed = asked[1].header("authorization").expect("signed");
        assert!(
            signed.starts_with(r#"Digest username="admin", realm="cam", nonce="n1", uri="/ISAPI/info?x=1", algorithm=SHA-256, response=""#),
            "{signed}"
        );
        let sent = response.sent.expect("sent");
        let header = sent.headers.iter().find(|h| h.name == "Authorization").expect("kept");
        assert_eq!(header.from, HeaderSource::Auth);

        // The request's own Authorization wins: no Digest.
        let (response, asked) = send_to(&setup, "auth", vec![kv("Authorization", "Bearer mine")]);
        assert_eq!((response.status, asked.len()), (Some(401), 1));

        // A challenge it does not answer: the 401, and why.
        let (response, asked) = send_to(&setup, "auth-int", Vec::new());
        assert_eq!((response.status, asked.len()), (Some(401), 1));
        assert!(response.notice.expect("why").contains("qop auth-int"));
    }

    /// C4: a token is asked for once and kept; one the API refuses is
    /// asked for again, with the refresh token first; a token that cannot
    /// be had is the answer, and why. Code takes the kept one, asking none.
    #[test]
    fn an_oauth_token_is_asked_for_once_and_again_when_refused() {
        /// Gives t1, t2…, and takes only the newest.
        struct Id {
            given: Mutex<u32>,
            fails: bool,
            asked: Mutex<Vec<PreparedRequest>>,
        }
        impl HttpEngine for Id {
            fn send(&self, request: PreparedRequest) -> SendFuture {
                self.asked.lock().expect("asked").push(request.clone());
                let mut given = self.given.lock().expect("given");
                let answer = match request.url.as_str() {
                    "https://id.test/token" if self.fails => ok(401, r#"{"error":"invalid_client"}"#),
                    "https://id.test/token" => {
                        *given += 1;
                        ok(
                            200,
                            &format!(
                                r#"{{"access_token":"t{0}","expires_in":3600,"refresh_token":"r{0}"}}"#,
                                *given
                            ),
                        )
                    }
                    _ if request.header("authorization") == Some(&format!("Bearer t{given}")) => ok(200, "{}"),
                    _ => ok(401, "{}"),
                };
                Box::pin(async move { answer })
            }
        }
        let setup = setup(ok(200, "{}"));
        let api = new_api(&setup, "oauth");
        let request = HttpRequest {
            uri: "https://api.test/a".into(),
            auth: cyberapi_core::auth::Auth::from(cyberapi_core::oauth::OAuth2 {
                token_url: "https://id.test/token".into(),
                client_id: "app".into(),
                client_secret: "s".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let id = Arc::new(Id {
            given: Mutex::new(0),
            fails: false,
            asked: Mutex::default(),
        });
        let sender = Sender {
            engine: id.clone(),
            ..setup.sender.clone()
        };
        let send_one = |sender: &Sender| {
            pollster::block_on(send(
                sender.clone(),
                setup.collection.clone(),
                api.clone(),
                request.clone(),
                Answers::default(),
            ))
            .expect("sent")
        };
        let asked = |id: &Id| -> Vec<(String, String)> {
            id.asked
                .lock()
                .expect("asked")
                .drain(..)
                .map(|asked| {
                    let said = match asked.url.as_str() {
                        "https://id.test/token" => String::from_utf8_lossy(&asked.body).to_string(),
                        _ => asked.header("authorization").unwrap_or_default().to_string(),
                    };
                    (asked.url, said)
                })
                .collect()
        };

        let response = send_one(&sender);
        assert_eq!(response.status, Some(200));
        assert_eq!(
            asked(&id),
            [
                (
                    "https://id.test/token".to_string(),
                    "grant_type=client_credentials".to_string()
                ),
                ("https://api.test/a".into(), "Bearer t1".into()),
            ]
        );
        let sent = response.sent.expect("sent");
        let header = sent.headers.iter().find(|h| h.name == "Authorization").expect("kept");
        assert_eq!(header.from, HeaderSource::Auth, "masked as the auth's");

        // Kept: not asked for again.
        assert_eq!(send_one(&sender).status, Some(200));
        assert_eq!(
            asked(&id),
            [("https://api.test/a".to_string(), "Bearer t1".to_string())]
        );

        // Refused: refreshed, and sent again once.
        *id.given.lock().expect("given") = 5;
        assert_eq!(send_one(&sender).status, Some(200));
        assert_eq!(
            asked(&id),
            [
                ("https://api.test/a".to_string(), "Bearer t1".to_string()),
                (
                    "https://id.test/token".into(),
                    "grant_type=refresh_token&refresh_token=r1".into()
                ),
                ("https://api.test/a".into(), "Bearer t6".into()),
            ]
        );

        // Code takes the kept token; with none kept, says where one is had.
        let code_of = |sender: &Sender| {
            pollster::block_on(code(
                sender.clone(),
                setup.collection.clone(),
                request.clone(),
                Answers::default(),
                CodeTarget::Curl,
            ))
            .expect("code")
        };
        assert!(code_of(&sender).contains("Authorization: Bearer t6"));
        let fresh = Sender {
            tokens: Default::default(),
            ..sender.clone()
        };
        assert!(code_of(&fresh).contains("Bearer <an access token from https://id.test/token>"));
        assert_eq!(asked(&id), [], "code asks for none");

        // No token: nothing goes to the API, and the answer says why.
        let refusing = Arc::new(Id {
            given: Mutex::new(0),
            fails: true,
            asked: Mutex::default(),
        });
        let response = send_one(&Sender {
            engine: refusing.clone(),
            tokens: Default::default(),
            ..sender.clone()
        });
        assert_eq!(response.status, None);
        assert_eq!(
            response.error.as_deref(),
            Some("OAuth 2.0: no token from https://id.test/token: 401 invalid_client")
        );
        assert_eq!(asked(&refusing).len(), 1);
    }

    /// C4: an authorization code is not posted as client credentials. With
    /// no refresh token, nothing is sent and the answer says to authorize
    /// in the desktop app. Exchanging a code keeps the token the next send
    /// uses, and that send does not ask the token endpoint again.
    #[test]
    fn an_authorization_code_is_not_asked_until_the_desktop_has_one() {
        struct Id(Mutex<Vec<PreparedRequest>>);
        impl HttpEngine for Id {
            fn send(&self, request: PreparedRequest) -> SendFuture {
                self.0.lock().expect("asked").push(request.clone());
                let answer = if request.url == "https://id.test/token" {
                    ok(200, r#"{"access_token":"t1","expires_in":3600,"refresh_token":"r1"}"#)
                } else if request.header("authorization") == Some("Bearer t1") {
                    ok(200, "{}")
                } else {
                    ok(401, "{}")
                };
                Box::pin(async move { answer })
            }
        }
        let setup = setup(ok(200, "{}"));
        let api = new_api(&setup, "code");
        let request = HttpRequest {
            uri: "https://api.test/a".into(),
            auth: cyberapi_core::auth::Auth::from(cyberapi_core::oauth::OAuth2 {
                grant: cyberapi_core::oauth::Grant::AuthorizationCode,
                authorization_url: "https://id.test/authorize".into(),
                token_url: "https://id.test/token".into(),
                client_id: "app".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let id = Arc::new(Id(Mutex::default()));
        let sender = Sender {
            engine: id.clone(),
            tokens: Default::default(),
            ..setup.sender.clone()
        };
        let response = pollster::block_on(send(
            sender.clone(),
            setup.collection.clone(),
            api.clone(),
            request.clone(),
            Answers::default(),
        ))
        .expect("sent");
        assert_eq!(response.status, None);
        assert_eq!(
            response.error.as_deref(),
            Some("OAuth 2.0: authorize this API in the desktop app first")
        );
        assert!(id.0.lock().expect("asked").is_empty(), "no client_credentials post");

        let grant = pollster::block_on(crate::oauth::prepare_code(
            &sender,
            &setup.collection,
            &request,
            &Answers::default(),
        ))
        .expect("prepared");
        let url = grant.authorize_url(8765).expect("authorize url");
        assert!(url.contains("response_type=code"), "{url}");
        assert!(url.contains("code_challenge_method=S256"), "{url}");
        assert!(
            url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A8765%2Fcallback"),
            "{url}"
        );
        pollster::block_on(crate::oauth::exchange_code(&sender, &grant, 8765, "abc")).expect("exchanged");
        let asked = id.0.lock().expect("asked").drain(..).collect::<Vec<_>>();
        assert_eq!(asked.len(), 1);
        let body = String::from_utf8(asked[0].body.clone()).expect("form");
        assert!(body.starts_with("grant_type=authorization_code"), "{body}");
        assert!(body.contains("code=abc"), "{body}");
        assert!(
            body.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A8765%2Fcallback"),
            "{body}"
        );
        assert!(body.contains("code_verifier="), "{body}");
        assert!(!body.contains("client_credentials"), "{body}");
        assert!(!body.contains("scope="), "{body}");

        let response =
            pollster::block_on(send(sender, setup.collection.clone(), api, request, Answers::default())).expect("sent");
        assert_eq!(response.status, Some(200));
        let asked = id.0.lock().expect("asked").drain(..).collect::<Vec<_>>();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].url, "https://api.test/a");
        assert_eq!(asked[0].header("authorization"), Some("Bearer t1"));
    }

    /// C4: the app's tokens are the reader's, kept in their store under a
    /// hash: after a restart the kept one signs, and none is asked for.
    #[test]
    fn the_apps_oauth_tokens_outlive_it() {
        struct Id(Mutex<usize>);
        impl HttpEngine for Id {
            fn send(&self, request: PreparedRequest) -> SendFuture {
                let answer = if request.url == "https://id.test/token" {
                    *self.0.lock().expect("asked") += 1;
                    ok(200, r#"{"access_token":"t1","expires_in":3600}"#)
                } else if request.header("authorization") == Some("Bearer t1") {
                    ok(200, "{}")
                } else {
                    ok(401, "{}")
                };
                Box::pin(async move { answer })
            }
        }
        let setup = setup(ok(200, "{}"));
        let api = new_api(&setup, "kept");
        let request = HttpRequest {
            uri: "https://api.test/a".into(),
            auth: cyberapi_core::auth::Auth::from(cyberapi_core::oauth::OAuth2 {
                token_url: "https://id.test/token".into(),
                client_id: "app".into(),
                client_secret: "the-secret".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let id = Arc::new(Id(Mutex::new(0)));
        let send_with_tokens = |tokens: Arc<Tokens>| {
            pollster::block_on(send(
                Sender {
                    engine: id.clone(),
                    tokens,
                    ..setup.sender.clone()
                },
                setup.collection.clone(),
                api.clone(),
                request.clone(),
                Answers::default(),
            ))
            .expect("sent")
        };
        assert_eq!(send_with_tokens(Arc::new(Tokens::stored())).status, Some(200));
        assert_eq!(*id.0.lock().expect("asked"), 1);
        let kept = pollster::block_on(setup.storage.prefs().oauth_tokens()).expect("kept");
        assert_eq!(kept.0.len(), 1);
        let (key, token) = kept.0.iter().next().expect("one");
        assert_eq!(key.len(), 64);
        assert_eq!(token.access_token, "t1");
        assert!(token.expires_at.is_some());
        let doc = serde_json::to_string(&kept).expect("JSON");
        assert!(!doc.contains("the-secret") && !doc.contains("id.test"), "{doc}");

        // As the app starts again: the kept one signs.
        assert_eq!(send_with_tokens(Arc::new(Tokens::stored())).status, Some(200));
        assert_eq!(*id.0.lock().expect("asked"), 1, "none asked for");
        // A run's live in memory alone.
        assert_eq!(send_with_tokens(Arc::new(Tokens::default())).status, Some(200));
        assert_eq!(*id.0.lock().expect("asked"), 2);
    }

    /// D3: a secret variable's value is the reader's, filled in as it is
    /// sent; what the history keeps of the request has it masked.
    #[test]
    fn secret_values_are_filled_in_and_kept_masked() {
        let setup = setup(ok(200, "{}"));
        variable(&setup, VariableCategory::Custom, "key", "");
        let key = pollster::block_on(
            setup
                .storage
                .variables()
                .list(setup.collection.clone(), VariableCategory::Custom),
        )
        .expect("variables")[0]
            .id
            .clone();
        let api = new_api(&setup, "secret");
        let sender = Sender {
            secrets: SecretValues(std::collections::BTreeMap::from([(key, "s3cret-key".to_string())])),
            ..setup.sender.clone()
        };
        let response = pollster::block_on(send(
            sender,
            setup.collection.clone(),
            api.clone(),
            HttpRequest {
                uri: "http://a.test/x".into(),
                headers: vec![kv("X-Key", "{{value(key)}}")],
                query: vec![kv("k", "{{value(key)}}")],
                ..Default::default()
            },
            Answers::default(),
        ))
        .expect("sent");
        let asked = setup.engine.sent.lock().expect("sent")[0].clone();
        assert_eq!(asked.header("x-key"), Some("s3cret-key"));
        assert!(asked.url.ends_with("k=s3cret-key"), "{}", asked.url);
        let kept = pollster::block_on(setup.storage.history().list(setup.collection.clone(), api)).expect("history");
        for shown in [&response, &kept[0]] {
            let sent = shown.sent.as_ref().expect("sent");
            let header = sent.headers.iter().find(|h| h.name == "X-Key").expect("header");
            assert_eq!(header.value, secrets::MASK);
            assert!(
                !sent.url.contains("s3cret") && !shown.url.contains("s3cret"),
                "{}",
                sent.url
            );
        }
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
                Answers::default(),
            ))
            .expect("sent")
        };
        send_one(request("https://api.test/login", vec![]));
        let jar = pollster::block_on(setup.storage.cookies().list()).expect("cookies");
        assert_eq!(jar.len(), 1, "a cookie for another site is not kept: {jar:?}");
        assert_eq!((jar[0].name.as_str(), jar[0].domain.as_str()), ("sid", "api.test"));

        let jars = send_one(request("https://api.test/me", vec![]));
        send_one(request("https://other.test/", vec![]));
        let own = send_one(request("https://api.test/me", vec![kv("Cookie", "mine=1")]));
        let sent = setup.engine.sent.lock().expect("sent").clone();
        assert_eq!(sent[1].header("cookie"), Some("sid=s-1"));
        assert_eq!(sent[2].header("cookie"), None);
        assert_eq!(sent[3].header("cookie"), Some("mine=1"), "the request's own wins");
        // #58: the history says what went, and where its cookie came from.
        let cookie = |response: &HttpResponse| {
            let sent = response.sent.as_ref().expect("kept as sent");
            sent.headers
                .iter()
                .find(|header| header.name.eq_ignore_ascii_case("cookie"))
                .map(|header| (header.value.clone(), header.from))
        };
        assert_eq!(cookie(&jars), Some(("sid=s-1".into(), HeaderSource::Cookie)));
        assert_eq!(cookie(&own), Some(("mine=1".into(), HeaderSource::Request)));
        let kept =
            pollster::block_on(setup.storage.history().list(setup.collection.clone(), api.clone())).expect("history");
        assert!(kept.iter().all(|response| response.sent.is_some()));
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
            Answers::default(),
        ))
        .expect("sent")
    }

    /// #71: an API's own settings stand in for the reader's.
    #[test]
    fn an_apis_own_settings_go_out_with_it() {
        let setup = setup(Ok(Exchange {
            status: 200,
            body: b"me".to_vec(),
            ..Default::default()
        }));
        route(&setup, "https://api.test/login", redirect("/me", None));
        let api = new_api(&setup, "login");
        let request = HttpRequest {
            method: HttpMethod::Get,
            uri: "https://api.test/login".into(),
            settings: cyberapi_core::prefs::RequestSettings {
                timeouts: Timeouts {
                    connect: 3,
                    ..Timeouts::default()
                },
                follow_redirects: Some(true),
                accept_invalid_certs: Some(true),
                http_version: Some(cyberapi_core::prefs::HttpVersion::Http1),
            },
            ..Default::default()
        };
        // The reader's settings follow no redirect and check certificates.
        assert!(!setup.sender.sending.follow_redirects && !setup.sender.sending.accept_invalid_certs);
        let answer = pollster::block_on(send(
            setup.sender.clone(),
            setup.collection.clone(),
            api,
            request.clone(),
            Answers::default(),
        ))
        .expect("sent");
        assert_eq!(answer.status, Some(200), "followed, as the API says");
        let sent = setup.engine.sent.lock().expect("sent").clone();
        assert!(sent.iter().all(|hop| hop.accept_invalid_certs));
        assert!(
            sent.iter()
                .all(|hop| hop.http_version == cyberapi_core::prefs::HttpVersion::Http1)
        );
        assert_eq!(sent[0].timeouts.connect, 3);
        assert_eq!(
            sent[0].timeouts.read,
            Timeouts::DEFAULT_READ,
            "not set here: the reader's"
        );
        // Its code too: curl follows and takes any certificate.
        let curl = pollster::block_on(code(
            setup.sender.clone(),
            setup.collection.clone(),
            request,
            Answers::default(),
            CodeTarget::Curl,
        ))
        .expect("code");
        assert!(
            curl.contains(" -L") && curl.contains(" -k") && curl.contains("--http1.1"),
            "{curl}"
        );
    }

    /// #66: what the API takes from its folders and project goes out with
    /// it, and the history says where each header came from.
    #[test]
    fn an_api_sends_what_it_inherits() {
        let mut setup = setup(Ok(Exchange {
            status: 200,
            ..Default::default()
        }));
        setup.sender.inherited = Inherited {
            headers: vec![KvParam {
                key: "X-Team".into(),
                value: "users".into(),
                enabled: true,
                ..Default::default()
            }],
            auth: cyberapi_core::auth::Auth::Bearer { token: "t0".into() },
            ..Default::default()
        };
        let api = new_api(&setup, "me");
        let request = HttpRequest {
            uri: "https://api.test/me".into(),
            ..Default::default()
        };
        let answer = pollster::block_on(send(
            setup.sender.clone(),
            setup.collection.clone(),
            api,
            request.clone(),
            Answers::default(),
        ))
        .expect("sent");
        let sent = setup.engine.sent.lock().expect("sent")[0].clone();
        assert_eq!(sent.header("x-team"), Some("users"));
        assert_eq!(sent.header("authorization"), Some("Bearer t0"));
        let from = |name: &str| {
            answer
                .sent
                .as_ref()
                .and_then(|sent| {
                    sent.headers
                        .iter()
                        .find(|header| header.name.eq_ignore_ascii_case(name))
                })
                .map(|header| header.from)
        };
        assert_eq!(from("x-team"), Some(HeaderSource::Folder));
        assert_eq!(from("authorization"), Some(HeaderSource::Auth));
        // Its code too.
        let curl = pollster::block_on(code(
            setup.sender.clone(),
            setup.collection.clone(),
            request,
            Answers::default(),
            CodeTarget::Curl,
        ))
        .expect("code");
        assert!(curl.contains("X-Team: users") && curl.contains("Bearer t0"), "{curl}");
    }

    /// D5: a folder's variables and the API's own stand in for the
    /// environment's and the project's custom ones of the same names, the
    /// API's before the folder's.
    #[test]
    fn an_api_reads_its_own_and_its_folders_variables() {
        let mut setup = setup(Ok(Exchange {
            status: 200,
            ..Default::default()
        }));
        variable(&setup, VariableCategory::Environment, "HOST", "https://shared.test");
        variable(&setup, VariableCategory::Custom, "team", "project");
        variable(&setup, VariableCategory::Custom, "page", "0");
        let kv = |key: &str, value: &str| KvParam {
            key: key.into(),
            value: value.into(),
            enabled: true,
            ..Default::default()
        };
        setup.sender.inherited = Inherited {
            variables: vec![kv("HOST", "https://folder.test"), kv("team", "folder"), kv("page", "1")],
            ..Default::default()
        };
        let api = new_api(&setup, "list");
        let request = HttpRequest {
            uri: "{{HOST}}/list".into(),
            query: vec![kv("team", "{{value(team)}}"), kv("page", "{{value(page)}}")],
            variables: vec![kv("page", "2")],
            ..Default::default()
        };
        pollster::block_on(send(
            setup.sender.clone(),
            setup.collection.clone(),
            api,
            request,
            Answers::default(),
        ))
        .expect("sent");
        let sent = setup.engine.sent.lock().expect("sent")[0].clone();
        assert_eq!(sent.url, "https://folder.test/list?team=folder&page=2");
    }

    /// #61: in a named environment, its variables stand in for the shared
    /// ones; a prefix neither has says which environment it was sent in.
    #[test]
    fn an_api_is_sent_in_the_readers_environment() {
        let mut setup = setup(Ok(Exchange {
            status: 200,
            ..Default::default()
        }));
        variable(&setup, VariableCategory::Environment, "HOST", "https://shared.test");
        let prod = pollster::block_on(
            setup
                .storage
                .variables()
                .create_environment(setup.collection.clone(), "prod".into()),
        )
        .expect("an environment");
        pollster::block_on(setup.storage.variables().create(NewVariable {
            collection: setup.collection.clone(),
            category: VariableCategory::Environment,
            name: "HOST".into(),
            value: "https://prod.test".into(),
            enabled: true,
            environment: Some(prod.id.clone()),
        }))
        .expect("prod's HOST");
        let api = new_api(&setup, "me");
        let request = HttpRequest {
            uri: "{{HOST}}/me".into(),
            ..Default::default()
        };
        let send_in = |setup: &Setup, request: &HttpRequest| {
            pollster::block_on(send(
                setup.sender.clone(),
                setup.collection.clone(),
                api.clone(),
                request.clone(),
                Answers::default(),
            ))
        };
        send_in(&setup, &request).expect("sent");
        setup.sender.environment = Some(prod.clone());
        send_in(&setup, &request).expect("sent");
        let urls: Vec<String> = setup
            .engine
            .sent
            .lock()
            .expect("sent")
            .iter()
            .map(|sent| sent.url.clone())
            .collect();
        assert_eq!(urls, ["https://shared.test/me", "https://prod.test/me"]);
        let missing = HttpRequest {
            uri: "{{API}}/me".into(),
            ..Default::default()
        };
        match send_in(&setup, &missing) {
            Err(SendFailure::Prepare {
                source: send::PrepareError::Environment { name, within },
            }) => assert_eq!((name.as_str(), within.as_deref()), ("API", Some("prod"))),
            other => panic!("expected the prefix missing in prod, got {other:?}"),
        }
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
            Answers::default(),
        ))
        .expect("sent");
        let sent = setup.engine.sent.lock().expect("sent").clone();
        let cookies: Vec<_> = sent[2..].iter().map(|request| request.header("cookie")).collect();
        assert_eq!(cookies, vec![Some("mine=1"), Some("mine=1"), Some("b=2")]);
    }

    /// B3: an import's text fetched: redirects followed whatever the
    /// settings say, nothing kept; why when there is none.
    #[test]
    fn a_fetch_follows_redirects_and_keeps_nothing() {
        let setup = setup(ok(200, "openapi: 3.0.0"));
        route(&setup, "https://specs.test/latest", redirect("/v2.yaml", None));
        let fetch = |url: &str| pollster::block_on(fetch_text(setup.sender.clone(), url.into()));
        assert_eq!(fetch(" https://specs.test/latest ").as_deref(), Ok("openapi: 3.0.0"));
        let sent = setup.engine.sent.lock().expect("sent").clone();
        assert_eq!(
            sent.iter().map(|request| request.url.as_str()).collect::<Vec<_>>(),
            ["https://specs.test/latest", "https://specs.test/v2.yaml"]
        );
        assert_eq!(sent[0].header("user-agent"), Some("CyberAPI/test"));
        assert_eq!(sent[0].header("cookie"), None);
        assert!(
            pollster::block_on(setup.storage.cookies().list())
                .expect("cookies")
                .is_empty()
        );
        route(&setup, "https://specs.test/gone", ok(404, "no").expect("an answer"));
        assert_eq!(
            fetch("https://specs.test/gone"),
            Err("https://specs.test/gone answered 404".into())
        );
        assert!(fetch("specs.test/v2.yaml").is_err(), "no address");
        assert!(fetch("file:///etc/passwd").is_err(), "no http address");
        route(&setup, "https://specs.test/a", redirect("/b", None));
        route(&setup, "https://specs.test/b", redirect("/a", None));
        assert_eq!(fetch("https://specs.test/a"), Err("stopped after 10 redirects".into()));
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
        let request = HttpRequest {
            uri: "{{HOST}}/me".into(),
            query: vec![kv("v", "{{md5(a)}}")],
            ..Default::default()
        };
        let as_code = |target| {
            pollster::block_on(code(
                setup.sender.clone(),
                setup.collection.clone(),
                request.clone(),
                Answers::default(),
                target,
            ))
            .expect("code")
        };
        assert_eq!(
            as_code(CodeTarget::Curl),
            "curl -v -XGET -H 'User-Agent: CyberAPI/test' 'https://api.test/me?v=0cc175b9c0f1b6a831c399e269772661'"
        );
        // #69: another client's code, prepared the same way.
        assert_eq!(
            as_code(CodeTarget::Httpie),
            "http --verbose GET 'https://api.test/me?v=0cc175b9c0f1b6a831c399e269772661' 'User-Agent:CyberAPI/test'"
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
            Answers::default(),
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
            Answers::default(),
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
            Answers::default(),
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
                Answers::default(),
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
