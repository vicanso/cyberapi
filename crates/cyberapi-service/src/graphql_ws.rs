//! E3: a GraphQL subscription over WebSocket (graphql-transport-ws). It is
//! prepared as a send is, then opened by the engine (the desktop's alone)
//! and spoken by [`drive`]: `connection_init` until the server's ack, then
//! one `subscribe` of the query. What the server yields is told as it
//! comes and kept once the subscription ends. An address that is not a
//! WebSocket's is [`Started::Http`]: the caller sends it as a request.

use crate::oauth;
use crate::send::{SendFailure, Sender, Unanswered, add_cookies, keep_cookies, kept, prepare};
use cyberapi_core::graphql_ws::{self, Message};
use cyberapi_core::model::{ContentType, HttpRequest};
use cyberapi_core::response::{Exchange, HttpResponse, HttpStats, RESPONSE_LIMIT, SentRequest, StreamEnd, StreamInfo};
use cyberapi_core::secrets;
use cyberapi_core::send::{self, PrepareError, PreparedRequest};
use cyberapi_core::template::Answers;
use cyberapi_core::ws::{self, Frame};
use cyberapi_core::{CollectionId, ItemId};
use cyberapi_http::{Opened, SocketIn, SocketOut};
use futures::FutureExt as _;
use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use futures::channel::oneshot;
use std::collections::VecDeque;
use std::time::Duration;

/// What opening a subscription came to.
pub enum Started {
    /// The rendered address is `http` or `https`: send the request.
    Http,
    /// Open. [`drive`] speaks it and keeps it once it ends.
    Open(Subscription),
    /// It did not open, or the server rejected it: kept in the history.
    Kept(HttpResponse),
}

/// An open subscription, past `connection_ack`, with `subscribe` sent.
pub struct Subscription {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    outgoing: UnboundedSender<SocketOut>,
    incoming: UnboundedReceiver<(SocketIn, u64)>,
    /// Messages that arrived before `subscribe` was sent.
    early: VecDeque<(SocketIn, u64)>,
    head: Head,
}

/// What the history keeps of how it was opened.
struct Head {
    collection: CollectionId,
    api: ItemId,
    request: HttpRequest,
    url: String,
    status: u16,
    headers: Vec<(String, String)>,
    stats: HttpStats,
    sent: SentRequest,
}

/// One result, as the panel shows it while the subscription is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heard {
    pub kind: String,
    pub data: String,
    pub at: u64,
}

