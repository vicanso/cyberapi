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
