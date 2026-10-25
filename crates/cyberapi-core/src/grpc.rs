//! gRPC unary calls (`HttpMethod::Grpc`): what the editor keeps for one —
//! the service, the method, the request message as JSON and the definitions
//! it is encoded with — and the wire around it, after gRPC over HTTP/2: a
//! POST to `/package.Service/Method`, each message framed by a flag byte and
//! its length, the call's status in the `grpc-status` and `grpc-message`
//! trailers (in the headers, when the server answers with no body).
//!
//! A server-streaming method's reply is many messages: read as they come
//! ([`ReplyReader`]), and kept one a line ([`GrpcBody::stream_reply`]),
//! each with when it came. A client-streaming method is not called.
//!
//! The definitions are a `FileDescriptorSet`, from a server's reflection
//! ([`reflection`]) or compiled from `.proto` files. An API keeps the files
//! its service needs, so it is sent — on the desktop, in a page, by a
//! teammate — without asking the server for them again.

use crate::model::HttpMethod;
use crate::response::StreamInfo;
use crate::send::{PreparedRequest, strip_json_comments};
use crate::sse::Event;
use base64::Engine as _;
use prost::Message as _;
use prost_reflect::{
    DescriptorPool, DynamicMessage, FileDescriptor, Kind, MessageDescriptor, MethodDescriptor, SerializeOptions,
    Value as Field,
};
use prost_types::FileDescriptorSet;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

/// What `HttpRequest.body` keeps for a gRPC call.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GrpcBody {
    /// The service's full name: `helloworld.Greeter`.
    pub service: String,
    pub method: String,
    /// The request message in protobuf's JSON mapping; its template
    /// functions run before it is read.
    pub message: String,
    /// The definitions it is encoded with: a `FileDescriptorSet`, base64 —
    /// the service's file and the files it imports.
    pub descriptors: String,
    /// Where they came from: [`FROM_REFLECTION`], or the `.proto` files'
    /// names.
    pub source: String,
}

/// [`GrpcBody::source`] for definitions a server's reflection gave.
pub const FROM_REFLECTION: &str = "reflection";

impl GrpcBody {
    /// The body as stored. Text that is not one (the JSON body of the
    /// method the reader just switched from) is taken as the message.
    pub fn read(body: &str) -> Self {
        if body.trim().is_empty() {
            return Self::default();
        }
        if let Ok(Value::Object(fields)) = serde_json::from_str::<Value>(body)
            && ["service", "method", "message", "descriptors"]
                .iter()
                .any(|key| fields.contains_key(*key))
        {
            return serde_json::from_value(Value::Object(fields)).unwrap_or_default();
        }
        Self {
            message: body.to_string(),
            ..Self::default()
        }
    }

    pub fn to_body(&self) -> String {
        // A struct of strings cannot fail to serialize.
        serde_json::to_string(self).unwrap_or_default()
    }

    /// The call's path: `/package.Service/Method`.
    pub fn path(&self) -> String {
        format!("/{}/{}", self.service.trim(), self.method.trim())
    }

    /// The definitions it keeps, if it keeps any.
    pub fn definitions(&self) -> Option<Result<Definitions, String>> {
        (!self.descriptors.trim().is_empty()).then(|| Definitions::from_base64(&self.descriptors))
    }

    /// The method it calls in its definitions, or why it calls none.
    pub fn method_in(&self, definitions: &Definitions) -> Result<MethodDescriptor, String> {
        if self.service.trim().is_empty() || self.method.trim().is_empty() {
            return Err("choose the gRPC method to call".into());
        }
        let method = definitions
            .find(self.service.trim(), self.method.trim())
            .ok_or_else(|| format!("{} is not in the API's definitions", self.path()))?;
        if method.is_client_streaming() {
            return Err(format!(
                "{} streams from the client; only unary and server-streaming calls are made",
                self.path()
            ));
        }
        Ok(method)
    }

    /// Whether the method it calls answers with a stream of messages.
    pub fn server_streams(&self) -> bool {
        self.definitions()
            .and_then(Result::ok)
            .and_then(|definitions| self.method_in(&definitions).ok())
            .is_some_and(|method| method.is_server_streaming())
    }

    /// The type of the messages the method it calls answers with, by its
    /// own name.
    pub fn reply_type(&self) -> Option<String> {
        let definitions = self.definitions()?.ok()?;
        let method = self.method_in(&definitions).ok()?;
        Some(method.output().name().to_string())
    }

    /// A reader of the replies of the server-streaming method it calls.
    pub fn reply_reader(&self) -> Option<ReplyReader> {
        let definitions = self.definitions()?.ok()?;
        let method = self.method_in(&definitions).ok()?;
        method.is_server_streaming().then(|| ReplyReader {
            output: method.output(),
            pending: Vec::new(),
        })
    }