/// Opens `request` when its rendered address is a WebSocket. `answers` are
/// the reader's to its `prompt`s.
pub async fn subscribe(
    sender: Sender,
    collection: CollectionId,
    api: ItemId,
    request: HttpRequest,
    answers: Answers,
) -> Result<Started, SendFailure> {
    let sender = sender.for_request(&request);
    let (mut prepared, globals) =
        prepare(&sender, &collection, &request, &answers, Unanswered::Stops, None, &[]).await?;
    if request.content_type != ContentType::GraphQl || !graphql_ws::is_socket_url(&prepared.url) {
        return Ok(Started::Http);
    }
    let payload = graphql_payload(&request, &prepared).map_err(|message| SendFailure::Prepare {
        source: PrepareError::GraphQlVariables { message },
    })?;
    // The handshake carries no body. The query goes in `subscribe`.
    prepared.body.clear();
    prepared.headers.retain(|(name, value)| {
        let name = name.to_ascii_lowercase();
        if name == "content-type" || name == "content-length" {
            return false;
        }
        !(name == "accept" && value.to_ascii_lowercase().contains("text/event-stream"))
    });
    if !prepared
        .headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("sec-websocket-protocol"))
    {
        prepared
            .headers
            .push(("Sec-WebSocket-Protocol".into(), graphql_ws::PROTOCOL.into()));
    }
    let own_cookie = prepared.header("cookie").map(str::to_string);
    if own_cookie.is_none() {
        let mut asked = as_http(&prepared);
        add_cookies(&sender, &mut asked).await?;
        prepared.headers = asked.headers;
    }
    let no_token = oauth::sign(&sender, &mut prepared, false).await.err();
    let jar = prepared.header("cookie").is_some() && prepared.header("cookie") != own_cookie.as_deref();
    let sent = secrets::mask_sent(
        send::sent_request(&prepared, &request, &sender.inherited, &globals, jar, "HTTP/1.1"),
        &sender.secrets,
    );
    let shown_url = secrets::mask(&prepared.url, &sender.secrets);
    let failed = |error: String| HttpResponse {
        api: api.clone(),
        collection: collection.clone(),
        request: request.clone(),
        url: shown_url.clone(),
        error: Some(error),
        latency: Duration::from_millis(1),
        sent: Some(sent.clone()),
        ..Default::default()
    };
    if let Some(why) = no_token {
        return Ok(Started::Kept(kept(&sender, failed(why)).await));
    }
    let opened = sender.engine.open_socket(prepared.clone()).await;
    let cookies_of = |headers: &[(String, String)]| Exchange {
        headers: headers.to_vec(),
        ..Default::default()
    };
    let http_url = as_http(&prepared).url;
    let (status, headers, stats, outgoing, mut incoming) = match opened {
        Ok(Opened::Socket(socket)) => {
            keep_cookies(&sender, &cookies_of(&socket.headers), &http_url).await;
            (
                socket.status,
                socket.headers,
                socket.stats,
                socket.outgoing,
                socket.incoming,
            )
        }
        Ok(Opened::Refused(exchange)) => {
            keep_cookies(&sender, &exchange, &http_url).await;
            let response = HttpResponse {
                api,
                collection,
                request,
                url: shown_url,
                status: Some(exchange.status),
                headers: exchange.headers,
                body_size: exchange.body_size,
                body: exchange.body,
                latency: exchange.stats.total.max(Duration::from_millis(1)),
                stats: exchange.stats,
                truncated: exchange.truncated,
                sent: Some(sent),
                ..Default::default()
            };
            return Ok(Started::Kept(kept(&sender, response).await));
        }
        Err(error) => return Ok(Started::Kept(kept(&sender, failed(error.to_string())).await)),
    };
    let head = Head {
        collection: collection.clone(),
        api: api.clone(),
        request: request.clone(),
        url: shown_url.clone(),
        status,
        headers: headers.clone(),
        stats,
        sent: sent.clone(),
    };
    let authorization = prepared.header("authorization");
    if outgoing
        .unbounded_send(SocketOut::Send(Frame::Text(graphql_ws::connection_init(authorization))))
        .is_err()
    {
        return Ok(Started::Kept(
            kept(&sender, failed("the connection closed".into())).await,
        ));
    }
    let mut early = VecDeque::new();
    loop {
        let Some((came, at)) = incoming.next().await else {
            return Ok(Started::Kept(
                kept(
                    &sender,
                    failed("the server closed the connection before connection_ack".into()),
                )
                .await,
            ));
        };
        let SocketIn::Message(frame) = came else {
            let why = match &came {
                SocketIn::Failed(message) => message.clone(),
                _ => "the server closed the connection before connection_ack".into(),
            };
            return Ok(Started::Kept(kept(&sender, failed(why)).await));
        };
        let Frame::Text(text) = frame else {
            early.push_back((SocketIn::Message(frame), at));
            continue;
        };
        match graphql_ws::read_message(&text) {
            Message::Ack => break,
            Message::Ping(payload) => {
                let _ = outgoing.unbounded_send(SocketOut::Send(Frame::Text(graphql_ws::pong(payload.as_ref()))));
            }
            Message::ConnectionError(why) => {
                let _ = outgoing.unbounded_send(SocketOut::Close);
                let why = if why.is_empty() {
                    "the server rejected the connection".into()
                } else {
                    why
                };
                return Ok(Started::Kept(kept(&sender, failed(why)).await));
            }
            Message::Pong => {}
            _ => early.push_back((SocketIn::Message(Frame::Text(text)), at)),
        }
    }
    if outgoing
        .unbounded_send(SocketOut::Send(Frame::Text(graphql_ws::subscribe(&payload))))
        .is_err()
    {
        return Ok(Started::Kept(
            kept(&sender, failed("the connection closed".into())).await,
        ));
    }
    Ok(Started::Open(Subscription {
        status,
        headers,
        outgoing,
        incoming,
        early,
        head,
    }))
}

