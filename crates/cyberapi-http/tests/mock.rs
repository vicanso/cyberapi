//! G2: the mock server answers from a project's examples, and tells what
//! it answered.

use cyberapi_core::ItemId;
use cyberapi_core::mock::Routes;
use cyberapi_core::model::{ApiSetting, Example, HttpMethod, HttpRequest};
use cyberapi_core::prefs::Timeouts;
use cyberapi_core::tree::TreeData;
use cyberapi_http::{HttpEngine, MockServer, NativeEngine, PreparedRequest, TargetPolicy};
use futures::StreamExt as _;
use pretty_assertions::assert_eq;

fn routes(body: &str) -> Routes {
    Routes::of(&TreeData {
        apis: vec![ApiSetting {
            id: ItemId::new("user"),
            name: "user".into(),
            request: HttpRequest {
                method: HttpMethod::Get,
                uri: "{{HOST}}/users/{{value(id)}}".into(),
                examples: vec![Example {
                    name: "ok".into(),
                    status: 201,
                    headers: vec![("x-trace".into(), "t".into())],
                    body: body.into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..Default::default()
        }],
        ..Default::default()
    })
}

#[tokio::test]
async fn a_mock_server_answers_from_examples() {
    let (heard, mut told) = futures::channel::mpsc::unbounded();
    let server = MockServer::start(tokio::runtime::Handle::current(), 0, routes(r#"{"id":7}"#), heard)
        .await
        .expect("started");
    let address = server.address();
    assert!(address.ip().is_loopback());
    let engine = NativeEngine::new(tokio::runtime::Handle::current(), TargetPolicy::Anywhere);
    let get = |path: &str| PreparedRequest {
        method: HttpMethod::Get,
        url: format!("http://{address}{path}"),
        timeouts: Timeouts::default().effective(),
        ..Default::default()
    };
    let exchange = engine.send(get("/users/7?x=1")).await.expect("an answer");
    assert_eq!(exchange.status, 201);
    assert_eq!(exchange.body, br#"{"id":7}"#);
    assert!(exchange.headers.contains(&("x-trace".into(), "t".into())));
    assert!(
        exchange
            .headers
            .contains(&("access-control-allow-origin".into(), "*".into()))
    );
    let first = told.next().await.expect("told");
    assert_eq!(
        (
            first.method.as_str(),
            first.path.as_str(),
            first.status,
            first.api.as_deref()
        ),
        ("GET", "/users/7?x=1", 201, Some("user"))
    );

    // The project changed: what answers changes with it.
    server.set_routes(routes("changed"));
    let exchange = engine.send(get("/users/8")).await.expect("an answer");
    assert_eq!(exchange.body, b"changed");
    let missing = engine.send(get("/orders")).await.expect("an answer");
    assert_eq!(missing.status, 404);

    // Dropped, it stops.
    drop(server);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(engine.send(get("/users/7")).await.is_err(), "nothing listens");
}

#[tokio::test]
async fn a_port_taken_is_said() {
    let (heard, _told) = futures::channel::mpsc::unbounded();
    let first = MockServer::start(tokio::runtime::Handle::current(), 0, Routes::default(), heard.clone())
        .await
        .expect("started");
    let taken = MockServer::start(
        tokio::runtime::Handle::current(),
        first.address().port(),
        Routes::default(),
        heard,
    )
    .await;
    assert!(taken.is_err(), "the port is taken");
}

/// M1–M3: a body condition picks the example, a delay is waited, and an
/// event stream comes one event at a time.
#[tokio::test]
async fn conditions_delays_and_streams() {
    use cyberapi_core::model::{MockCondition, MockSource};
    let example = |name: &str, body: &str| Example {
        name: name.into(),
        status: 200,
        body: body.into(),
        ..Default::default()
    };
    let mut admin = example("admin", "hi admin");
    admin.when = vec![MockCondition {
        source: MockSource::Body,
        name: "$.role".into(),
        value: "admin".into(),
    }];
    admin.delay_ms = 200;
    let mut stream = example("stream", "data: 1\n\ndata: 2\n\ndata: 3\n\n");
    stream.headers = vec![("Content-Type".into(), "text/event-stream".into())];
    stream.stream_interval_ms = 150;
    let api = |id: &str, method: HttpMethod, uri: &str, examples: Vec<Example>| ApiSetting {
        id: ItemId::new(id),
        name: id.into(),
        request: HttpRequest {
            method,
            uri: uri.into(),
            examples,
            ..Default::default()
        },
        ..Default::default()
    };
    let routes = Routes::of(&TreeData {
        apis: vec![
            api(
                "login",
                HttpMethod::Post,
                "/login",
                vec![admin, example("guest", "hi guest")],
            ),
            api("chat", HttpMethod::Get, "/chat", vec![stream]),
        ],
        ..Default::default()
    });
    let (heard, mut told) = futures::channel::mpsc::unbounded();
    let server = MockServer::start(tokio::runtime::Handle::current(), 0, routes, heard)
        .await
        .expect("started");
    let address = server.address();
    let engine = NativeEngine::new(tokio::runtime::Handle::current(), TargetPolicy::Anywhere);
    let request = |method: HttpMethod, path: &str, body: &str| PreparedRequest {
        method,
        url: format!("http://{address}{path}"),
        headers: vec![("Content-Type".into(), "application/json".into())],
        body: body.as_bytes().to_vec(),
        timeouts: Timeouts::default().effective(),
        ..Default::default()
    };

    let started = std::time::Instant::now();
    let admin = engine
        .send(request(HttpMethod::Post, "/login", r#"{"role":"admin"}"#))
        .await
        .expect("an answer");
    assert_eq!(admin.body, b"hi admin");
    assert!(started.elapsed() >= std::time::Duration::from_millis(200), "it waited");
    assert!(admin.headers.contains(&("x-mock-example-used".into(), "admin".into())));
    assert_eq!(told.next().await.expect("told").example.as_deref(), Some("admin"));

    let guest = engine
        .send(request(HttpMethod::Post, "/login", r#"{"role":"user"}"#))
        .await
        .expect("an answer");
    assert_eq!(guest.body, b"hi guest");

    let started = std::time::Instant::now();
    let streamed = engine
        .send(request(HttpMethod::Get, "/chat", ""))
        .await
        .expect("an answer");
    assert_eq!(streamed.body, b"data: 1\n\ndata: 2\n\ndata: 3\n\n");
    assert!(
        started.elapsed() >= std::time::Duration::from_millis(300),
        "two pauses between three events"
    );
}