    /// A server-streaming reply as the history keeps it: its messages read
    /// as JSON, one a line, and — when its pieces' arrivals were kept — when
    /// each came. A message cut short by the stream's end is left out.
    /// `Err` says why a message cannot be read.
    pub fn stream_reply(
        &self,
        body: &[u8],
        stream: Option<&StreamInfo>,
    ) -> Result<(Vec<u8>, Option<StreamInfo>), String> {
        let mut reader = self
            .reply_reader()
            .ok_or("the method does not stream, or the API has no definitions for it")?;
        let mut messages: Vec<Event> = Vec::new();
        let mut from = 0;
        for &(end, at) in stream.map(|stream| stream.arrivals.as_slice()).unwrap_or_default() {
            let end = (end as usize).min(body.len());
            if end > from {
                messages.extend(reader.feed(&body[from..end], Some(at)));
                from = end;
            }
        }
        messages.extend(reader.feed(&body[from..], None));
        let mut kept = Vec::new();
        let mut arrivals = Vec::new();
        for message in messages {
            if message.event == UNREADABLE {
                return Err(message.data);
            }
            kept.extend_from_slice(message.data.as_bytes());
            kept.push(b'\n');
            arrivals.push((kept.len() as u64, message.at.unwrap_or_default()));
        }
        let stream = stream.map(|stream| StreamInfo {
            arrivals,
            ended: stream.ended.clone(),
        });
        Ok((kept, stream))
    }

    /// The request as it goes out: the message, its templates already run
    /// (`message`), encoded and framed. `Err` says why it cannot be.
    pub fn request(&self, message: &str) -> Result<Vec<u8>, String> {
        let definitions = self
            .definitions()
            .ok_or("the API has no gRPC definitions: load them from the server's reflection or a .proto file")?
            .map_err(|error| format!("the API's gRPC definitions cannot be read: {error}"))?;
        let method = self.method_in(&definitions)?;
        let input = method.input();
        let text = strip_json_comments(message);
        let text = if text.trim().is_empty() { "{}" } else { text.as_str() };
        let mut reader = serde_json::Deserializer::from_str(text);
        let message = DynamicMessage::deserialize(input.clone(), &mut reader)
            .and_then(|message| reader.end().map(|()| message))
            .map_err(|error| format!("the message is not a {}: {error}", input.full_name()))?;
        Ok(frame(&message.encode_to_vec()))
    }

    /// The reply in `body` read as JSON: `None` when there is no message
    /// (a call that failed). `Err` says why the bytes cannot be read.
    pub fn reply(&self, body: &[u8]) -> Result<Option<String>, String> {
        let definitions = self.definitions().ok_or("the API has no gRPC definitions")??;
        let method = self.method_in(&definitions)?;
        let Some(message) = unframe(body)?.into_iter().next() else {
            return Ok(None);
        };
        let output = method.output();
        let message = DynamicMessage::decode(output.clone(), message)
            .map_err(|error| format!("the reply is not a {}: {error}", output.full_name()))?;
        to_json(&message).map(Some)
    }

    /// The message the prepared request carries, as one line of JSON: what
    /// a grpcurl command sends.
    fn sent_message(&self, framed: &[u8]) -> Option<String> {
        let definitions = self.definitions()?.ok()?;
        let method = self.method_in(&definitions).ok()?;
        let message = unframe(framed).ok()?.into_iter().next()?;
        let message = DynamicMessage::decode(method.input(), message).ok()?;
        let value = message
            .serialize_with_options(serde_json::value::Serializer, &SerializeOptions::new())
            .ok()?;
        Some(value.to_string())
    }
}

/// The body a request keeps as its method changes to or from gRPC: the
/// body becomes the call's message, the message the body. What the reader
/// wrote stays; a call's definitions go with it.
pub fn switched_body(body: &str, from: HttpMethod, to: HttpMethod) -> String {
    match (from == HttpMethod::Grpc, to == HttpMethod::Grpc) {
        (false, true) if !body.trim().is_empty() => GrpcBody {
            message: body.to_string(),
            ..GrpcBody::default()
        }
        .to_body(),
        (true, false) => GrpcBody::read(body).message,
        _ => body.to_string(),
    }
}

/// The type of the event a message that cannot be read becomes, its data
/// why.
pub const UNREADABLE: &str = "unreadable";

/// Reads a server-streaming reply as it comes: each message, once all of
/// it is in, as an event of its type whose data is the message as one
/// line of JSON.
pub struct ReplyReader {
    output: MessageDescriptor,
    /// The bytes of a message not all in yet.
    pending: Vec<u8>,
}

impl ReplyReader {
    /// The messages `piece` completes; `at` is when it came.
    pub fn feed(&mut self, piece: &[u8], at: Option<u64>) -> Vec<Event> {
        self.pending.extend_from_slice(piece);
        let mut events = Vec::new();
        while let Some(head) = self.pending.get(..5) {
            let length = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
            if head[0] != 0 {
                self.pending.clear();
                events.push(unreadable("the server sent a compressed message".into(), at));
                break;
            }
            if self.pending.len() < 5 + length {
                break;
            }
            let message: Vec<u8> = self.pending.drain(..5 + length).skip(5).collect();
            let output = &self.output;
            let read = DynamicMessage::decode(output.clone(), message.as_slice())
                .map_err(|error| format!("the reply is not a {}: {error}", output.full_name()))
                .and_then(|message| {
                    let options = SerializeOptions::new().skip_default_fields(false);
                    message
                        .serialize_with_options(serde_json::value::Serializer, &options)
                        .map(|value| value.to_string())
                        .map_err(|error| error.to_string())
                });
            events.push(match read {
                Ok(data) => Event {
                    event: output.name().to_string(),
                    data,
                    at,
                    ..Default::default()
                },
                Err(why) => unreadable(why, at),
            });
        }
        events
    }
}

