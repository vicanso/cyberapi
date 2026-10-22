//! The bridge end to end, without a network: requests go straight into its
//! router. Each test makes a database of its own in the PostgreSQL that
//! `CYBERAPI_TEST_POSTGRES` names (whose user may create databases, as the
//! one `make db-up` starts may), and skips without it.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use cyberapi_bridge::Bridge;
use cyberapi_core::model::HttpMethod;
use cyberapi_core::prefs::Timeouts;
use cyberapi_http::{HttpEngine, NativeEngine, PreparedRequest, SendError, TargetPolicy};
use cyberapi_store::{Refusal, StoreError, conformance};
use cyberapi_store_bridge::{BridgeHttpEngine, BridgeStore, Response, STORE_PATH, Transport};
use cyberapi_store_sql::SqlStore;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tower::ServiceExt;

const SETUP_TOKEN: &str = "setup-secret";

/// A bridge on a new, empty database, or `None` when there is no server. It
/// sends as a deployed one does: to public addresses only.
async fn fresh_bridge() -> Option<Bridge> {
    fresh_bridge_sending_to(vec![]).await
}

async fn fresh_bridge_sending_to(allowed: Vec<ipnet::IpNet>) -> Option<Bridge> {
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
    let engine = NativeEngine::new(tokio::runtime::Handle::current(), TargetPolicy::PublicOnly { allowed });
    Some(Bridge::new(store, Arc::new(engine), SETUP_TOKEN.into(), false))
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

/// Two setups at once make one administrator; a name that keeps failing to
/// sign in is turned away for a while, even with the right password; the
/// logins that ran out are forgotten.
#[tokio::test]
async fn one_setup_slow_guesses_and_old_logins() {
    let Some(bridge) = fresh_bridge().await else { return };
    let router = bridge.router(None);
    let form = |name: &str| json!({"token": SETUP_TOKEN, "name": name, "password": "long enough"});
    let (first, second) = tokio::join!(
        send(&router, "POST", "/v1/setup", None, Some(form("one"))),
        send(&router, "POST", "/v1/setup", None, Some(form("two"))),
    );
    let mut statuses = [first.0, second.0];
    statuses.sort();
    assert_eq!(statuses, [StatusCode::OK, StatusCode::CONFLICT]);
    assert_eq!(bridge.store().account_count().await.expect("count"), 1);

    let winner = if first.0 == StatusCode::OK { "one" } else { "two" };
    let login = |name: &str, password: &str| json!({"name": name, "password": password});
    for _ in 0..5 {
        let (status, ..) = send(
            &router,
            "POST",
            "/v1/login",
            None,
            Some(login(winner, "wrong password")),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let (status, _, body) = send(
        &router,
        "POST",
        "/v1/login",
        None,
        Some(login(&winner.to_uppercase(), "long enough")),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");

    let (account, _) = bridge
        .store()
        .find_login(winner)
        .await
        .expect("find")
        .expect("the administrator");
    bridge
        .store()
        .create_session("ran-out", &account.id, 1)
        .await
        .expect("a session");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("after 1970")
        .as_secs();
    let now = i64::try_from(now).expect("a time");
    assert_eq!(
        bridge.store().delete_expired_sessions(now).await.expect("forget"),
        1,
        "the setup's own login lasts"
    );
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

/// A server on this machine that answers every request with `pong`.
async fn pong_server() -> std::net::SocketAddr {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buffer = [0u8; 4096];
                let _ = stream.read(&mut buffer).await;
                let _ = stream
                    .write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: 4\r\n\r\npong")
                    .await;
            });
        }
    });
    address
}

