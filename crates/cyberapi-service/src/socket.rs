//! E1: a WebSocket API's connection. It is prepared as a send is — the
//! variables, the templates, the jar's cookies, the sign-in, an OAuth token
//! — and opened by the engine (the desktop's alone). One that does not open
//! is kept in the history as any answer; one that does is kept once it
//! ends, with what went each way and when ([`keep`]). Each message sent is
//! the request's body as it is then, its templates rendered ([`message`]).

use crate::oauth;
use crate::send::{SendFailure, Sender, Unanswered, add_cookies, keep_cookies, kept, prepare};
use cyberapi_core::response::{Exchange, HttpResponse, HttpStats, SentRequest, StreamEnd, StreamInfo};
use cyberapi_core::secrets;
use cyberapi_core::send::{self, PreparedRequest};
use cyberapi_core::template::Answers;
use cyberapi_core::ws::{self, Frame};
use cyberapi_core::{CollectionId, ItemId};
use cyberapi_http::{Opened, SocketIn, SocketOut};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use std::time::Duration;

/// What connecting came to.
pub enum Connected {
    /// Open: send on it, hear from it, and [`keep`] it once it ends.
    Open(LiveSocket),
    /// It did not open: the server's answer, or why there was none, kept
    /// in the history.
    Kept(HttpResponse),
}

/// An open connection.
pub struct LiveSocket {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub outgoing: UnboundedSender<SocketOut>,
    /// What comes, with when: milliseconds after it opened.
    pub incoming: UnboundedReceiver<(SocketIn, u64)>,
    pub head: SocketHead,
}

/// What the history keeps of how a connection was opened.
#[derive(Clone)]
pub struct SocketHead {
    collection: CollectionId,
    api: ItemId,
    request: cyberapi_core::model::HttpRequest,
    url: String,
    status: u16,
    headers: Vec<(String, String)>,
    stats: HttpStats,
    sent: SentRequest,
}

/// Opens `request`'s connection — the API's, as the editor holds it.
/// `answers` are the reader's to its `prompt`s: one not answered stops it.
pub async fn connect(
    sender: Sender,
    collection: CollectionId,
    api: ItemId,
    request: cyberapi_core::model::HttpRequest,
    answers: Answers,
) -> Result<Connected, SendFailure> {
    let sender = sender.for_request(&request);
    let (mut prepared, globals) =
        prepare(&sender, &collection, &request, &answers, Unanswered::Stops, None, &[]).await?;
    // The body is the first message's, not the handshake's.
    prepared.body.clear();
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
        return Ok(Connected::Kept(kept(&sender, failed(why)).await));
    }
    let opened = sender.engine.open_socket(prepared.clone()).await;
    let cookies_of = |headers: &[(String, String)]| Exchange {
        headers: headers.to_vec(),
        ..Default::default()
    };
    let http_url = as_http(&prepared).url;
    match opened {
        Ok(Opened::Socket(socket)) => {
            keep_cookies(&sender, &cookies_of(&socket.headers), &http_url).await;
            Ok(Connected::Open(LiveSocket {
                status: socket.status,
                headers: socket.headers.clone(),
                outgoing: socket.outgoing,
                incoming: socket.incoming,
                head: SocketHead {
                    collection,
                    api,
                    request,
                    url: shown_url,
                    status: socket.status,
                    headers: socket.headers,
                    stats: socket.stats,
                    sent,
                },
            }))
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
            Ok(Connected::Kept(kept(&sender, response).await))
        }
        Err(error) => Ok(Connected::Kept(kept(&sender, failed(error.to_string())).await)),
    }
}

/// The request as HTTP, for its cookies: `ws` as `http`, `wss` as `https`.
fn as_http(prepared: &PreparedRequest) -> PreparedRequest {
    PreparedRequest {
        url: ws::http_url(&prepared.url).unwrap_or_else(|| prepared.url.clone()),
        ..prepared.clone()
    }
}

/// The message to send now: `request`'s body, its templates rendered as
/// sending renders them.
pub async fn message(
    sender: Sender,
    collection: CollectionId,
    request: cyberapi_core::model::HttpRequest,
    answers: Answers,
) -> Result<String, SendFailure> {
    let sender = sender.for_request(&request);
    let (prepared, _) = prepare(&sender, &collection, &request, &answers, Unanswered::Stops, None, &[]).await?;
    Ok(String::from_utf8_lossy(&prepared.body).into_owned())
}

