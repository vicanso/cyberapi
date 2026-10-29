//! The bridge end to end, without a network: requests go straight into its
//! router. Each test makes a database of its own in the PostgreSQL that
//! `CYBERAPI_TEST_POSTGRES` names (whose user may create databases, as the
//! one `make db-up` starts may), and skips without it.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use cyberapi_bridge::Bridge;
use cyberapi_core::auth::Auth;
use cyberapi_core::model::HttpMethod;
use cyberapi_core::prefs::Timeouts;
use cyberapi_core::version::MODEL;
use cyberapi_http::{HttpEngine, NativeEngine, PreparedRequest, SendError, TargetPolicy};
use cyberapi_store::{NewCollection, Refusal, StoreError, conformance};
use cyberapi_store_bridge::{BridgeHttpEngine, BridgeStore, Call, MODEL_HEADER, Response, STORE_PATH, Transport};
use cyberapi_store_sql::SqlStore;
use http_body_util::BodyExt;
use serde_json::{Value, Value as Json, json};
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
    fresh_bridge_at(allowed, String::new()).await
}

async fn fresh_bridge_at(allowed: Vec<ipnet::IpNet>, base_path: String) -> Option<Bridge> {
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
    Some(Bridge::new(
        store,
        Arc::new(engine),
        SETUP_TOKEN.into(),
        false,
        base_path,
    ))
}

/// The status and the `Location` of a GET.
async fn get_location(router: &Router, path: &str) -> (StatusCode, Option<String>) {
    let request = Request::builder().uri(path).body(Body::empty()).expect("a request");
    let response = router.clone().oneshot(request).await.expect("an answer");
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    (response.status(), location)
}