fn unreadable(why: String, at: Option<u64>) -> Event {
    Event {
        event: UNREADABLE.into(),
        data: why,
        at,
        ..Default::default()
    }
}

/// `message` in protobuf's JSON mapping, indented as the app indents JSON,
/// with the fields it leaves at their defaults: an answer of `0` or `[]`
/// is an answer.
fn to_json(message: &DynamicMessage) -> Result<String, String> {
    let options = SerializeOptions::new().skip_default_fields(false);
    let value = message
        .serialize_with_options(serde_json::value::Serializer, &options)
        .map_err(|error| error.to_string())?;
    Ok(crate::format::json(&value))
}

/// A message on the wire: not compressed, its length, its bytes.
pub fn frame(message: &[u8]) -> Vec<u8> {
    let mut framed = Vec::with_capacity(message.len() + 5);
    framed.push(0);
    framed.extend_from_slice(&(message.len() as u32).to_be_bytes());
    framed.extend_from_slice(message);
    framed
}

/// The messages of a body. A compressed one is refused: the request does
/// not say it takes any.
pub fn unframe(body: &[u8]) -> Result<Vec<&[u8]>, String> {
    let mut messages = Vec::new();
    let mut rest = body;
    while !rest.is_empty() {
        let Some((head, after)) = rest.split_at_checked(5) else {
            return Err("the body ends inside a message's header".into());
        };
        if head[0] != 0 {
            return Err("the server sent a compressed message".into());
        }
        let length = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
        let Some((message, after)) = after.split_at_checked(length) else {
            return Err("the body ends inside a message".into());
        };
        messages.push(message);
        rest = after;
    }
    Ok(messages)
}

/// The URL a gRPC address names: `host:port` and `grpc://` are plain
/// HTTP/2, `grpcs://` is over TLS, `http://` and `https://` as written.
pub fn address_url(address: &str) -> String {
    let address = address.trim();
    if let Some(rest) = address.strip_prefix("grpcs://") {
        format!("https://{rest}")
    } else if let Some(rest) = address.strip_prefix("grpc://") {
        format!("http://{rest}")
    } else if address.contains("://") {
        address.to_string()
    } else {
        format!("http://{address}")
    }
}

/// How a call ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrpcStatus {
    pub code: u32,
    /// `grpc-message`, decoded.
    pub message: String,
}

impl GrpcStatus {
    /// The status the headers end with — the trailers come after the
    /// headers — or none when the server gave none.
    pub fn read(headers: &[(String, String)]) -> Option<Self> {
        let last = |name: &str| {
            headers
                .iter()
                .rev()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.trim())
        };
        let code = last("grpc-status")?.parse().ok()?;
        let message = last("grpc-message")
            .map(|text| {
                percent_encoding::percent_decode_str(text)
                    .decode_utf8_lossy()
                    .into_owned()
            })
            .unwrap_or_default();
        Some(Self { code, message })
    }

    pub fn is_ok(&self) -> bool {
        self.code == 0
    }

    /// The code's name in the gRPC specification.
    pub fn name(&self) -> &'static str {
        match self.code {
            0 => "OK",
            1 => "CANCELLED",
            2 => "UNKNOWN",
            3 => "INVALID_ARGUMENT",
            4 => "DEADLINE_EXCEEDED",
            5 => "NOT_FOUND",
            6 => "ALREADY_EXISTS",
            7 => "PERMISSION_DENIED",
            8 => "RESOURCE_EXHAUSTED",
            9 => "FAILED_PRECONDITION",
            10 => "ABORTED",
            11 => "OUT_OF_RANGE",
            12 => "UNIMPLEMENTED",
            13 => "INTERNAL",
            14 => "UNAVAILABLE",
            15 => "DATA_LOSS",
            16 => "UNAUTHENTICATED",
            _ => "UNKNOWN_CODE",
        }
    }
}

/// A method the definitions offer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MethodInfo {
    /// The service's full name.
    pub service: String,
    pub method: String,
    pub input: String,
    pub output: String,
    /// It takes a stream of messages: it is not called.
    pub client_streaming: bool,
    /// It answers with a stream of messages.
    pub server_streaming: bool,
}

/// The services and messages of `.proto` files, compiled.
#[derive(Clone, Debug)]
pub struct Definitions(DescriptorPool);

impl Definitions {
    /// From an encoded `FileDescriptorSet`, its files in any order.
    pub fn decode(set: &[u8]) -> Result<Self, String> {
        DescriptorPool::decode(set).map(Self).map_err(|error| error.to_string())
    }