/// Speaks the subscription until it ends: answers a ping, tells `events`
/// each result, and on `stop` sends `complete` and closes. The result is
/// kept in the history.
pub async fn drive(
    sender: Sender,
    subscription: Subscription,
    events: UnboundedSender<Heard>,
    stop: oneshot::Receiver<()>,
) -> HttpResponse {
    drive_within(sender, subscription, events, stop, RESPONSE_LIMIT).await
}

/// [`drive`], the results kept to `limit` bytes: once they reach it the
/// subscription is completed and closed, and the answer says it was cut —
/// as an event stream's is at the most the engine reads (PERF-6).
async fn drive_within(
    sender: Sender,
    mut subscription: Subscription,
    events: UnboundedSender<Heard>,
    stop: oneshot::Receiver<()>,
    limit: usize,
) -> HttpResponse {
    let mut stop = stop.fuse();
    let mut stopping = false;
    let mut body = Vec::new();
    let mut arrivals = Vec::new();
    let mut ended = StreamEnd::Closed;
    loop {
        let came = if stopping {
            subscription.incoming.next().await
        } else if let Some(early) = subscription.early.pop_front() {
            Some(early)
        } else {
            futures::select! {
                _ = stop => {
                    stopping = true;
                    ended = StreamEnd::Stopped;
                    let _ = subscription
                        .outgoing
                        .unbounded_send(SocketOut::Send(Frame::Text(graphql_ws::complete())));
                    let _ = subscription.outgoing.unbounded_send(SocketOut::Close);
                    continue;
                }
                came = subscription.incoming.next() => came,
            }
        };
        let Some((came, at)) = came else {
            if !stopping {
                ended = StreamEnd::Closed;
            }
            break;
        };
        match came {
            SocketIn::Message(Frame::Text(text)) => match graphql_ws::read_message(&text) {
                Message::Ping(payload) => {
                    let _ = subscription
                        .outgoing
                        .unbounded_send(SocketOut::Send(Frame::Text(graphql_ws::pong(payload.as_ref()))));
                }
                Message::Next(data) => note(&mut body, &mut arrivals, &events, graphql_ws::NEXT, data, at),
                Message::Error(data) => {
                    note(&mut body, &mut arrivals, &events, graphql_ws::ERROR, data, at);
                    if !stopping {
                        ended = StreamEnd::Closed;
                    }
                    let _ = subscription.outgoing.unbounded_send(SocketOut::Close);
                    stopping = true;
                }
                Message::Complete => {
                    if !stopping {
                        ended = StreamEnd::Closed;
                    }
                    break;
                }
                Message::ConnectionError(why) => {
                    ended = StreamEnd::Failed {
                        message: if why.is_empty() {
                            "the server rejected the connection".into()
                        } else {
                            why
                        },
                    };
                    break;
                }
                Message::Other(text) => note(&mut body, &mut arrivals, &events, "message", text, at),
                Message::Ack | Message::Pong => {}
            },
            SocketIn::Message(Frame::Binary(bytes)) => {
                note(
                    &mut body,
                    &mut arrivals,
                    &events,
                    "message",
                    format!("binary, {} bytes", bytes.len()),
                    at,
                );
            }
            SocketIn::Failed(message) => {
                if !stopping {
                    ended = StreamEnd::Failed { message };
                }
                break;
            }
            SocketIn::Closed { .. } => {
                if !stopping {
                    ended = StreamEnd::Closed;
                }
                break;
            }
        }
        if body.len() >= limit && !stopping {
            ended = StreamEnd::Cut;
            stopping = true;
            let _ = subscription
                .outgoing
                .unbounded_send(SocketOut::Send(Frame::Text(graphql_ws::complete())));
            let _ = subscription.outgoing.unbounded_send(SocketOut::Close);
        }
    }
    finish(&sender, subscription.head, body, arrivals, ended).await
}

