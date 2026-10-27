//! E1: the native engine's WebSockets against a local server.

// tungstenite's handshake callback answers with its error response by value.
#![allow(clippy::result_large_err)]

use cyberapi_core::model::HttpMethod;
use cyberapi_core::prefs::{HttpVersion, Timeouts};
use cyberapi_http::{
    Frame, HttpEngine, NativeEngine, Opened, PreparedRequest, Socket, SocketIn, SocketOut, TargetPolicy,
};
use futures::{SinkExt as _, StreamExt as _};
use pretty_assertions::assert_eq;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

/// A WebSocket server: it greets with the path and the Authorization it
/// was opened with, echoes text, sends `[1, 2]` for `bytes`, closes with
/// 1000 `done` for `bye`; `/no` it will not open.
async fn serve() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut greeting = String::new();
                let callback = |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
                    if request.uri().path() == "/no" {
                        let mut refused = ErrorResponse::new(Some("not here".into()));
                        *refused.status_mut() = http::StatusCode::FORBIDDEN;
                        return Err(refused);
                    }
                    let auth = request
                        .headers()
                        .get("authorization")
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default();
                    greeting = format!("{} {auth}", request.uri());
                    Ok(response)
                };
                let Ok(mut socket) = tokio_tungstenite::accept_hdr_async(stream, callback).await else {
                    return;
                };
                let _ = socket.send(Message::Text(greeting.into())).await;
                while let Some(Ok(message)) = socket.next().await {
                    match message {
                        Message::Text(text) if text.as_str() == "bye" => {
                            let _ = socket
                                .close(Some(CloseFrame {
                                    code: CloseCode::Normal,
                                    reason: "done".into(),
                                }))
                                .await;
                        }
                        Message::Text(text) if text.as_str() == "bytes" => {
                            let _ = socket.send(Message::Binary(vec![1u8, 2].into())).await;
                        }
                        Message::Text(text) => {
                            let _ = socket.send(Message::Text(format!("echo: {text}").into())).await;
                        }
                        _ => {}
                    }
                }
            });
        }
    });
    address
}

fn engine() -> NativeEngine {
    NativeEngine::new(tokio::runtime::Handle::current(), TargetPolicy::Anywhere)
}

fn request(address: SocketAddr, path: &str) -> PreparedRequest {
    PreparedRequest {
        method: HttpMethod::WebSocket,
        url: format!("ws://{address}{path}"),
        headers: vec![("Authorization".into(), "Bearer t".into())],
        timeouts: Timeouts::default().effective(),
        http_version: HttpVersion::Http1,
        ..Default::default()
    }
}

async fn open(address: SocketAddr, path: &str) -> Socket {
    match engine().open_socket(request(address, path)).await.expect("opened") {
        Opened::Socket(socket) => socket,
        Opened::Refused(exchange) => panic!("refused with {}", exchange.status),
    }
}

async fn next(socket: &mut Socket) -> SocketIn {
    socket.incoming.next().await.expect("something comes").0
}

fn text(text: &str) -> SocketIn {
    SocketIn::Message(Frame::Text(text.into()))
}

#[tokio::test]
async fn messages_go_both_ways_till_the_reader_closes() {
    let address = serve().await;
    let mut socket = open(address, "/chat?room=1").await;
    assert_eq!(socket.status, 101);
    assert!(socket.stats.tcp > std::time::Duration::ZERO);
    assert_eq!(
        next(&mut socket).await,
        text("/chat?room=1 Bearer t"),
        "its headers went"
    );

    let say = |socket: &Socket, what: &str| {
        socket
            .outgoing
            .unbounded_send(SocketOut::Send(Frame::Text(what.into())))
            .expect("open");
    };
    say(&socket, "hi");
    assert_eq!(next(&mut socket).await, text("echo: hi"));
    say(&socket, "bytes");
    assert_eq!(next(&mut socket).await, SocketIn::Message(Frame::Binary(vec![1, 2])));

    socket.outgoing.unbounded_send(SocketOut::Close).expect("open");
    assert!(matches!(next(&mut socket).await, SocketIn::Closed { .. }));
    assert!(socket.incoming.next().await.is_none(), "it ends there");
}

#[tokio::test]
async fn the_server_may_close_it() {
    let address = serve().await;
    let mut socket = open(address, "/").await;
    next(&mut socket).await;
    socket
        .outgoing
        .unbounded_send(SocketOut::Send(Frame::Text("bye".into())))
        .expect("open");
    assert_eq!(
        next(&mut socket).await,
        SocketIn::Closed {
            code: Some(1000),
            reason: "done".into()
        }
    );
}

#[tokio::test]
async fn a_server_that_will_not_upgrade_answers_as_any() {
    let address = serve().await;
    match engine().open_socket(request(address, "/no")).await.expect("an answer") {
        Opened::Refused(exchange) => {
            assert_eq!(exchange.status, 403);
            assert_eq!(String::from_utf8_lossy(&exchange.body), "not here");
        }
        Opened::Socket(_) => panic!("opened"),
    }
}

#[tokio::test]
async fn the_address_rules_hold() {
    let address = serve().await;
    let engine = NativeEngine::new(
        tokio::runtime::Handle::current(),
        TargetPolicy::Networks { allowed: Vec::new() },
    );
    let refused = engine.open_socket(request(address, "/")).await;
    assert!(
        matches!(refused, Err(cyberapi_http::SendError::Forbidden { .. })),
        "loopback is not for the bridge"
    );
}
