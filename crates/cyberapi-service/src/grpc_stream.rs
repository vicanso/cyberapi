//! E2: a gRPC call that streams from the client. It is prepared as a send
//! is — the variables, the templates, the jar's cookies, the sign-in — and
//! opened by the engine (the desktop's alone) with an empty body. Each
//! message sent after that is the request's message as it is then, its
//! templates rendered and framed ([`message`]). One that does not open is
//! kept in the history as any answer; one that does is kept once it ends,
//! with what went each way ([`keep`]).

use crate::oauth;
use crate::send::{SendFailure, Sender, Unanswered, add_cookies, keep_cookies, kept, prepare};
use cyberapi_core::grpc::{self, GrpcBody, ReplyReader};
use cyberapi_core::model::{ContentType, HttpMethod};
use cyberapi_core::response::{Exchange, HttpResponse, HttpStats, SentRequest, StreamEnd, StreamInfo};
use cyberapi_core::secrets;
use cyberapi_core::send::{self, PrepareError};
use cyberapi_core::template::Answers;
use cyberapi_core::{CollectionId, ItemId};
use cyberapi_http::{CallIn, CallOut, LiveCall};
use futures::channel::mpsc::UnboundedSender;
use std::time::Duration;

/// What connecting came to.
pub enum Connected {
    /// Open: send on it, hear from it, and [`keep`] it once it ends.
    Open(OpenedCall),
    /// It did not open. Kept in the history.
    Kept(HttpResponse),
}

/// An open call.
pub struct OpenedCall {
    pub outgoing: UnboundedSender<CallOut>,
    pub incoming: futures::channel::mpsc::UnboundedReceiver<CallIn>,
    /// Reads the reply messages. `None` when the definitions name no such
    /// method: the bytes are then kept unread.
    pub reader: Option<ReplyReader>,
    /// It streams both ways, so sending does not end before the answer.
    pub bidi: bool,
    pub head: CallHead,
}

/// What the history keeps of how a call was opened. The status and the
/// headers arrive later, with the answer.
#[derive(Clone)]
pub struct CallHead {
    collection: CollectionId,
    api: ItemId,
    request: cyberapi_core::model::HttpRequest,
    /// The address, with secret values masked.
    url: String,
    /// The address the jar matches cookies against.
    cookie_url: String,
    stats: HttpStats,
    sent: SentRequest,
}

/// Opens `request`'s call. The message the editor holds is not sent: it is
/// what [`message`] sends next. `answers` are the reader's to its prompts.
pub async fn connect(
    sender: Sender,
    collection: CollectionId,
    api: ItemId,
    request: cyberapi_core::model::HttpRequest,
    answers: Answers,
) -> Result<Connected, SendFailure> {
    let sender = sender.for_request(&request);
    let grpc = GrpcBody::read(&request.body);
    let (mut prepared, globals) = prepare(
        &sender,
        &collection,
        &request,
        &answers,
        Unanswered::Stops,
        Some((&grpc.path(), Vec::new())),
        &[],
    )
    .await?;
    prepared.body.clear();
    let own_cookie = prepared.header("cookie").map(str::to_string);
    if own_cookie.is_none() {
        add_cookies(&sender, &mut prepared).await?;
    }
    let no_token = oauth::sign(&sender, &mut prepared, false).await.err();
    let jar = prepared.header("cookie").is_some() && prepared.header("cookie") != own_cookie.as_deref();
    let sent = secrets::mask_sent(
        send::sent_request(&prepared, &request, &sender.inherited, &globals, jar, "HTTP/2"),
        &sender.secrets,
    );
    let shown_url = secrets::mask(&prepared.url, &sender.secrets);
    let cookie_url = prepared.url.clone();
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
    match sender.engine.open_call(prepared).await {
        Ok(LiveCall {
            stats,
            outgoing,
            incoming,
        }) => Ok(Connected::Open(OpenedCall {
            outgoing,
            incoming,
            reader: grpc.streaming_reader(),
            bidi: grpc.both_streams(),
            head: CallHead {
                collection,
                api,
                request,
                url: shown_url,
                cookie_url,
                stats,
                sent,
            },
        })),
        Err(error) => Ok(Connected::Kept(kept(&sender, failed(error.to_string())).await)),
    }
}