fn note(
    body: &mut Vec<u8>,
    arrivals: &mut Vec<(u64, u64)>,
    events: &UnboundedSender<Heard>,
    kind: &str,
    data: String,
    at: u64,
) {
    body.extend(graphql_ws::line(kind, &data));
    arrivals.push((body.len() as u64, at));
    let _ = events.unbounded_send(Heard {
        kind: kind.to_string(),
        data,
        at,
    });
}

async fn finish(
    sender: &Sender,
    head: Head,
    body: Vec<u8>,
    arrivals: Vec<(u64, u64)>,
    ended: StreamEnd,
) -> HttpResponse {
    let lasted = arrivals.last().map_or(0, |(_, at)| *at);
    let response = HttpResponse {
        api: head.api,
        collection: head.collection,
        request: head.request,
        url: head.url,
        status: Some(head.status),
        headers: head.headers,
        body_size: body.len() as u64,
        body,
        latency: (head.stats.total + Duration::from_millis(lasted)).max(Duration::from_millis(1)),
        stats: head.stats,
        stream: Some(StreamInfo { arrivals, ended }),
        sent: Some(head.sent),
        ..Default::default()
    };
    kept(sender, response).await
}

/// The query as `subscribe`'s payload. A POST's is the body `prepare`
/// built; a GET's is the `query` / `variables` / `operationName` it put on
/// the address.
fn graphql_payload(request: &HttpRequest, prepared: &PreparedRequest) -> Result<serde_json::Value, String> {
    if request.method.has_body() {
        let text = String::from_utf8_lossy(&prepared.body);
        if text.trim().is_empty() {
            return Ok(serde_json::json!({"query": ""}));
        }
        return serde_json::from_str(text.trim()).map_err(|error| error.to_string());
    }
    let http = ws::http_url(&prepared.url).unwrap_or_else(|| prepared.url.clone());
    let url = url::Url::parse(&http).map_err(|error| error.to_string())?;
    let mut query = String::new();
    let mut variables = None;
    let mut operation_name = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "query" => query = value.into_owned(),
            "variables" => variables = Some(value.into_owned()),
            "operationName" => operation_name = Some(value.into_owned()),
            _ => {}
        }
    }
    let mut payload = serde_json::Map::new();
    payload.insert("query".into(), serde_json::Value::String(query));
    if let Some(text) = variables {
        payload.insert(
            "variables".into(),
            serde_json::from_str(&text).map_err(|error| error.to_string())?,
        );
    }
    if let Some(name) = operation_name.filter(|name| !name.is_empty()) {
        payload.insert("operationName".into(), serde_json::Value::String(name));
    }
    Ok(serde_json::Value::Object(payload))
}