    pub fn from_base64(text: &str) -> Result<Self, String> {
        let set = base64::engine::general_purpose::STANDARD
            .decode(text.trim())
            .map_err(|error| error.to_string())?;
        Self::decode(&set)
    }

    fn find(&self, service: &str, method: &str) -> Option<MethodDescriptor> {
        self.0
            .get_service_by_name(service)?
            .methods()
            .find(|found| found.name() == method)
    }

    /// Every method, its services by name and its methods as declared; the
    /// reflection service's own left out.
    pub fn methods(&self) -> Vec<MethodInfo> {
        let mut services: Vec<_> = self
            .0
            .services()
            .filter(|service| !service.full_name().starts_with("grpc.reflection."))
            .collect();
        services.sort_by(|a, b| a.full_name().cmp(b.full_name()));
        services
            .iter()
            .flat_map(|service| {
                service.methods().map(|method| MethodInfo {
                    service: service.full_name().to_string(),
                    method: method.name().to_string(),
                    input: method.input().full_name().to_string(),
                    output: method.output().full_name().to_string(),
                    client_streaming: method.is_client_streaming(),
                    server_streaming: method.is_server_streaming(),
                })
            })
            .collect()
    }

    /// What an API calling `service` keeps: its file and the files that
    /// imports, imports first, as [`GrpcBody::descriptors`].
    pub fn kept_for(&self, service: &str) -> Option<String> {
        fn visit(file: FileDescriptor, seen: &mut HashSet<String>, files: &mut Vec<prost_types::FileDescriptorProto>) {
            if !seen.insert(file.name().to_string()) {
                return;
            }
            for import in file.dependencies() {
                visit(import, seen, files);
            }
            files.push(file.file_descriptor_proto().clone());
        }
        let file = self.0.get_service_by_name(service)?.parent_file();
        let mut files = Vec::new();
        visit(file, &mut HashSet::new(), &mut files);
        let set = FileDescriptorSet { file: files };
        Some(base64::engine::general_purpose::STANDARD.encode(set.encode_to_vec()))
    }

    /// A request message to start from: every field, at its default, a
    /// nested message filled in too (three levels down), a list with one
    /// entry, the first member of each `oneof`.
    pub fn example(&self, service: &str, method: &str) -> Option<String> {
        let method = self.find(service, method)?;
        to_json(&example(&method.input(), 0)).ok()
    }
}

/// How far down [`Definitions::example`] fills nested messages: a message
/// that holds its own kind would otherwise never end.
const EXAMPLE_DEPTH: usize = 3;

fn example(descriptor: &MessageDescriptor, depth: usize) -> DynamicMessage {
    let mut message = DynamicMessage::new(descriptor.clone());
    let mut oneofs = HashSet::new();
    for field in descriptor.fields() {
        if let Some(oneof) = field.containing_oneof()
            && !oneof.is_synthetic()
            && !oneofs.insert(oneof.name().to_string())
        {
            continue;
        }
        if field.is_map() {
            continue;
        }
        let value = match field.kind() {
            Kind::Message(nested) => {
                // `Any` names its type in a URL, which an example has none
                // of.
                if depth >= EXAMPLE_DEPTH || nested.full_name() == "google.protobuf.Any" {
                    continue;
                }
                Field::Message(example(&nested, depth + 1))
            }
            Kind::Enum(kind) => Field::EnumNumber(kind.default_value().number()),
            kind => Field::default_value(&kind),
        };
        if field.is_list() {
            message.set_field(&field, Field::List(vec![value]));
        } else {
            message.set_field(&field, value);
        }
    }
    message
}

/// The grpcurl command of a call: what [`crate::curl::curl`] gives for one.
/// The definitions come from the server's reflection, which grpcurl asks
/// for itself.
pub fn grpcurl(prepared: &PreparedRequest, body: &GrpcBody) -> String {
    let quote = |text: &str| format!("'{}'", text.replace('\'', r"'\''"));
    let mut parts = vec!["grpcurl".to_string()];
    let url = url::Url::parse(&prepared.url).ok();
    let tls = url.as_ref().is_some_and(|url| url.scheme() == "https");
    if !tls {
        parts.push("-plaintext".into());
    } else if prepared.accept_invalid_certs {
        parts.push("-insecure".into());
    }
    for (name, value) in &prepared.headers {
        if !["content-type", "te", "user-agent"].contains(&name.to_ascii_lowercase().as_str()) {
            parts.push(format!("-H {}", quote(&format!("{name}: {value}"))));
        }
    }
    if let Some(message) = body.sent_message(&prepared.body).filter(|message| message != "{}") {
        parts.push(format!("-d {}", quote(&message)));
    }
    let address = url
        .as_ref()
        .map(|url| {
            let host = url.host_str().unwrap_or_default();
            let port = url.port_or_known_default().unwrap_or(if tls { 443 } else { 80 });
            format!("{host}:{port}")
        })
        .unwrap_or_default();
    parts.push(address);
    parts.push(format!("{}/{}", body.service.trim(), body.method.trim()));
    parts.join(" ")
}