/// The message to send now: `request`'s gRPC message, its templates
/// rendered, and the same bytes framed for the wire.
pub async fn message(
    sender: Sender,
    collection: CollectionId,
    request: cyberapi_core::model::HttpRequest,
    answers: Answers,
) -> Result<(String, Vec<u8>), SendFailure> {
    let sender = sender.for_request(&request);
    let grpc = GrpcBody::read(&request.body);
    // Rendered as text, so the gRPC encoder is not asked to send the call.
    let mut carrier = request;
    carrier.method = HttpMethod::Post;
    carrier.content_type = ContentType::Plain;
    carrier.body = grpc.message.clone();
    let (prepared, _) = prepare(&sender, &collection, &carrier, &answers, Unanswered::Stops, None, &[]).await?;
    let text = String::from_utf8_lossy(&prepared.body).into_owned();
    let framed = grpc.framed_message(&text).map_err(|message| SendFailure::Prepare {
        source: PrepareError::Grpc { message },
    })?;
    Ok((text, framed))
}

/// The call, once it ended, in the API's history.
pub async fn keep(
    sender: Sender,
    head: CallHead,
    status: Option<u16>,
    headers: Vec<(String, String)>,
    trailers: Vec<(String, String)>,
    messages: Vec<(bool, String, u64)>,
    ended: StreamEnd,
) -> HttpResponse {
    let mut headers = headers;
    headers.extend(trailers);
    keep_cookies(
        &sender,
        &Exchange {
            headers: headers.clone(),
            ..Default::default()
        },
        &head.cookie_url,
    )
    .await;
    let (body, arrivals) = grpc::transcript(&messages);
    let lasted = messages.last().map_or(0, |(_, _, at)| *at);
    let response = HttpResponse {
        api: head.api,
        collection: head.collection,
        request: head.request,
        url: head.url,
        status,
        headers,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::send::send;
    use cyberapi_core::grpc::Definitions;
    use cyberapi_core::model::{HttpRequest, VariableCategory};
    use cyberapi_core::prefs::{SendPrefs, Timeouts};
    use cyberapi_core::template::{NoEnv, NoFiles};
    use cyberapi_http::{CallFuture, HttpEngine, OpenFuture, PreparedRequest, SendError, SendFuture};
    use cyberapi_store::{MemoryStore, NewCollection, NewVariable};
    use futures::channel::mpsc;
    use pretty_assertions::assert_eq;
    use prost::Message as _;
    use prost_types::field_descriptor_proto::{Label, Type};
    use std::sync::{Arc, Mutex};

    /// Opens every call and keeps what it was asked. The server's side is
    /// kept too.
    #[derive(Default)]
    struct Engine {
        asked: Mutex<Vec<PreparedRequest>>,
        server: Mutex<Option<Server>>,
    }

    type Server = (mpsc::UnboundedReceiver<CallOut>, mpsc::UnboundedSender<CallIn>);

    impl HttpEngine for Engine {
        fn send(&self, _: PreparedRequest) -> SendFuture {
            Box::pin(async { Err(SendError::Invalid { message: "no".into() }) })
        }

        fn open_socket(&self, _: PreparedRequest) -> OpenFuture {
            Box::pin(async { Err(SendError::Invalid { message: "no".into() }) })
        }

        fn open_call(&self, request: PreparedRequest) -> CallFuture {
            self.asked.lock().expect("asked").push(request);
            let (outgoing, sent) = mpsc::unbounded();
            let (told, incoming) = mpsc::unbounded();
            *self.server.lock().expect("server") = Some((sent, told));
            Box::pin(async move {
                Ok(LiveCall {
                    stats: HttpStats {
                        version: "HTTP/2".into(),
                        ..Default::default()
                    },
                    outgoing,
                    incoming,
                })
            })
        }
    }

    fn upload() -> HttpRequest {
        let file = prost_types::FileDescriptorProto {
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
                    name: Some("Upload".into()),
                    input_type: Some(".shop.Ask".into()),
                    output_type: Some(".shop.Ask".into()),
                    client_streaming: Some(true),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let set = prost_types::FileDescriptorSet { file: vec![file] };
        let definitions = Definitions::decode(&set.encode_to_vec()).expect("definitions");
        HttpRequest {
            method: HttpMethod::Grpc,
            uri: "grpc://shop.test:50051".into(),
            body: GrpcBody {
                service: "shop.Store".into(),
                method: "Upload".into(),
                message: r#"{"id":"{{value(N)}}"}"#.into(),
                descriptors: definitions.kept_for("shop.Store").expect("kept"),
                source: grpc::FROM_REFLECTION.into(),
            }
            .to_body(),
            ..Default::default()
        }
    }

    /// E2: the call opens with an empty body; a message is framed from the
    /// rendered text; the turns are what the history keeps.
    #[test]
    fn a_client_stream_opens_empty_and_keeps_its_turns() {
        pollster::block_on(async {
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
                .create_api(collection.clone(), None, "upload".into())
                .await
                .expect("api")
                .item
                .id;
            storage
                .variables()
                .create(NewVariable {
                    collection: collection.clone(),
                    category: VariableCategory::Custom,
                    name: "N".into(),
                    value: "7".into(),
                    enabled: true,
                    environment: None,
                })
                .await
                .expect("variable");
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
            let request = upload();
            let refused = send(
                sender.clone(),
                collection.clone(),
                api.clone(),
                request.clone(),
                Answers::default(),
            )
            .await
            .map(|response| response.status);
            assert_eq!(refused, Err(SendFailure::GrpcStream));

            let Connected::Open(opened) = connect(
                sender.clone(),
                collection.clone(),
                api.clone(),
                request.clone(),
                Answers::default(),
            )
            .await
            .expect("connected") else {
                panic!("it opens");
            };
            assert!(!opened.bidi && opened.reader.is_some());
            let asked = engine.asked.lock().expect("asked").pop().expect("asked");
            assert_eq!(asked.url, "http://shop.test:50051/shop.Store/Upload");
            assert!(asked.body.is_empty(), "the message waits");
            assert_eq!(asked.header("content-type"), Some("application/grpc"));
            assert_eq!(asked.header("te"), Some("trailers"));
            assert!(asked.header("content-length").is_none());

            let (text, framed) = message(sender.clone(), collection.clone(), request, Answers::default())
                .await
                .expect("framed");
            assert_eq!(text, r#"{"id":"7"}"#);
            assert_eq!(framed, [0, 0, 0, 0, 3, 0x0a, 1, b'7']);

            let kept = keep(
                sender,
                opened.head,
                Some(200),
                vec![("content-type".into(), "application/grpc".into())],
                vec![("grpc-status".into(), "0".into())],
                vec![(true, text, 4), (false, r#"{"id":"8"}"#.into(), 9)],
                StreamEnd::Closed,
            )
            .await;
            assert!(kept.is_stream());
            assert_eq!(kept.grpc_status().map(|status| status.name()), Some("OK"));
            assert_eq!(kept.body_kind(), cyberapi_core::response::BodyKind::Text);
            let events = kept.stream_events();
            assert_eq!(
                events
                    .iter()
                    .map(|event| (event.event.as_str(), event.data.as_str()))
                    .collect::<Vec<_>>(),
                [("sent", r#"{"id":"7"}"#), ("received", r#"{"id":"8"}"#)]
            );
            assert_eq!(events[1].at, Some(9));
        });
    }
}