/// The connection, once it ended, in the API's history: how it opened,
/// `messages` — `(sent, frame, when)` — and how it ended.
pub async fn keep(
    sender: Sender,
    head: SocketHead,
    messages: Vec<(bool, Frame, u64)>,
    ended: StreamEnd,
) -> HttpResponse {
    let (body, arrivals) = ws::transcript(&messages);
    let lasted = messages.last().map_or(0, |(_, _, at)| *at);
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
    kept(&sender, response).await
}

/// How a connection ended, from what it last told.
pub fn ending(last: &SocketIn, stopped: bool) -> StreamEnd {
    match last {
        SocketIn::Failed(message) => StreamEnd::Failed {
            message: message.clone(),
        },
        _ if stopped => StreamEnd::Stopped,
        _ => StreamEnd::Closed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cyberapi_core::auth::Auth;
    use cyberapi_core::cookie::Cookie;
    use cyberapi_core::model::{HttpMethod, HttpRequest};
    use cyberapi_core::prefs::{SendPrefs, Timeouts};
    use cyberapi_core::response::HeaderSource;
    use cyberapi_core::template::{NoEnv, NoFiles};
    use cyberapi_http::{HttpEngine, OpenFuture, SendError, SendFuture, Socket};
    use cyberapi_store::{MemoryStore, NewCollection, Storage};
    use futures::StreamExt as _;
    use futures::channel::mpsc;
    use pretty_assertions::assert_eq;
    use std::sync::{Arc, Mutex};

    /// Opens what goes to `/chat`, refuses the rest with a 403, and keeps
    /// what it was asked; the server's side of an open one is kept too.
    #[derive(Default)]
    struct Engine {
        asked: Mutex<Vec<PreparedRequest>>,
        server: Mutex<Option<Server>>,
    }

    /// The server's side of an open socket: what was sent, and its telling.
    type Server = (
        mpsc::UnboundedReceiver<SocketOut>,
        mpsc::UnboundedSender<(SocketIn, u64)>,
    );

    impl HttpEngine for Engine {
        fn send(&self, _: PreparedRequest) -> SendFuture {
            Box::pin(async { Err(SendError::Invalid { message: "no".into() }) })
        }

        fn open_socket(&self, request: PreparedRequest) -> OpenFuture {
            let opens = request.url.ends_with(".test/chat");
            self.asked.lock().expect("asked").push(request);
            if !opens {
                return Box::pin(async {
                    Ok(Opened::Refused(Exchange {
                        status: 403,
                        body: b"no".to_vec(),
                        ..Default::default()
                    }))
                });
            }
            let (outgoing, sent) = mpsc::unbounded();
            let (told, incoming) = mpsc::unbounded();
            *self.server.lock().expect("server") = Some((sent, told));
            Box::pin(async move {
                Ok(Opened::Socket(Socket {
                    status: 101,
                    headers: vec![("set-cookie".into(), "seen=1".into())],
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
            .create_api(collection.clone(), None, "chat".into())
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
            method: HttpMethod::WebSocket,
            uri: uri.into(),
            auth: Auth::Bearer { token: "t".into() },
            body: "hello {{ts()}}".into(),
            ..Default::default()
        }
    }

    /// E1: opened with the jar's cookies and the sign-in, the body left for
    /// the messages; once it ends, kept with what went each way and how it
    /// ended, read back as events.
    #[test]
    fn a_connection_is_kept_once_it_ends() {
        pollster::block_on(async {
            let (storage, engine, sender, collection, api) = setup().await;
            storage
                .cookies()
                .put(vec![Cookie {
                    name: "sid".into(),
                    value: "s".into(),
                    domain: "chat.test".into(),
                    path: "/".into(),
                    expires: None,
                    secure: false,
                    http_only: false,
                    host_only: true,
                }])
                .await
                .expect("cookie");
            let connected = connect(
                sender.clone(),
                collection.clone(),
                api.clone(),
                request("ws://chat.test/chat"),
                Answers::default(),
            )
            .await
            .expect("connected");
            let Connected::Open(mut socket) = connected else {
                panic!("not opened");
            };
            let asked = engine.asked.lock().expect("asked")[0].clone();
            assert_eq!(asked.url, "ws://chat.test/chat");
            assert_eq!(asked.header("authorization"), Some("Bearer t"));
            assert_eq!(asked.header("cookie"), Some("sid=s"), "the jar's, as for http://");
            assert!(asked.body.is_empty(), "the body is the messages'");
            let jar = storage.cookies().list().await.expect("jar");
            assert!(jar.iter().any(|cookie| cookie.name == "seen"), "the handshake's kept");

            // A message: the body as it is now, rendered.
            let said = message(
                sender.clone(),
                collection.clone(),
                request("ws://chat.test/chat"),
                Answers::default(),
            )
            .await
            .expect("rendered");
            assert!(said.starts_with("hello ") && !said.contains("{{"), "{said}");
            socket
                .outgoing
                .unbounded_send(SocketOut::Send(Frame::Text(said.clone())))
                .expect("open");
            let (mut sent, told) = engine.server.lock().expect("server").take().expect("open");
            assert_eq!(sent.next().await, Some(SocketOut::Send(Frame::Text(said.clone()))));
            told.unbounded_send((SocketIn::Message(Frame::Text("hi".into())), 20))
                .expect("open");
            let (heard, at) = socket.incoming.next().await.expect("heard");
            assert_eq!((heard, at), (SocketIn::Message(Frame::Text("hi".into())), 20));

            let ended = ending(
                &SocketIn::Closed {
                    code: Some(1000),
                    reason: String::new(),
                },
                true,
            );
            assert_eq!(ended, StreamEnd::Stopped);
            let kept = keep(
                sender.clone(),
                socket.head,
                vec![
                    (true, Frame::Text(said.clone()), 10),
                    (false, Frame::Text("hi".into()), 20),
                ],
                ended,
            )
            .await;
            let history = storage.history().list(collection, api).await.expect("history");
            assert_eq!(history.len(), 1);
            assert_eq!(history[0].id, kept.id);
            assert_eq!(kept.status, Some(101));
            assert!(kept.is_stream());
            let events = kept.stream_events();
            assert_eq!(
                events
                    .iter()
                    .map(|event| (event.event.as_str(), event.data.as_str()))
                    .collect::<Vec<_>>(),
                [(ws::SENT, said.as_str()), (ws::RECEIVED, "hi")]
            );
            assert_eq!(
                kept.stream.as_ref().map(|stream| &stream.ended),
                Some(&StreamEnd::Stopped)
            );
            let auth = kept
                .sent
                .expect("sent")
                .headers
                .into_iter()
                .find(|h| h.name == "Authorization");
            assert_eq!(auth.map(|h| h.from), Some(HeaderSource::Auth));
        });
    }

    /// One the server would not open is kept as its answer; one the engine
    /// cannot open (a page's), with why.
    #[test]
    fn one_that_does_not_open_is_kept_as_any_answer() {
        pollster::block_on(async {
            let (storage, _, sender, collection, api) = setup().await;
            let connected = connect(
                sender.clone(),
                collection.clone(),
                api.clone(),
                request("ws://chat.test/elsewhere"),
                Answers::default(),
            )
            .await
            .expect("an answer");
            let Connected::Kept(refused) = connected else {
                panic!("opened");
            };
            assert_eq!((refused.status, refused.body.as_slice()), (Some(403), b"no".as_slice()));

            struct Page;
            impl HttpEngine for Page {
                fn send(&self, _: PreparedRequest) -> SendFuture {
                    Box::pin(async { Err(SendError::Invalid { message: "no".into() }) })
                }
            }
            let page = Sender {
                engine: Arc::new(Page),
                ..sender
            };
            let Connected::Kept(unopened) = connect(
                page,
                collection.clone(),
                api.clone(),
                request("ws://chat.test/chat"),
                Answers::default(),
            )
            .await
            .expect("an answer") else {
                panic!("opened");
            };
            assert!(
                unopened
                    .error
                    .as_deref()
                    .is_some_and(|error| error.contains("desktop app")),
                "{:?}",
                unopened.error
            );
            assert_eq!(storage.history().list(collection, api).await.expect("history").len(), 2);
        });
    }
}