/// gRPC server reflection: asking a server for its services and the files
/// that define them. Version 1, and the `v1alpha` older servers have, take
/// the same messages.
pub mod reflection {
    use super::{Definitions, frame, unframe};
    use prost::Message as _;
    use std::collections::BTreeSet;

    /// The service's paths, newest first: a server without one answers
    /// `UNIMPLEMENTED`.
    pub const PATHS: [&str; 2] = [
        "/grpc.reflection.v1.ServerReflection/ServerReflectionInfo",
        "/grpc.reflection.v1alpha.ServerReflection/ServerReflectionInfo",
    ];

    #[derive(Clone, PartialEq, prost::Message)]
    struct Request {
        #[prost(string, tag = "1")]
        host: String,
        #[prost(oneof = "Ask", tags = "3, 4, 7")]
        ask: Option<Ask>,
    }

    #[derive(Clone, PartialEq, prost::Oneof)]
    enum Ask {
        #[prost(string, tag = "3")]
        FileByFilename(String),
        #[prost(string, tag = "4")]
        FileContainingSymbol(String),
        #[prost(string, tag = "7")]
        ListServices(String),
    }

    #[derive(Clone, PartialEq, prost::Message)]
    struct Response {
        #[prost(oneof = "Answer", tags = "4, 6, 7")]
        answer: Option<Answer>,
    }

    #[derive(Clone, PartialEq, prost::Oneof)]
    enum Answer {
        #[prost(message, tag = "4")]
        Files(FileDescriptorResponse),
        #[prost(message, tag = "6")]
        Services(ListServiceResponse),
        #[prost(message, tag = "7")]
        Error(ErrorResponse),
    }

