//! The bridge end to end, without a network: requests go straight into its
//! router. Each test makes a database of its own in the PostgreSQL that
//! `CYBERAPI_TEST_POSTGRES` names (whose user may create databases, as the
//! one `make db-up` starts may), and skips without it.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use cyberapi_bridge::Bridge;
use cyberapi_store::conformance;
use cyberapi_store_bridge::{BridgeStore, Response, STORE_PATH, Transport};
use cyberapi_store_sql::SqlStore;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tower::ServiceExt;

const SETUP_TOKEN: &str = "setup-secret";

/// A bridge on a new, empty database, or `None` when there is no server.
async fn fresh_bridge() -> Option<Bridge> {
    let Ok(url) = std::env::var("CYBERAPI_TEST_POSTGRES") else {
        eprintln!("skipped: CYBERAPI_TEST_POSTGRES is not set");
        return None;
    };
    sqlx::any::install_default_drivers();
    let name = format!("cyberapi_bridge_{}", ulid::Ulid::generate().to_string().to_lowercase());
    let admin = sqlx::AnyPool::connect(&url).await.expect("connect");
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
        .execute(&admin)
        .await
        .expect("create a database");
    let (server, _) = url.rsplit_once('/').expect("a database in the URL");
    let store = SqlStore::connect(&format!("{server}/{name}")).await.expect("connect");
    Some(Bridge::new(store, SETUP_TOKEN.into(), false))
}

/// One request; the status, the `Set-Cookie` value, and the body.
async fn send(
    router: &Router,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Option<String>, String) {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    let request = match body {
        Some(body) => request
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string())),
        None => request.body(Body::empty()),
    }
    .expect("a request");
    let response = router.clone().oneshot(request).await.expect("an answer");
    let status = response.status();
    let set_cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let bytes = response.into_body().collect().await.expect("a body").to_bytes();
    (status, set_cookie, String::from_utf8_lossy(&bytes).into_owned())
}

/// `name=value` out of a `Set-Cookie`.
fn cookie_pair(set_cookie: &str) -> String {
    set_cookie.split(';').next().unwrap_or_default().to_string()
}

#[tokio::test]
async fn first_administrator_sessions_and_sign_in() {
    let Some(bridge) = fresh_bridge().await else { return };
    let router = bridge.router(None);

    let (status, _, body) = send(&router, "GET", "/v1/setup", None, None).await;
    assert_eq!((status, body.as_str()), (StatusCode::OK, r#"{"needed":true}"#));
    let (status, ..) = send(
        &router,
        "POST",
        STORE_PATH,
        None,
        Some(json!({"call": "collectionsList"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "nobody signed in");

    let form = |token: &str, password: &str| json!({"token": token, "name": "admin", "displayName": "The Admin", "password": password});
    let (status, ..) = send(&router, "POST", "/v1/setup", None, Some(form("guess", "long enough"))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, ..) = send(&router, "POST", "/v1/setup", None, Some(form(SETUP_TOKEN, "short"))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, set_cookie, body) = send(
        &router,
        "POST",
        "/v1/setup",
        None,
        Some(form(SETUP_TOKEN, "long enough")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let set_cookie = set_cookie.expect("a cookie");
    assert!(
        set_cookie.contains("HttpOnly") && set_cookie.contains("SameSite=Strict"),
        "{set_cookie}"
    );
    let cookie = cookie_pair(&set_cookie);
    let account: Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        (account["name"].as_str(), account["admin"].as_bool()),
        (Some("admin"), Some(true))
    );

    // The token counts only while there is no account.
    let (_, _, body) = send(&router, "GET", "/v1/setup", None, None).await;
    assert_eq!(body, r#"{"needed":false}"#);
    let (status, ..) = send(
        &router,
        "POST",
        "/v1/setup",
        None,
        Some(form(SETUP_TOKEN, "another one")),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, _, body) = send(&router, "GET", "/v1/session", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("The Admin"), "{body}");

    // Signing out ends that session, not the account.
    let (status, set_cookie, _) = send(&router, "POST", "/v1/logout", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(set_cookie.is_some_and(|c| c.contains("Max-Age=0")));
    let (status, ..) = send(&router, "GET", "/v1/session", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let login = |name: &str, password: &str| json!({"name": name, "password": password});
    let (status, ..) = send(
        &router,
        "POST",
        "/v1/login",
        None,
        Some(login("admin", "wrong password")),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, ..) = send(&router, "POST", "/v1/login", None, Some(login("nobody", "long enough"))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, set_cookie, _) = send(&router, "POST", "/v1/login", None, Some(login("admin", "long enough"))).await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie_pair(&set_cookie.expect("a cookie"));
    let (status, ..) = send(&router, "GET", "/v1/session", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);
}

/// The page's transport, into the router instead of over a network.
struct RouterTransport {
    router: Router,
    cookie: String,
}

impl Transport for RouterTransport {
    fn post(
        &self,
        path: &'static str,
        body: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<Response, String>> + Send>> {
        let (router, cookie) = (self.router.clone(), self.cookie.clone());
        Box::pin(async move {
            let request = Request::post(path)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::COOKIE, cookie)
                .body(Body::from(body))
                .map_err(|e| e.to_string())?;
            let response = router.oneshot(request).await.map_err(|e| e.to_string())?;
            let status = response.status().as_u16();
            let body = response
                .into_body()
                .collect()
                .await
                .map_err(|e| e.to_string())?
                .to_bytes()
                .to_vec();
            Ok(Response { status, body })
        })
    }
}

/// The shared rules, through the whole way a page's call goes: serialized,
/// routed, signed in, dispatched on the account's scoped store, answered.
#[tokio::test]
async fn bridge_store_conforms() {
    let Some(bridge) = fresh_bridge().await else { return };
    let router = bridge.router(None);
    let form = json!({"token": SETUP_TOKEN, "name": "admin", "password": "long enough"});
    let (status, set_cookie, body) = send(&router, "POST", "/v1/setup", None, Some(form)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let cookie = cookie_pair(&set_cookie.expect("a cookie"));

    let store = BridgeStore::new(Arc::new(RouterTransport { router, cookie }));
    conformance::run(&store.storage(), bridge.store()).await;
}