/// Whether the session a `Set-Cookie` carries is still good `after` seconds
/// from now, as the bridge checks it: by the token's SHA-256.
async fn session_lasts(bridge: &Bridge, set_cookie: &str, after: i64) -> bool {
    use sha2::Digest as _;
    let pair = cookie_pair(set_cookie);
    let token = pair.split_once('=').map(|(_, token)| token).unwrap_or_default();
    let hash = hex::encode(sha2::Sha256::digest(token.as_bytes()));
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("a clock")
        .as_secs() as i64;
    bridge
        .store()
        .session_account(&hash, now + after)
        .await
        .expect("a session check")
        .is_some()
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

/// The page's transport, into the router instead of over a network. It
/// says its model as the page's does; `None` is a client from before it
/// said one.
struct RouterTransport {
    router: Router,
    cookie: String,
    model: Option<u32>,
}

impl Transport for RouterTransport {
    fn post(
        &self,
        path: &'static str,
        body: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<Response, String>> + Send>> {
        let (router, cookie, model) = (self.router.clone(), self.cookie.clone(), self.model);
        Box::pin(async move {
            let mut request = Request::post(path)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::COOKIE, cookie);
            if let Some(model) = model {
                request = request.header(MODEL_HEADER, model.to_string());
            }
            let request = request.body(Body::from(body)).map_err(|e| e.to_string())?;
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

    let store = BridgeStore::new(Arc::new(RouterTransport {
        router,
        cookie,
        model: Some(MODEL),
    }));
    conformance::run(&store.storage(), bridge.store()).await;
}

/// A client that keeps projects in an older shape than the bridge's — or
/// does not say, as none did before batch 4 — reads them and keeps its own
/// things, but changes no project (`docs/batch4-design.md` §2.1). The bridge
/// says its own shape in every answer.
#[tokio::test]
async fn an_older_client_reads_but_changes_no_project() {
    let Some(bridge) = fresh_bridge().await else { return };
    let (router, cookie) = signed_in_router(&bridge).await;
    let client = |model: Option<u32>| {
        BridgeStore::new(Arc::new(RouterTransport {
            router: router.clone(),
            cookie: cookie.clone(),
            model,
        }))
        .storage()
    };
    let (current, older, unsaid) = (client(Some(MODEL)), client(Some(MODEL - 1)), client(None));
    let collection = current
        .collections()
        .create(NewCollection {
            name: "shapes".into(),
            description: String::new(),
            owner: None,
            id: None,
        })
        .await
        .expect("create a project");
    let api = current
        .tree()
        .create_api(collection.id.clone(), None, "kept".into())
        .await
        .expect("the current client changes the project");

    for client in [&older, &unsaid] {
        let tree = client.tree().load(collection.id.clone()).await.expect("it reads");
        assert_eq!(tree.apis.len(), 1);
        let refused = client
            .tree()
            .save_request(collection.id.clone(), api.item.id.clone(), Default::default())
            .await;
        assert!(
            matches!(
                refused,
                Err(StoreError::Refused {
                    reason: Refusal::ClientTooOld
                })
            ),
            "{refused:?}"
        );
        client.prefs().app().await.expect("its own things are its own");
    }

    let request = Request::post(STORE_PATH)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, cookie.clone())
        .body(Body::from(serde_json::to_vec(&Call::CollectionsList).expect("a call")))
        .expect("a request");
    let response = router.clone().oneshot(request).await.expect("an answer");
    assert_eq!(
        response
            .headers()
            .get(MODEL_HEADER)
            .and_then(|value| value.to_str().ok()),
        Some(MODEL.to_string().as_str())
    );
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

    // A deployed bridge does not reach its own host.
    let Some(bridge) = fresh_bridge().await else { return };
    let (router, cookie) = signed_in_router(&bridge).await;
    let engine = BridgeHttpEngine::new(Arc::new(RouterTransport {
        router: router.clone(),
        cookie: cookie.clone(),
        model: Some(MODEL),
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
        model: Some(MODEL),
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
    let engine = BridgeHttpEngine::new(Arc::new(RouterTransport {
        router,
        cookie,
        model: Some(MODEL),
    }));
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
        model: Some(MODEL),
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
        model: Some(MODEL),
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
        remember: true,
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

/// Under a base path the page and the API are below it and nowhere else,
/// and the login cookie is kept to it.
#[tokio::test]
async fn a_bridge_under_a_base_path_answers_only_there() {
    let Some(bridge) = fresh_bridge_at(vec![], "/cyberapi".into()).await else {
        return;
    };
    let router = bridge.router(None);

    assert_eq!(
        get_location(&router, "/cyberapi").await,
        (StatusCode::TEMPORARY_REDIRECT, Some("/cyberapi/".into())),
        "the page resolves against its own address, so it needs the slash"
    );
    assert_eq!(
        get_location(&router, "/cyberapi?x=1").await.1,
        Some("/cyberapi/?x=1".into())
    );
    let (status, _, body) = send(&router, "GET", "/cyberapi/", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("<html"), "the page");
    for outside in ["/", "/index.html", "/v1/setup", "/cyberapix/"] {
        let (status, ..) = send(&router, "GET", outside, None, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{outside}");
    }

    let (status, _, body) = send(&router, "GET", "/cyberapi/v1/setup", None, None).await;
    assert_eq!((status, body.as_str()), (StatusCode::OK, r#"{"needed":true}"#));
    let form = json!({"token": SETUP_TOKEN, "name": "admin", "password": "long enough", "remember": false});
    let (status, set_cookie, body) = send(&router, "POST", "/cyberapi/v1/setup", None, Some(form)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let set_cookie = set_cookie.expect("a cookie");
    assert!(set_cookie.contains("Path=/cyberapi/"), "{set_cookie}");

    let (status, set_cookie, _) = send(
        &router,
        "POST",
        "/cyberapi/v1/login",
        None,
        Some(json!({"name": "admin", "password": "long enough"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let set_cookie = set_cookie.expect("a cookie");
    assert!(set_cookie.contains("Path=/cyberapi/"), "{set_cookie}");
    let cookie = cookie_pair(&set_cookie);
    let (status, ..) = send(&router, "GET", "/cyberapi/v1/session", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    let (_, set_cookie, _) = send(&router, "POST", "/cyberapi/v1/logout", Some(&cookie), None).await;
    let set_cookie = set_cookie.expect("a cookie");
    assert!(
        set_cookie.contains("Path=/cyberapi/") && set_cookie.contains("Max-Age=0"),
        "the cookie cleared where it was set: {set_cookie}"
    );
}

/// "Keep me signed in" is on unless the page says otherwise: a kept login
/// lasts its 30 days; one not kept ends with the browser, and on the bridge
/// within the day.
#[tokio::test]
async fn a_login_is_kept_unless_asked_not_to_be() {
    let Some(bridge) = fresh_bridge().await else { return };
    let router = bridge.router(None);
    let form = json!({"token": SETUP_TOKEN, "name": "admin", "password": "long enough"});
    let (status, set_cookie, _) = send(&router, "POST", "/v1/setup", None, Some(form)).await;
    assert_eq!(status, StatusCode::OK);
    let kept = set_cookie.expect("a cookie");
    assert!(
        kept.contains(&format!("Max-Age={}", cyberapi_bridge::KEPT_SESSION)),
        "{kept}"
    );
    assert!(session_lasts(&bridge, &kept, cyberapi_bridge::KEPT_SESSION - 60).await);

    let login = |remember: bool| json!({"name": "admin", "password": "long enough", "remember": remember});
    let (status, set_cookie, _) = send(&router, "POST", "/v1/login", None, Some(login(false))).await;
    assert_eq!(status, StatusCode::OK);
    let unkept = set_cookie.expect("a cookie");
    assert!(!unkept.contains("Max-Age"), "gone when the browser closes: {unkept}");
    assert!(session_lasts(&bridge, &unkept, cyberapi_bridge::UNKEPT_SESSION - 60).await);
    assert!(
        !session_lasts(&bridge, &unkept, cyberapi_bridge::UNKEPT_SESSION + 60).await,
        "and not past the day on the bridge"
    );
    let (status, ..) = send(&router, "GET", "/v1/session", Some(&cookie_pair(&unkept)), None).await;
    assert_eq!(status, StatusCode::OK);

    let (_, set_cookie, _) = send(&router, "POST", "/v1/login", None, Some(login(true))).await;
    assert!(set_cookie.expect("a cookie").contains("Max-Age="));
}

/// One JSON-RPC request to `/v1/mcp`, with the token given; the status and
/// the body as JSON (null when there is none).
async fn mcp(router: &Router, authorization: Option<&str>, cookie: Option<&str>, body: Value) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method("POST")
        .uri("/v1/mcp")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(authorization) = authorization {
        request = request.header(header::AUTHORIZATION, authorization);
    }
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    let request = request.body(Body::from(body.to_string())).expect("a request");
    let response = router.clone().oneshot(request).await.expect("an answer");
    let status = response.status();
    let bytes = response.into_body().collect().await.expect("a body").to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// A tool's call, its text parsed back into JSON when it is JSON, and
/// whether the tool said it failed.
async fn tool(router: &Router, token: &str, name: &str, arguments: Value) -> (Value, bool) {
    let call =
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": arguments}});
    let (status, body) = mcp(router, Some(&format!("Bearer {token}")), None, call).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let result = &body["result"];
    let text = result["content"][0]["text"].as_str().unwrap_or_default();
    let value = serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.into()));
    (value, result["isError"] == json!(true))
}

/// An AI assistant at `/v1/mcp` (zedis-bridge's design): only an MCP token
/// signs in there, the tools read what its account sees with the secrets
/// masked, and only a token that may send sends — where its account edits.
#[tokio::test]
async fn an_assistant_reads_with_a_token_and_sends_only_when_allowed() {
    use cyberapi_core::access::{CollectionRole, Grantee};
    use cyberapi_core::directory::NewMcpToken;
    use cyberapi_core::model::{HttpRequest, KvParam, ProjectSettings, VariableCategory};
    use cyberapi_store::{NewCollection, NewVariable};
    use cyberapi_store_sql::NewAccount;

    let target = pong_server().await;
    let Some(bridge) = fresh_bridge_sending_to(vec!["127.0.0.0/8".parse().expect("a range")]).await else {
        return;
    };
    let (router, cookie) = signed_in_router(&bridge).await;
    let store = bridge.store();
    let account = |name: &str| NewAccount {
        name: name.into(),
        display_name: name.into(),
        password_hash: "unused".into(),
        admin: false,
    };
    let ann = store.create_account(account("ann")).await.expect("ann");
    let bob = store.create_account(account("bob")).await.expect("bob");
    let storage = store.scoped(ann.clone());
    let project = storage
        .collections()
        .create(NewCollection {
            name: "shop".into(),
            description: "orders".into(),
            owner: None,
            id: None,
        })
        .await
        .expect("a project");
    let api = storage
        .tree()
        .create_api(project.id.clone(), None, "ping".into())
        .await
        .expect("an API")
        .item;
    let param = |key: &str, value: &str| KvParam {
        key: key.into(),
        value: value.into(),
        enabled: true,
    };
    storage
        .tree()
        .save_request(
            project.id.clone(),
            api.id.clone(),
            HttpRequest {
                uri: "{{HOST}}/ping".into(),
                headers: vec![param("Authorization", "Bearer s3cret"), param("X-Token", "{{TOKEN}}")],
                body: r#"{"user":"ann","password":"hunter2"}"#.into(),
                auth: Auth::Basic {
                    username: "ann".into(),
                    password: "hunter2".into(),
                },
                // A1.
                assertions: vec![cyberapi_core::assertions::Assertion {
                    expected: "200".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        )
        .await
        .expect("its request");
    for (name, value) in [("HOST", format!("http://{target}")), ("TOKEN", "t0ken".into())] {
        storage
            .variables()
            .create(NewVariable {
                collection: project.id.clone(),
                category: VariableCategory::Environment,
                name: name.into(),
                value,
                enabled: true,
                environment: None,
            })
            .await
            .expect("a variable");
    }
    // #61: a named environment, and a variable in it.
    let staging = storage
        .variables()
        .create_environment(project.id.clone(), "staging".into())
        .await
        .expect("an environment");
    storage
        .variables()
        .create(NewVariable {
            collection: project.id.clone(),
            category: VariableCategory::Custom,
            name: "user".into(),
            value: "stage-user".into(),
            enabled: true,
            environment: Some(staging.id.clone()),
        })
        .await
        .expect("a variable in it");
    let issue_with = |who, name: &str, can_send, can_write| {
        let directory = store.directory(who);
        let input = NewMcpToken {
            name: name.into(),
            can_send,
            can_write,
            days: None,
        };
        async move { directory.issue_mcp_token(input).await.expect("a token") }
    };
    let issue = |who, name: &str, can_send| issue_with(who, name, can_send, false);
    let reader = issue(ann.clone(), "reader", false).await;
    let sender = issue(ann.clone(), "sender", true).await;
    // H1.
    let writer = issue_with(ann.clone(), "writer", false, true).await;
    assert!(writer.token.can_write && !sender.token.can_write);

    // Only a token signs in here: not the page's cookie, nor nothing.
    let ping = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"});
    assert_eq!(mcp(&router, None, None, ping.clone()).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(
        mcp(&router, None, Some(&cookie), ping.clone()).await.0,
        StatusCode::UNAUTHORIZED
    );
    let wrong = format!("Bearer {}0", reader.secret);
    assert_eq!(
        mcp(&router, Some(&wrong), None, ping.clone()).await.0,
        StatusCode::UNAUTHORIZED
    );
    // And a token is no session anywhere else.
    let (status, _, _) = send_bearer(&router, STORE_PATH, &reader.secret).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // The handshake, a notification, the tools.
    let bearer = format!("Bearer {}", reader.secret);
    let initialize =
        json!({"jsonrpc": "2.0", "id": "a", "method": "initialize", "params": {"protocolVersion": "2025-03-26"}});
    let (_, body) = mcp(&router, Some(&bearer), None, initialize).await;
    assert_eq!(body["result"]["protocolVersion"], json!("2025-03-26"));
    assert_eq!(body["result"]["serverInfo"]["name"], json!("cyberapi-bridge"));
    let notified = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
    assert_eq!(
        mcp(&router, Some(&bearer), None, notified).await.0,
        StatusCode::ACCEPTED
    );
    let names = |body: &Value| -> Vec<String> {
        body["result"]["tools"]
            .as_array()
            .map(|tools| {
                tools
                    .iter()
                    .filter_map(|tool| tool["name"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    let list = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"});
    let (_, body) = mcp(&router, Some(&bearer), None, list.clone()).await;
    assert_eq!(
        names(&body),
        ["list_projects", "list_apis", "get_api", "get_variables", "get_history"]
    );
    let (_, body) = mcp(&router, Some(&format!("Bearer {}", sender.secret)), None, list.clone()).await;
    assert!(names(&body).contains(&"send_api".to_string()));
    assert!(!names(&body).contains(&"create_api".to_string()));
    let (_, body) = mcp(&router, Some(&format!("Bearer {}", writer.secret)), None, list).await;
    assert!(names(&body).contains(&"create_api".to_string()) && names(&body).contains(&"update_api".to_string()));
    let listed = store.directory(ann.clone()).mcp_tokens().await.expect("listed");
    assert!(
        listed.iter().any(|token| token.name == "writer" && token.can_write),
        "kept as allowed"
    );

    // What it reads, with the secrets masked and the templates kept.
    let (projects, _) = tool(&router, &reader.secret, "list_projects", json!({})).await;
    assert_eq!(projects[0]["name"], json!("shop"));
    assert_eq!(projects[0]["apis"], json!(1));
    let (apis, _) = tool(&router, &reader.secret, "list_apis", json!({"project": "shop"})).await;
    assert_eq!(apis[0]["path"], json!("ping"));
    let (got, _) = tool(
        &router,
        &reader.secret,
        "get_api",
        json!({"project": "shop", "api": "ping"}),
    )
    .await;
    assert_eq!(got["headers"][0]["value"], json!("***"));
    assert_eq!(got["headers"][1]["value"], json!("{{TOKEN}}"));
    assert_eq!(
        got["assertions"],
        json!([{"enabled": true, "subject": "status", "target": "", "op": "equals", "expected": "200"}])
    );
    assert_eq!(
        got["auth"],
        json!({"type": "basic", "username": "ann", "password": "***"})
    );
    assert!(
        got["body"]
            .as_str()
            .is_some_and(|body| body.contains("***") && !body.contains("hunter2"))
    );
    let (variables, _) = tool(&router, &reader.secret, "get_variables", json!({"project": "shop"})).await;
    assert_eq!(variables["environments"][0]["value"], json!(format!("http://{target}")));
    assert_eq!(variables["environments"][1]["value"], json!("***"));
    assert_eq!(variables["namedEnvironments"], json!(["staging"]));
    assert_eq!(variables["custom"][0]["environment"], json!("staging"));
    assert_eq!(variables["environments"][0]["environment"], Json::Null);
    assert_eq!(variables["selected"], Json::Null);
    let (missing, failed) = tool(
        &router,
        &reader.secret,
        "get_api",
        json!({"project": "shop", "api": "nope"}),
    )
    .await;
    assert!(failed && missing.as_str().is_some_and(|why| why.contains("list_apis")));

    // Sending: not with a reading token; with a sending one, from the bridge,
    // into the history.
    let (refused, failed) = tool(
        &router,
        &reader.secret,
        "send_api",
        json!({"project": "shop", "api": "ping"}),
    )
    .await;
    assert!(failed && refused.as_str().is_some_and(|why| why.contains("may not send")));
    let (answer, failed) = tool(
        &router,
        &sender.secret,
        "send_api",
        json!({"project": "shop", "api": "ping"}),
    )
    .await;
    assert!(!failed, "{answer}");
    assert_eq!(
        (answer["status"].clone(), answer["body"].clone()),
        (json!(200), json!("pong"))
    );
    // H1: each enabled assertion's result, on this answer only.
    assert_eq!(
        answer["assertions"],
        json!([{"index": 0, "subject": "status", "op": "equals", "expected": "200", "passed": true}])
    );
    let (history, _) = tool(
        &router,
        &reader.secret,
        "get_history",
        json!({"project": "shop", "api": "ping"}),
    )
    .await;
    assert_eq!(history.as_array().map(Vec::len), Some(1));
    assert!(history[0].get("assertions").is_none(), "results are not kept");
    // In a named environment, or one the project does not have.
    let (staged, failed) = tool(
        &router,
        &sender.secret,
        "send_api",
        json!({"project": "shop", "api": "ping", "environment": "Staging"}),
    )
    .await;
    assert!(!failed, "{staged}");
    let (nowhere, failed) = tool(
        &router,
        &sender.secret,
        "send_api",
        json!({"project": "shop", "api": "ping", "environment": "prod"}),
    )
    .await;
    assert!(
        failed && nowhere.as_str().is_some_and(|why| why.contains("staging")),
        "{nowhere}"
    );
    // D6: in production, what may change something is refused; a GET goes.
    let drop = storage
        .tree()
        .create_api(project.id.clone(), None, "drop".into())
        .await
        .expect("an API")
        .item;
    storage
        .tree()
        .save_request(
            project.id.clone(),
            drop.id.clone(),
            HttpRequest {
                method: HttpMethod::Delete,
                uri: "{{HOST}}/ping".into(),
                ..Default::default()
            },
        )
        .await
        .expect("its request");
    storage
        .tree()
        .set_project_settings(
            project.id.clone(),
            ProjectSettings {
                protected_environments: vec![staging.id.clone()],
                ..Default::default()
            },
        )
        .await
        .expect("staging is production");
    let (refused, failed) = tool(
        &router,
        &sender.secret,
        "send_api",
        json!({"project": "shop", "api": "drop", "environment": "staging"}),
    )
    .await;
    assert!(
        failed && refused.as_str().is_some_and(|why| why.contains("production")),
        "{refused}"
    );
    let (read, failed) = tool(
        &router,
        &sender.secret,
        "send_api",
        json!({"project": "shop", "api": "ping", "environment": "staging"}),
    )
    .await;
    assert!(!failed, "a GET goes: {read}");

    // H1: making and changing, with a token allowed to; never with another.
    let (refused, failed) = tool(
        &router,
        &sender.secret,
        "create_api",
        json!({"project": "shop", "name": "orders"}),
    )
    .await;
    assert!(failed && refused.as_str().is_some_and(|why| why.contains("may not change")));
    let folder = storage
        .tree()
        .create_folder(project.id.clone(), None, "users".into())
        .await
        .expect("a folder")
        .item;
    let (made, failed) = tool(
        &router,
        &writer.secret,
        "create_api",
        json!({"project": "shop", "name": "list", "folder": "users", "method": "GET",
               "url": "{{HOST}}/users?page=1", "headers": {"Accept": "application/json"},
               "description": "Lists the users."}),
    )
    .await;
    assert!(!failed, "{made}");
    assert_eq!(made["path"], json!("users/list"));
    assert_eq!(made["url"], json!("{{HOST}}/users"));
    assert_eq!(made["query"][0]["name"], json!("page"));
    assert_eq!(made["description"], json!("Lists the users."));
    let data = storage.tree().load(project.id.clone()).await.expect("the tree");
    let kept = data.apis.iter().find(|api| api.name == "list").expect("made");
    assert!(
        data.folders
            .iter()
            .any(|f| f.id == folder.id && f.children.contains(&kept.id))
    );
    let (changed, failed) = tool(
        &router,
        &writer.secret,
        "update_api",
        json!({"project": "shop", "api": "users/list", "name": "list users", "method": "POST",
               "contentType": "json", "body": {"page": 2}}),
    )
    .await;
    assert!(!failed, "{changed}");
    assert_eq!(changed["name"], json!("list users"));
    assert_eq!(changed["method"], json!("POST"));
    assert_eq!(changed["url"], json!("{{HOST}}/users"), "not given, kept");
    assert_eq!(changed["description"], json!("Lists the users."));
    let (bad, failed) = tool(
        &router,
        &writer.secret,
        "update_api",
        json!({"project": "shop", "api": "users/list users", "method": "FETCH"}),
    )
    .await;
    assert!(
        failed && bad.as_str().is_some_and(|why| why.contains("no method")),
        "{bad}"
    );

    // Where its account only views, a sending token does not send.
    store
        .set_grant(
            &project.id,
            &Grantee::Account(bob.clone()),
            Some(CollectionRole::Viewer),
        )
        .await
        .expect("bob views it");
    let bobs = issue(bob.clone(), "bobs", true).await;
    let (refused, failed) = tool(
        &router,
        &bobs.secret,
        "send_api",
        json!({"project": "shop", "api": "ping"}),
    )
    .await;
    assert!(failed && refused.as_str().is_some_and(|why| why.contains("editor")));
    let (seen, _) = tool(
        &router,
        &bobs.secret,
        "get_api",
        json!({"project": "shop", "api": "ping"}),
    )
    .await;
    assert_eq!(seen["method"], json!("GET"), "it reads what it views");
    // H1: nor does a writing token write there.
    let bobs_writer = issue_with(bob.clone(), "bobs-writer", false, true).await;
    let (refused, failed) = tool(
        &router,
        &bobs_writer.secret,
        "update_api",
        json!({"project": "shop", "api": "ping", "url": "http://elsewhere.test"}),
    )
    .await;
    assert!(failed, "a viewer's token changes nothing: {refused}");

    // Revoked, it signs in no more.
    store
        .directory(ann)
        .revoke_mcp_token(reader.token.id.clone())
        .await
        .expect("revoke");
    assert_eq!(
        mcp(&router, Some(&bearer), None, ping).await.0,
        StatusCode::UNAUTHORIZED
    );
}

/// A POST of an empty store call with a bearer token.
async fn send_bearer(router: &Router, path: &str, token: &str) -> (StatusCode, Option<String>, String) {
    let request = Request::builder()
        .method("POST")
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({"call": "collectionsList"}).to_string()))
        .expect("a request");
    let response = router.clone().oneshot(request).await.expect("an answer");
    let status = response.status();
    let bytes = response.into_body().collect().await.expect("a body").to_bytes();
    (status, None, String::from_utf8_lossy(&bytes).into_owned())
}