async fn signed_in_router(bridge: &Bridge) -> (Router, String) {
    let router = bridge.router(None);
    let form = json!({"token": SETUP_TOKEN, "name": "admin", "password": "long enough"});
    let (status, set_cookie, body) = send(&router, "POST", "/v1/setup", None, Some(form)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    (router, cookie_pair(&set_cookie.expect("a cookie")))
}

/// The page's engine through the bridge's: sent from the server, for a
/// signed-in account, only where the target policy allows.
#[tokio::test]
async fn requests_are_sent_by_the_bridge() {
    let target = pong_server().await;
    let request = PreparedRequest {
        method: HttpMethod::Get,
        url: format!("http://{target}/ping"),
        timeouts: Timeouts::default().effective(),
        ..Default::default()
    };

    // A deployed bridge does not reach into its own network.
    let Some(bridge) = fresh_bridge().await else { return };
    let (router, cookie) = signed_in_router(&bridge).await;
    let engine = BridgeHttpEngine::new(Arc::new(RouterTransport {
        router: router.clone(),
        cookie: cookie.clone(),
    }));
    assert_eq!(
        engine.send(request.clone()).await,
        Err(SendError::Forbidden {
            target: "127.0.0.1".into()
        })
    );
    // Nobody signed in: nothing is sent.
    let anonymous = BridgeHttpEngine::new(Arc::new(RouterTransport {
        router,
        cookie: String::new(),
    }));
    assert_eq!(
        anonymous.send(request.clone()).await,
        Err(SendError::Bridge {
            message: "not signed in".into()
        })
    );

    // Allowed back by its administrator.
    let Some(bridge) = fresh_bridge_sending_to(vec!["127.0.0.0/8".parse().expect("a range")]).await else {
        return;
    };
    let (router, cookie) = signed_in_router(&bridge).await;
    let engine = BridgeHttpEngine::new(Arc::new(RouterTransport { router, cookie }));
    let exchange = engine.send(request).await.expect("an exchange");
    assert_eq!(exchange.status, 200);
    assert_eq!(exchange.body, b"pong");
    assert_eq!(exchange.stats.remote_addr, target.to_string());
}

/// The directory through the page's store: an administrator's invitation
/// link makes an account (`docs/server-sync.md` §5), which signs in at once
/// and with its password later, and sees its own space.
#[tokio::test]
async fn the_directory_over_the_bridge() {
    let Some(bridge) = fresh_bridge().await else { return };
    let (router, cookie) = signed_in_router(&bridge).await;
    let admin = BridgeStore::new(Arc::new(RouterTransport {
        router: router.clone(),
        cookie,
    }))
    .directory();
    let me = admin.me().await.expect("me");
    assert!(me.account.admin && me.teams.len() == 1 && me.teams[0].personal);
    let made = admin.create_invite(None).await.expect("an invitation");
    assert_eq!(
        admin.invites().await.expect("invites"),
        std::slice::from_ref(&made.invite)
    );

    // What the link says, before anyone signs in.
    let token = json!({"token": made.token});
    let (status, _, body) = send(&router, "POST", "/v1/invite", None, Some(token.clone())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let info: Value = serde_json::from_str(&body).expect("json");
    assert_eq!(info["inviter"], "admin");

    let form =
        |name: &str| json!({"token": made.token, "name": name, "displayName": "Erin", "password": "erins password"});
    let (status, _, _) = send(&router, "POST", "/v1/invite/accept", None, Some(form("admin"))).await;
    assert_eq!(status, StatusCode::CONFLICT, "the name is taken");
    let (status, set_cookie, body) = send(&router, "POST", "/v1/invite/accept", None, Some(form("erin"))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let erin = BridgeStore::new(Arc::new(RouterTransport {
        router: router.clone(),
        cookie: cookie_pair(&set_cookie.expect("signed in")),
    }))
    .directory();
    let me = erin.me().await.expect("me");
    assert_eq!((me.account.label(), me.teams.len()), ("Erin", 1));
    assert_eq!(
        erin.create_invite(None).await,
        Err(StoreError::Refused {
            reason: Refusal::AdminOnly
        })
    );
    // Used once.
    let (status, _, _) = send(&router, "POST", "/v1/invite/accept", None, Some(form("erin2"))).await;
    assert_eq!(status, StatusCode::GONE);
    let (status, _, _) = send(&router, "POST", "/v1/invite", None, Some(token)).await;
    assert_eq!(status, StatusCode::GONE);

    let login = json!({"name": "erin", "password": "erins password"});
    let (status, _, body) = send(&router, "POST", "/v1/login", None, Some(login)).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Taken back, a link opens nothing.
    let revoked = admin.create_invite(None).await.expect("an invitation");
    admin.revoke_invite(revoked.invite.id).await.expect("revoke");
    let form = json!({"token": revoked.token, "name": "fred", "password": "freds password"});
    let (status, _, _) = send(&router, "POST", "/v1/invite/accept", None, Some(form)).await;
    assert_eq!(status, StatusCode::GONE);
    assert!(admin.invites().await.expect("invites").is_empty());
}

/// The desktop's way in (`docs/server-sync.md` step 5): a bearer sign-in
/// over a real socket, through the desktop's own engine. The token comes in
/// the body and no cookie is set; sent back, it is the session, until the
/// desktop signs out.
#[tokio::test]
async fn a_client_of_its_own_signs_in_with_a_bearer_token() {
    use cyberapi_store_bridge::{EngineTransport, LoginForm, SignInError, session_me, sign_in, sign_out};
    let Some(bridge) = fresh_bridge().await else { return };
    let (router, _) = signed_in_router(&bridge).await;

    // No cookie for a bearer client.
    let form = json!({"name": "admin", "password": "long enough", "bearer": true});
    let (status, set_cookie, body) = send(&router, "POST", "/v1/login", None, Some(form)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(set_cookie, None);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = format!("http://{}", listener.local_addr().expect("address"));
    tokio::spawn(async move {
        let service = router.into_make_service_with_connect_info::<std::net::SocketAddr>();
        let _ = axum::serve(listener, service).await;
    });
    let engine: Arc<dyn HttpEngine> = Arc::new(NativeEngine::new(
        tokio::runtime::Handle::current(),
        TargetPolicy::Anywhere,
    ));
    let login = |password: &str| LoginForm {
        name: "admin".into(),
        password: password.into(),
        bearer: false,
    };
    assert_eq!(
        sign_in(engine.clone(), address.clone(), login("wrong one")).await,
        Err(SignInError::WrongLogin)
    );
    let token = sign_in(engine.clone(), address.clone(), login("long enough"))
        .await
        .expect("a token");

    let transport = EngineTransport::new(engine.clone(), address.clone(), Some(token.clone()));
    let me = session_me(&transport).await.expect("an answer").expect("signed in");
    assert!(me.account.admin);
    let storage = BridgeStore::new(Arc::new(transport.clone())).storage();
    assert!(storage.collections().list().await.expect("projects").is_empty());

    sign_out(&transport).await.expect("signed out");
    assert_eq!(
        session_me(&transport).await,
        Ok(None),
        "the token is no session any more"
    );
    let nobody = EngineTransport::new(engine, address, None);
    assert_eq!(session_me(&nobody).await, Ok(None));
}