    #[derive(Clone, PartialEq, prost::Message)]
    struct FileDescriptorResponse {
        #[prost(bytes = "vec", repeated, tag = "1")]
        file_descriptor_proto: Vec<Vec<u8>>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    struct ListServiceResponse {
        #[prost(message, repeated, tag = "1")]
        service: Vec<ServiceResponse>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    struct ServiceResponse {
        #[prost(string, tag = "1")]
        name: String,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    struct ErrorResponse {
        #[prost(int32, tag = "1")]
        error_code: i32,
        #[prost(string, tag = "2")]
        error_message: String,
    }

    fn ask(asks: impl IntoIterator<Item = Ask>) -> Vec<u8> {
        asks.into_iter()
            .flat_map(|ask| {
                let request = Request {
                    host: String::new(),
                    ask: Some(ask),
                };
                frame(&request.encode_to_vec())
            })
            .collect()
    }

    /// The request body that asks for the services.
    pub fn list_services() -> Vec<u8> {
        ask([Ask::ListServices(String::new())])
    }

    /// The request body that asks for the files defining `symbols`, and
    /// what they import.
    pub fn files_containing(symbols: &[String]) -> Vec<u8> {
        ask(symbols.iter().cloned().map(Ask::FileContainingSymbol))
    }

    /// The request body that asks for the files named `names`.
    pub fn files_named(names: &[String]) -> Vec<u8> {
        ask(names.iter().cloned().map(Ask::FileByFilename))
    }

    /// One answer in a reflection reply.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum Reply {
        Services(Vec<String>),
        /// Encoded `FileDescriptorProto`s.
        Files(Vec<Vec<u8>>),
        Error(String),
    }

    pub fn read(body: &[u8]) -> Result<Vec<Reply>, String> {
        unframe(body)?
            .into_iter()
            .map(|message| {
                let response = Response::decode(message).map_err(|error| error.to_string())?;
                Ok(match response.answer {
                    Some(Answer::Services(list)) => {
                        Reply::Services(list.service.into_iter().map(|service| service.name).collect())
                    }
                    Some(Answer::Files(files)) => Reply::Files(files.file_descriptor_proto),
                    Some(Answer::Error(error)) => Reply::Error(error.error_message),
                    None => Reply::Error("an empty answer".into()),
                })
            })
            .collect()
    }

    /// The services worth asking about: not reflection itself.
    pub fn wanted(services: Vec<String>) -> Vec<String> {
        services
            .into_iter()
            .filter(|service| !service.starts_with("grpc.reflection."))
            .collect()
    }

    /// The files gathered from the answers so far.
    #[derive(Default)]
    pub struct Collected {
        files: Vec<prost_types::FileDescriptorProto>,
        names: BTreeSet<String>,
    }

    impl Collected {
        /// One encoded file; one seen before is kept once.
        pub fn add(&mut self, bytes: &[u8]) -> Result<(), String> {
            let file = prost_types::FileDescriptorProto::decode(bytes).map_err(|error| error.to_string())?;
            if self.names.insert(file.name().to_string()) {
                self.files.push(file);
            }
            Ok(())
        }

        /// The files imported and not gathered: a server that sends a file
        /// alone, not what it imports, is asked for them by name.
        pub fn missing(&self) -> Vec<String> {
            let imported: BTreeSet<&String> = self.files.iter().flat_map(|file| &file.dependency).collect();
            imported
                .into_iter()
                .filter(|name| !self.names.contains(*name))
                .cloned()
                .collect()
        }

        pub fn definitions(self) -> Result<Definitions, String> {
            let set = prost_types::FileDescriptorSet { file: self.files };
            Definitions::decode(&set.encode_to_vec())
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use prost_types::{
        DescriptorProto, EnumDescriptorProto, EnumValueDescriptorProto, FieldDescriptorProto, FileDescriptorProto,
        MethodDescriptorProto, ServiceDescriptorProto, field_descriptor_proto::Label, field_descriptor_proto::Type,
    };

    fn field(name: &str, number: i32, kind: Type, label: Label, type_name: Option<&str>) -> FieldDescriptorProto {
        FieldDescriptorProto {
            name: Some(name.into()),
            number: Some(number),
            r#type: Some(kind as i32),
            label: Some(label as i32),
            type_name: type_name.map(str::to_string),
            json_name: Some(name.into()),
            ..Default::default()
        }
    }

    /// `package shop;`
    /// `message Item { string name = 1; int64 count = 2; Kind kind = 3; repeated string tags = 4; Item parent = 5; }`,
    /// `enum Kind { BOOK = 0; FOOD = 1; }`,
    /// `message Ask { string id = 1; }`,
    /// `service Store { rpc Get(Ask) returns (Item); rpc Watch(Ask) returns (stream Item);`
    /// `rpc Upload(stream Ask) returns (Item); }`.
    fn file() -> FileDescriptorProto {
        let optional = Label::Optional;
        FileDescriptorProto {
            name: Some("shop.proto".into()),
            package: Some("shop".into()),
            syntax: Some("proto3".into()),
            message_type: vec![
                DescriptorProto {
                    name: Some("Item".into()),
                    field: vec![
                        field("name", 1, Type::String, optional, None),
                        field("count", 2, Type::Int64, optional, None),
                        field("kind", 3, Type::Enum, optional, Some(".shop.Kind")),
                        field("tags", 4, Type::String, Label::Repeated, None),
                        field("parent", 5, Type::Message, optional, Some(".shop.Item")),
                    ],
                    ..Default::default()
                },
                DescriptorProto {
                    name: Some("Ask".into()),
                    field: vec![field("id", 1, Type::String, optional, None)],
                    ..Default::default()
                },
            ],
            enum_type: vec![EnumDescriptorProto {
                name: Some("Kind".into()),
                value: vec![
                    EnumValueDescriptorProto {
                        name: Some("BOOK".into()),
                        number: Some(0),
                        ..Default::default()
                    },
                    EnumValueDescriptorProto {
                        name: Some("FOOD".into()),
                        number: Some(1),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            service: vec![ServiceDescriptorProto {
                name: Some("Store".into()),
                method: vec![
                    MethodDescriptorProto {
                        name: Some("Get".into()),
                        input_type: Some(".shop.Ask".into()),
                        output_type: Some(".shop.Item".into()),
                        ..Default::default()
                    },
                    MethodDescriptorProto {
                        name: Some("Watch".into()),
                        input_type: Some(".shop.Ask".into()),
                        output_type: Some(".shop.Item".into()),
                        server_streaming: Some(true),
                        ..Default::default()
                    },
                    MethodDescriptorProto {
                        name: Some("Upload".into()),
                        input_type: Some(".shop.Ask".into()),
                        output_type: Some(".shop.Item".into()),
                        client_streaming: Some(true),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn definitions() -> Definitions {
        let set = FileDescriptorSet { file: vec![file()] };
        Definitions::decode(&set.encode_to_vec()).expect("definitions")
    }

    /// A call of `shop.Store`'s `method`, with the definitions.
    pub(crate) fn body(method: &str) -> GrpcBody {
        GrpcBody {
            service: "shop.Store".into(),
            method: method.into(),
            descriptors: definitions().kept_for("shop.Store").expect("kept"),
            source: FROM_REFLECTION.into(),
            ..Default::default()
        }
    }

    #[test]
    fn the_body_reads_back_what_it_keeps() {
        let kept = body("Get");
        assert_eq!(GrpcBody::read(&kept.to_body()), kept);
        assert_eq!(GrpcBody::read(""), GrpcBody::default());
        let switched = GrpcBody::read(r#"{"id": "7"}"#);
        assert_eq!(
            (switched.message.as_str(), switched.service.as_str()),
            (r#"{"id": "7"}"#, "")
        );
    }

    #[test]
    fn the_definitions_offer_their_methods_and_keep_what_a_service_needs() {
        let methods = definitions().methods();
        assert_eq!(
            methods
                .iter()
                .map(|m| (m.method.as_str(), m.client_streaming, m.server_streaming))
                .collect::<Vec<_>>(),
            [("Get", false, false), ("Watch", false, true), ("Upload", true, false)]
        );
        assert_eq!(
            (methods[0].input.as_str(), methods[0].output.as_str()),
            ("shop.Ask", "shop.Item")
        );
        let kept = Definitions::from_base64(&body("Get").descriptors).expect("kept");
        assert_eq!(kept.methods(), methods);
        assert!(definitions().kept_for("shop.Nothing").is_none());
    }

    #[test]
    fn a_message_goes_out_framed_and_a_reply_comes_back_as_json() {
        let call = body("Get");
        let framed = call
            .request("{\n    // the item\n    \"id\": \"7\"\n}")
            .expect("encoded");
        assert_eq!(framed, [0, 0, 0, 0, 3, 0x0a, 1, b'7']);
        assert_eq!(call.request("").expect("empty"), [0, 0, 0, 0, 0]);

        let item = DynamicMessage::decode(
            definitions().find("shop.Store", "Get").expect("Get").output(),
            &[0x0a, 2, b'h', b'i', 0x10, 3, 0x18, 1][..],
        )
        .expect("an item");
        let reply = call.reply(&frame(&item.encode_to_vec())).expect("read");
        let reply: Value = serde_json::from_str(&reply.expect("a message")).expect("JSON");
        assert_eq!(
            reply,
            serde_json::json!({"name": "hi", "count": "3", "kind": "FOOD", "tags": []}),
            "defaults shown, 64-bit numbers as strings, enums by name"
        );
        assert_eq!(call.reply(b""), Ok(None), "a failed call has no message");
    }

    #[test]
    fn what_cannot_be_sent_says_why() {
        let wrong = |call: &GrpcBody, message: &str| call.request(message).expect_err("refused");
        assert!(wrong(&body("Get"), r#"{"nope": 1}"#).contains("is not a shop.Ask"));
        assert!(wrong(&body("Get"), "{").contains("is not a shop.Ask"));
        assert!(wrong(&body("Upload"), "{}").contains("streams from the client"));
        assert!(wrong(&body(""), "{}").contains("choose"));
        assert!(wrong(&body("Gone"), "{}").contains("/shop.Store/Gone is not"));
        let bare = GrpcBody {
            service: "shop.Store".into(),
            method: "Get".into(),
            ..Default::default()
        };
        assert!(wrong(&bare, "{}").contains("no gRPC definitions"));
    }

    /// A server-streaming reply is read a message at a time, whatever the
    /// pieces it comes in, and kept one a line with when each came.
    #[test]
    fn a_streamed_reply_is_read_as_it_comes() {
        let watch = body("Watch");
        assert!(watch.server_streams() && !body("Get").server_streams());
        assert!(body("Get").reply_reader().is_none());
        let item = |name: &[u8]| frame(&[&[0x0a, name.len() as u8][..], name].concat());
        let wire = [item(b"a"), item(b"bc")].concat();
        let mut reader = watch.reply_reader().expect("a reader");
        let (first, rest) = wire.split_at(4);
        assert!(reader.feed(first, Some(1)).is_empty(), "not all of it yet");
        let read = reader.feed(rest, Some(9));
        assert_eq!(
            read.iter().map(|e| (e.event.as_str(), e.at)).collect::<Vec<_>>(),
            [("Item", Some(9)), ("Item", Some(9))]
        );
        let first: Value = serde_json::from_str(&read[0].data).expect("JSON");
        assert_eq!(first["name"], "a");
        assert!(!read[0].data.contains('\n'), "one line");

        let stream = StreamInfo {
            arrivals: vec![(8, 10), (wire.len() as u64, 30)],
            ended: crate::response::StreamEnd::Stopped,
        };
        let cut = [&wire[..], &item(b"zz")[..3]].concat();
        let (kept, kept_stream) = watch.stream_reply(&cut, Some(&stream)).expect("read");
        let lines: Vec<&str> = std::str::from_utf8(&kept).expect("text").lines().collect();
        assert_eq!(lines.len(), 2, "the message cut short is left out");
        let kept_stream = kept_stream.expect("a stream");
        assert_eq!(
            kept_stream.arrivals.iter().map(|(_, at)| *at).collect::<Vec<_>>(),
            [10, 30]
        );
        assert_eq!(kept_stream.arrivals[1].0, kept.len() as u64);
        assert!(watch.stream_reply(&frame(&[0xff]), None).is_err());
    }

    #[test]
    fn an_example_fills_every_field_once() {
        let example: Value =
            serde_json::from_str(&definitions().example("shop.Store", "Get").expect("Get")).expect("JSON");
        assert_eq!(example, serde_json::json!({"id": ""}));
        let defs = definitions();
        let item = defs.find("shop.Store", "Get").expect("Get").output();
        let filled = to_json(&super::example(&item, 0)).expect("JSON");
        let filled: Value = serde_json::from_str(&filled).expect("JSON");
        assert_eq!(filled["tags"], serde_json::json!([""]));
        assert_eq!(filled["kind"], "BOOK");
        assert_eq!(filled["parent"]["parent"]["parent"]["name"], "");
        assert!(
            filled["parent"]["parent"]["parent"].get("parent").is_none(),
            "three levels down"
        );
    }

    #[test]
    fn frames_and_statuses_are_read() {
        let body = [frame(b"ab"), frame(b"")].concat();
        assert_eq!(unframe(&body), Ok(vec![&b"ab"[..], &b""[..]]));
        assert!(unframe(&[0, 0, 0, 0, 9, 1]).is_err());
        assert!(unframe(&[1, 0, 0, 0, 0]).is_err());
        let headers = vec![
            ("content-type".to_string(), "application/grpc".to_string()),
            ("grpc-status".to_string(), "5".to_string()),
            ("grpc-message".to_string(), "no%20such%20item".to_string()),
        ];
        let status = GrpcStatus::read(&headers).expect("a status");
        assert_eq!(
            (status.name(), status.message.as_str(), status.is_ok()),
            ("NOT_FOUND", "no such item", false)
        );
        assert!(GrpcStatus::read(&headers[..1]).is_none());
    }

    #[test]
    fn the_body_turns_into_the_message_and_back() {
        let to = switched_body(r#"{"id": 1}"#, HttpMethod::Post, HttpMethod::Grpc);
        assert_eq!(GrpcBody::read(&to).message, r#"{"id": 1}"#);
        assert_eq!(switched_body(&to, HttpMethod::Grpc, HttpMethod::Post), r#"{"id": 1}"#);
        assert_eq!(switched_body("", HttpMethod::Get, HttpMethod::Grpc), "");
        assert_eq!(switched_body("x", HttpMethod::Get, HttpMethod::Post), "x");
    }

    #[test]
    fn an_address_names_its_url() {
        assert_eq!(address_url("localhost:50051"), "http://localhost:50051");
        assert_eq!(address_url(" grpc://a:1 "), "http://a:1");
        assert_eq!(address_url("grpcs://api.test"), "https://api.test");
        assert_eq!(address_url("https://api.test:8443"), "https://api.test:8443");
    }

    #[test]
    fn the_grpcurl_command_sends_what_the_app_sends() {
        let call = body("Get");
        let prepared = PreparedRequest {
            url: "http://localhost:50051/shop.Store/Get".into(),
            headers: vec![
                ("User-Agent".into(), "CyberAPI".into()),
                ("authorization".into(), "Bearer it's".into()),
                ("content-type".into(), "application/grpc".into()),
            ],
            body: call.request(r#"{"id": "7"}"#).expect("encoded"),
            ..Default::default()
        };
        assert_eq!(
            grpcurl(&prepared, &call),
            r#"grpcurl -plaintext -H 'authorization: Bearer it'\''s' -d '{"id":"7"}' localhost:50051 shop.Store/Get"#
        );
        let tls = PreparedRequest {
            url: "https://api.test/shop.Store/Get".into(),
            accept_invalid_certs: true,
            body: call.request("{}").expect("encoded"),
            ..Default::default()
        };
        assert_eq!(grpcurl(&tls, &call), "grpcurl -insecure api.test:443 shop.Store/Get");
    }

    #[test]
    fn reflection_is_asked_and_its_answers_read() {
        use reflection::{Collected, Reply};
        let asked = reflection::list_services();
        assert_eq!(asked, frame(&[0x3a, 0]), "list_services, empty");
        let named = reflection::files_named(&["a.proto".into(), "b.proto".into()]);
        assert_eq!(unframe(&named).expect("frames").len(), 2);

        // A reply as a server writes it: the services, then a file.
        let services = [0x32, 10, 0x0a, 8, 0x0a, 6, b'a', b'.', b'B', b'e', b'e', b's'];
        let mut file_answer = vec![0x22];
        let encoded = file().encode_to_vec();
        let mut inner = vec![0x0a];
        prost::encoding::encode_varint(encoded.len() as u64, &mut inner);
        inner.extend_from_slice(&encoded);
        prost::encoding::encode_varint(inner.len() as u64, &mut file_answer);
        file_answer.extend_from_slice(&inner);
        let error = [0x3a, 6, 0x08, 5, 0x12, 2, b'n', b'o'];
        let body = [frame(&services), frame(&file_answer), frame(&error)].concat();
        let replies = reflection::read(&body).expect("read");
        assert_eq!(replies[0], Reply::Services(vec!["a.Bees".into()]));
        assert_eq!(replies[2], Reply::Error("no".into()));
        let Reply::Files(files) = &replies[1] else {
            panic!("files: {:?}", replies[1]);
        };

        let mut collected = Collected::default();
        let mut importer = file();
        importer.name = Some("main.proto".into());
        importer.package = Some("main".into());
        importer.dependency = vec!["shop.proto".into()];
        importer.service.clear();
        collected.add(&importer.encode_to_vec()).expect("added");
        assert_eq!(collected.missing(), ["shop.proto"]);
        collected.add(&files[0]).expect("added");
        collected.add(&files[0]).expect("once");
        assert!(collected.missing().is_empty());
        let definitions = collected.definitions().expect("definitions");
        assert_eq!(definitions.methods().len(), 3);
        assert_eq!(
            reflection::wanted(vec!["grpc.reflection.v1.ServerReflection".into(), "shop.Store".into()]),
            ["shop.Store"]
        );
    }
}