fn as_http(prepared: &PreparedRequest) -> PreparedRequest {
    PreparedRequest {
        url: ws::http_url(&prepared.url).unwrap_or_else(|| prepared.url.clone()),
        ..prepared.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cyberapi_core::auth::Auth;
    use cyberapi_core::graphql::GraphQlBody;
    use cyberapi_core::graphql_ws::NEXT;
    use cyberapi_core::model::HttpMethod;
    use cyberapi_core::prefs::{SendPrefs, Timeouts};
    use cyberapi_core::response::StreamEnd;
    use cyberapi_core::template::{NoEnv, NoFiles};
    use cyberapi_http::{HttpEngine, OpenFuture, SendError, SendFuture, Socket};
    use cyberapi_store::{MemoryStore, NewCollection, Storage};
    use futures::channel::mpsc;
    use pretty_assertions::assert_eq;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Engine {
        asked: Mutex<Vec<PreparedRequest>>,
        server: Mutex<Option<Server>>,
    }

    type Server = (
        mpsc::UnboundedReceiver<SocketOut>,
        mpsc::UnboundedSender<(SocketIn, u64)>,
    );

    impl HttpEngine for Engine {
        fn send(&self, _: PreparedRequest) -> SendFuture {
            Box::pin(async { Err(SendError::Invalid { message: "no".into() }) })
        }

        fn open_socket(&self, request: PreparedRequest) -> OpenFuture {
            self.asked.lock().expect("asked").push(request);
            let (outgoing, sent) = mpsc::unbounded();
            let (told, incoming) = mpsc::unbounded();
            *self.server.lock().expect("server") = Some((sent, told));
            Box::pin(async {
                Ok(Opened::Socket(Socket {
                    status: 101,
                    headers: Vec::new(),
                    stats: HttpStats::default(),
                    outgoing,
                    incoming,
                }))
            })
        }
    }

    async fn setup() -> (Storage, Arc<Engine>, Sender, CollectionId, ItemId) {
        let storage = MemoryStore::new().storage();
        let collection = storage
            .collections()
            .create(NewCollection {
                name: "p".into(),
                ..Default::default()
            })
            .await
            .expect("project")
            .id;
        let api = storage
            .tree()
            .create_api(collection.clone(), None, "live".into())
            .await
            .expect("api")
            .item
            .id;
        let engine = Arc::new(Engine::default());
        let sender = Sender {
            storage: storage.clone(),
            engine: engine.clone(),
            files: Arc::new(NoFiles("no files")),
            env: Arc::new(NoEnv("no environment")),
            user_agent: "CyberAPI/test".into(),
            timeouts: Timeouts::default(),
            sending: SendPrefs::default(),
            inherited: Default::default(),
            environment: None,
            tokens: Default::default(),
            secrets: Default::default(),
        };
        (storage, engine, sender, collection, api)
    }

    fn request(uri: &str) -> HttpRequest {
        HttpRequest {
            method: HttpMethod::Post,
            uri: uri.into(),
            content_type: ContentType::GraphQl,
            body: GraphQlBody {
                query: "subscription { ping }".into(),
                ..Default::default()
            }
            .to_body(),
            auth: Auth::Bearer { token: "t".into() },
            ..Default::default()
        }
    }

    async fn wait_server(engine: &Engine) -> Server {
        loop {
            if let Some(server) = engine.server.lock().expect("server").take() {
                return server;
            }
            futures::future::poll_fn(|cx| {
                cx.waker().wake_by_ref();
                std::task::Poll::<()>::Pending
            })
            .await;
        }
    }

    #[test]
    fn a_socket_address_is_subscribed_and_an_http_one_is_not() {
        pollster::block_on(async {
            let (_storage, engine, sender, collection, api) = setup().await;
            let http = subscribe(
                sender.clone(),
                collection.clone(),
                api.clone(),
                request("https://a.test/graphql"),
                Answers::default(),
            )
            .await
            .expect("http");
            assert!(matches!(http, Started::Http));
            assert!(engine.asked.lock().expect("asked").is_empty(), "nothing opened");

            let opening = subscribe(
                sender.clone(),
                collection.clone(),
                api.clone(),
                request("wss://a.test/graphql"),
                Answers::default(),
            );
            let serving = async {
                let (mut sent, told) = wait_server(&engine).await;
                let init = sent.next().await.expect("init");
                assert!(
                    matches!(init, SocketOut::Send(Frame::Text(text)) if text.contains("connection_init") && text.contains("Bearer t"))
                );
                told.unbounded_send((
                    SocketIn::Message(Frame::Text("{\"type\":\"connection_ack\"}".into())),
                    1,
                ))
                .expect("ack");
                let subscribe = sent.next().await.expect("subscribe");
                assert!(
                    matches!(subscribe, SocketOut::Send(Frame::Text(text)) if text.contains("subscribe") && text.contains("subscription { ping }"))
                );
                (sent, told)
            };
            let (started, (mut sent, told)) = futures::future::join(opening, serving).await;
            let Started::Open(subscription) = started.expect("opened") else {
                panic!("not open");
            };
            let asked = engine.asked.lock().expect("asked")[0].clone();
            assert_eq!(asked.url, "wss://a.test/graphql");
            assert_eq!(asked.header("sec-websocket-protocol"), Some(graphql_ws::PROTOCOL));
            assert!(asked.body.is_empty());
            assert_eq!(asked.header("authorization"), Some("Bearer t"));
            assert!(asked.header("content-type").is_none());

            let (events_tx, mut events) = mpsc::unbounded();
            let (stop_tx, stop_rx) = oneshot::channel();
            let driving = drive(sender, subscription, events_tx, stop_rx);
            let feeding = async {
                told.unbounded_send((
                    SocketIn::Message(Frame::Text("{\"type\":\"ping\",\"payload\":{\"n\":1}}".into())),
                    5,
                ))
                .expect("ping");
                let pong = sent.next().await.expect("pong");
                assert!(
                    matches!(pong, SocketOut::Send(Frame::Text(text)) if text.contains("pong") && text.contains("\"n\":1"))
                );
                told.unbounded_send((
                    SocketIn::Message(Frame::Text(
                        "{\"id\":\"1\",\"type\":\"next\",\"payload\":{\"data\":{\"ping\":\"ok\"}}}".into(),
                    )),
                    9,
                ))
                .expect("next");
                told.unbounded_send((
                    SocketIn::Message(Frame::Text("{\"id\":\"1\",\"type\":\"complete\"}".into())),
                    12,
                ))
                .expect("complete");
            };
            let (response, _) = futures::future::join(driving, feeding).await;
            drop(stop_tx);
            let heard = events.next().await.expect("an event");
            assert_eq!(heard.kind, NEXT);
            assert!(heard.data.contains("ping"));
            assert!(events.next().await.is_none());
            assert!(response.is_stream());
            assert_eq!(
                response.stream.as_ref().map(|stream| stream.ended.clone()),
                Some(StreamEnd::Closed)
            );
            let shown = response.stream_events();
            assert_eq!(shown.len(), 1);
            assert_eq!(shown[0].event, NEXT);
            assert_eq!(shown[0].at, Some(9));
        });
    }

    /// PERF-6: results stop being kept at the limit: the subscription is
    /// completed and closed, and the answer says it was cut.
    #[test]
    fn a_subscription_ends_at_the_limit() {
        pollster::block_on(async {
            let (_storage, engine, sender, collection, api) = setup().await;
            let opening = subscribe(
                sender.clone(),
                collection,
                api,
                request("wss://a.test/graphql"),
                Answers::default(),
            );
            let serving = async {
                let (mut sent, told) = wait_server(&engine).await;
                sent.next().await.expect("init");
                told.unbounded_send((
                    SocketIn::Message(Frame::Text("{\"type\":\"connection_ack\"}".into())),
                    1,
                ))
                .expect("ack");
                sent.next().await.expect("subscribe");
                (sent, told)
            };
            let (started, (mut sent, told)) = futures::future::join(opening, serving).await;
            let Started::Open(subscription) = started.expect("opened") else {
                panic!("not open");
            };
            let next = |n: u32| {
                SocketIn::Message(Frame::Text(format!(
                    "{{\"id\":\"1\",\"type\":\"next\",\"payload\":{{\"data\":{{\"n\":{n}}}}}}}"
                )))
            };
            let (events_tx, mut events) = mpsc::unbounded();
            let (_stop_tx, stop_rx) = oneshot::channel();
            let driving = drive_within(sender, subscription, events_tx, stop_rx, 40);
            let feeding = async {
                told.unbounded_send((next(1), 5)).expect("next");
                told.unbounded_send((next(2), 6)).expect("next");
                // Past the limit: complete, then close.
                let complete = sent.next().await.expect("complete");
                assert!(matches!(complete, SocketOut::Send(Frame::Text(text)) if text.contains("complete")));
                assert_eq!(sent.next().await, Some(SocketOut::Close));
                told.unbounded_send((next(3), 7)).expect("next");
                told.unbounded_send((
                    SocketIn::Closed {
                        code: Some(1000),
                        reason: String::new(),
                    },
                    8,
                ))
                .expect("closed");
            };
            let (response, _) = futures::future::join(driving, feeding).await;
            assert_eq!(
                response.stream.as_ref().map(|stream| stream.ended.clone()),
                Some(StreamEnd::Cut)
            );
            let heard: Vec<String> = std::iter::from_fn(|| events.try_recv().ok())
                .map(|heard| heard.data)
                .collect();
            assert_eq!(heard.len(), 3, "what came while closing is still told: {heard:?}");
        });
    }
}
