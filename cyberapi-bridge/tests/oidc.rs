//! OpenID Connect sign-in (G5) against a provider on this machine.
//! Skips without `CYBERAPI_TEST_POSTGRES`, as the other bridge tests do.

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode, header};
use axum::routing::{get, post};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use cyberapi_bridge::Bridge;
use cyberapi_http::{NativeEngine, TargetPolicy};
use cyberapi_store_sql::{NewAccount, SqlStore};
use http_body_util::BodyExt;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use tower::ServiceExt;

const PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQDRplhAo1gLGYrm\n\
bT6l6jK69uP6NaP5LARK5jh06+XXsOhAfFv5U4wx6MY1zoczzPlQDsY3zfYTqoMa\n\
uAYOpGI0PUWu3tohkc+zp+reYKZFzKJ/EsUVwGjaCjLOMTR6MRWuO4mdKGCJlbTm\n\
gMG5YkZUIwwGaiHxXoiuxC2AoV8UGUmkqRR6A9gScr7ULOj2aMLVK/E5jCZUtC7A\n\
pD17nhWyGIu9hgNBBzeBhqYK4QBCfQE7Tb6ikdITdq2ZgCJ2a6BAEDqeP29LCL7t\n\
8qXT1uvBVRAq93ULaWOmZWigTAY+X9bVs9Z786BTqhTVokYi2L73Z6TElRV8RmBJ\n\
GFD3S5uPAgMBAAECggEAAxtqu7TLdojvqVkrIqqlL9R1Le+1S2uoAupy2JwxQ2vN\n\
EM+ZTe3yV9JyDrk7k3NjhfQUYx57md4oEdYEpq15Sx7oNVVoeMV3KdvAhTd8DRUP\n\
LS4X5Dw1EZvoOkgVQ3Lp7Opp1P25M/nyvNTz98Rk89379hGE5LEGaCHQd/f+kuJe\n\
uzaJv66iV5KZNj/eCHCCEWLm9kB8s/9DzbcjA5OOT0rNvy33kBV47mB7WGygoQcm\n\
r3iTzw9uSa32ntAajqg3wk2d24pVKXFw7nZwuWo7mFgr0+Lna/tzzDk4vtNUKhQ3\n\
pVgLopmsDvMOa7qcLECYDpiR6/4m/zfMbAmXEl8kcQKBgQD/UjaYwYHzMEr5V6a8\n\
PLxpDhYKgoLz90idLlRkSVzjpJUEsHmAowDyRHPsi1dQWjD1Fdis4LSp8Gj0EL4s\n\
ceACUw97AP/xrqokw6sBUdQCJzLWvf2Ttj5Mjf7d5t+m0Pyxrws4oSKDPOkjGsud\n\
rnPzumEvZ5lAi+EeHRQaLYFzaQKBgQDSNQt50VUeZlTl+6fzL8KB3iyqIQXIPseH\n\
+a8bA2pN9Ku68M899pL5aIdB3Oxl74JOGa2WY9Hh2RzuwjaC3SUN9J9pbHm0arKL\n\
R7OUu14avLkD5/uH8BEOEZ4zW5x1MiwOfKAmeff8WwZ3YLpLObeV8pJG067MGyXC\n\
FViterNQNwKBgQCMLAqlWreJV94Rfjmf5ohZ9M3M/1H9tqk7AhB67nv8zoKxjGFo\n\
ahU+7K3oE6fgCWQPaesHHmaRVdeJBxghNnORUyCQmz/o48etX28mzlHizLpwguYQ\n\
jmEtjld6ZCItZ1cUBdHXST+1ZV9pUnCvJPOrv4tTh0Dv2tvAuJ4ssQb84QKBgDfs\n\
3OzAAMFhwFyfo4NTxY//qCprNrqb2UIX77pkIjgzTC0kPD7kBx5L7RBc0pgcLLLS\n\
qhhE3mYT+VqcWRxf8afQ2UiXD5oseOwNW33Ki7LZWu/rHESCXkc78LcBPJMBki8Y\n\
gRM7h/qfzQyds3KoHGvMaZZmYy7yWg1U2I7vAh3pAoGAIjd/BMM49YgebSDIzV0H\n\
F+Oyt4dXnKezjuYAluqg6Bs7fcTH7kJuBAULkXJ34ufk23OCSHPq6TiAWJ0KFMnK\n\
bTR6ECL51BkeG31zrLUta3d18wFoUswt6DKeZ8Mmfe35wGdVjc2qYdkshD03vVrN\n\
6e5ffUYneSq6ZoZLQUaEsZw=\n\
-----END PRIVATE KEY-----\n";

const N: &str = "0aZYQKNYCxmK5m0-peoyuvbj-jWj-SwESuY4dOvl17DoQHxb-VOMMejGNc6HM8z5UA7GN832E6qDGrgGDqRiND1Frt7aIZHPs6fq3mCmRcyifxLFFcBo2goyzjE0ejEVrjuJnShgiZW05oDBuWJGVCMMBmoh8V6IrsQtgKFfFBlJpKkUegPYEnK-1Czo9mjC1SvxOYwmVLQuwKQ9e54VshiLvYYDQQc3gYamCuEAQn0BO02-opHSE3atmYAidmugQBA6nj9vSwi-7fKl09brwVUQKvd1C2ljpmVooEwGPl_W1bPWe_OgU6oU1aJGIti-92ekxJUVfEZgSRhQ90ubjw";

struct Idp {
    issuer: String,
    token: Mutex<String>,
    seen: Mutex<String>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

async fn discovery(State(idp): State<Arc<Idp>>) -> axum::Json<Value> {
    let base = idp.issuer.clone();
    axum::Json(json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/authorize"),
        "token_endpoint": format!("{base}/token"),
        "jwks_uri": format!("{base}/jwks"),
    }))
}

async fn jwks() -> axum::Json<Value> {
    axum::Json(json!({
        "keys": [{
            "kty": "RSA",
            "kid": "test",
            "alg": "RS256",
            "use": "sig",
            "e": "AQAB",
            "n": N,
        }]
    }))
}

async fn token(State(idp): State<Arc<Idp>>, body: String) -> axum::Json<Value> {
    *lock(&idp.seen) = body;
    axum::Json(json!({ "id_token": lock(&idp.token).clone() }))
}

struct Answer {
    status: StatusCode,
    location: Option<String>,
    cookies: Vec<String>,
    body: String,
}

async fn send(router: &Router, method: &str, path: &str, cookie: Option<&str>, body: Option<Value>) -> Answer {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, "bridge.test");
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
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let cookies = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok().map(str::to_string))
        .collect();
    let bytes = response.into_body().collect().await.expect("a body").to_bytes();
    Answer {
        status,
        location,
        cookies,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

fn query(url: &str) -> HashMap<String, String> {
    url.split_once('?')
        .map(|(_, query)| query)
        .unwrap_or("")
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn form_value(body: &str, key: &str) -> String {
    body.split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(name, _)| *name == key)
        .map(|(_, value)| value.to_string())
        .unwrap_or_default()
}

fn challenge_of(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn id_token(issuer: &str, subject: &str, email: &str, verified: bool, nonce: &str) -> String {
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("test".into());
    let claims = json!({
        "iss": issuer,
        "sub": subject,
        "aud": "client",
        "exp": 4_000_000_000_u64,
        "nonce": nonce,
        "email": email,
        "email_verified": verified,
    });
    let key = EncodingKey::from_rsa_pem(PEM.as_bytes()).expect("a key");
    encode(&header, &claims, &key).expect("a token")
}

async fn bridge(allowed: Vec<ipnet::IpNet>) -> Option<Bridge> {
    let Ok(url) = std::env::var("CYBERAPI_TEST_POSTGRES") else {
        eprintln!("skipped: CYBERAPI_TEST_POSTGRES is not set");
        return None;
    };
    sqlx::any::install_default_drivers();
    let name = format!("cyberapi_oidc_{}", ulid::Ulid::generate().to_string().to_lowercase());
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
        std::sync::Arc::new(engine),
        "setup-secret".into(),
        false,
        String::new(),
        cyberapi_bridge::MAX_SENDS,
    ))
}

async fn account(store: &SqlStore, name: &str) {
    store
        .create_account(NewAccount {
            name: name.to_string(),
            display_name: name.to_string(),
            password_hash: "unused".into(),
            admin: false,
        })
        .await
        .expect("an account");
}

struct Begun {
    state: String,
    cookie: String,
    challenge: String,
    nonce: String,
}

async fn begin(router: &Router, bearer: bool) -> Begun {
    let answer = send(
        router,
        "POST",
        "/v1/oidc/start",
        None,
        Some(json!({ "bearer": bearer })),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    let body: Value = serde_json::from_str(&answer.body).expect("json");
    let state = body["state"].as_str().expect("state").to_string();
    let url = body["url"].as_str().expect("url");
    let params = query(url);
    let cookie = if bearer {
        String::new()
    } else {
        let set = answer
            .cookies
            .iter()
            .find(|cookie| cookie.starts_with("cyberapi_oidc="))
            .expect("the state cookie");
        assert!(set.contains("SameSite=Lax"));
        set.split(';').next().expect("a pair").to_string()
    };
    Begun {
        state,
        cookie,
        challenge: params["code_challenge"].clone(),
        nonce: params["nonce"].clone(),
    }
}

#[tokio::test]
async fn single_sign_on_links_an_invited_account() {
    let Some(plain) = bridge(Vec::new()).await else { return };
    let plain = plain.router(None);
    let off = send(&plain, "GET", "/v1/oidc", None, None).await;
    assert_eq!(
        (off.status, off.body.as_str()),
        (StatusCode::OK, r#"{"enabled":false}"#)
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let issuer = format!("http://{}", listener.local_addr().expect("an address"));
    let idp = Arc::new(Idp {
        issuer: issuer.clone(),
        token: Mutex::new(String::new()),
        seen: Mutex::new(String::new()),
    });
    let app = Router::new()
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/jwks", get(jwks))
        .route("/token", post(token))
        .with_state(idp.clone());
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("the provider");
    });

    let allowed = vec!["127.0.0.1/32".parse().expect("a range")];
    let Some(bridge) = bridge(allowed).await else { return };
    let bridge = bridge
        .with_single_sign_on(Some(issuer.clone()), Some("client".into()), Some("secret".into()), None)
        .expect("settings");
    let email = format!("ada-{}@example.com", ulid::Ulid::generate());
    account(bridge.store(), &email).await;
    let quiet = format!("bea-{}@example.com", ulid::Ulid::generate());
    account(bridge.store(), &quiet).await;
    let router = bridge.router(None);

    let on = send(&router, "GET", "/v1/oidc", None, None).await;
    assert_eq!((on.status, on.body.as_str()), (StatusCode::OK, r#"{"enabled":true}"#));

    let first = begin(&router, false).await;
    let missed = send(
        &router,
        "GET",
        &format!("/v1/oidc/callback?code=abc&state={}", first.state),
        None,
        None,
    )
    .await;
    assert_eq!(missed.status, StatusCode::SEE_OTHER);
    assert_eq!(missed.location.as_deref(), Some("/?oidc=failed"));

    *lock(&idp.token) = id_token(&issuer, "subject-ada", &email, true, &first.nonce);
    let linked = send(
        &router,
        "GET",
        &format!("/v1/oidc/callback?code=abc&state={}", first.state),
        Some(&first.cookie),
        None,
    )
    .await;
    assert_eq!(linked.status, StatusCode::SEE_OTHER);
    assert_eq!(linked.location.as_deref(), Some("/"));
    assert!(
        linked
            .cookies
            .iter()
            .any(|cookie| cookie.starts_with("cyberapi_session=")),
        "{:?}",
        linked.cookies
    );
    let verifier = form_value(&lock(&idp.seen), "code_verifier");
    assert_eq!(challenge_of(&verifier), first.challenge);

    let again = begin(&router, false).await;
    *lock(&idp.token) = id_token(&issuer, "subject-ada", &email, false, &again.nonce);
    let signed_in = send(
        &router,
        "GET",
        &format!("/v1/oidc/callback?code=abc&state={}", again.state),
        Some(&again.cookie),
        None,
    )
    .await;
    assert_eq!(signed_in.location.as_deref(), Some("/"));

    let none = begin(&router, false).await;
    *lock(&idp.token) = id_token(&issuer, "subject-nobody", "nobody@example.com", true, &none.nonce);
    let no_account = send(
        &router,
        "GET",
        &format!("/v1/oidc/callback?code=abc&state={}", none.state),
        Some(&none.cookie),
        None,
    )
    .await;
    assert_eq!(no_account.location.as_deref(), Some("/?oidc=no-account"));

    let unverified = begin(&router, false).await;
    *lock(&idp.token) = id_token(&issuer, "subject-bea", &quiet, false, &unverified.nonce);
    let unverified = send(
        &router,
        "GET",
        &format!("/v1/oidc/callback?code=abc&state={}", unverified.state),
        Some(&unverified.cookie),
        None,
    )
    .await;
    assert_eq!(unverified.location.as_deref(), Some("/?oidc=unverified"));

    let taken = begin(&router, false).await;
    *lock(&idp.token) = id_token(&issuer, "subject-other", &email, true, &taken.nonce);
    let taken = send(
        &router,
        "GET",
        &format!("/v1/oidc/callback?code=abc&state={}", taken.state),
        Some(&taken.cookie),
        None,
    )
    .await;
    assert_eq!(taken.location.as_deref(), Some("/?oidc=taken"));

    let desktop = begin(&router, true).await;
    assert!(desktop.cookie.is_empty());
    let waiting = send(
        &router,
        "GET",
        &format!("/v1/oidc/poll?state={}", desktop.state),
        None,
        None,
    )
    .await;
    assert_eq!(waiting.status, StatusCode::ACCEPTED);
    *lock(&idp.token) = id_token(&issuer, "subject-ada", &email, true, &desktop.nonce);
    let page = send(
        &router,
        "GET",
        &format!("/v1/oidc/callback?code=desk&state={}", desktop.state),
        None,
        None,
    )
    .await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("Signed in") && page.body.contains("已登录"));
    assert!(!page.body.contains("subject-ada"));
    let polled = send(
        &router,
        "GET",
        &format!("/v1/oidc/poll?state={}", desktop.state),
        None,
        None,
    )
    .await;
    assert_eq!(polled.status, StatusCode::OK);
    let session: Value = serde_json::from_str(&polled.body).expect("a session");
    assert!(session["token"].as_str().is_some_and(|token| !token.is_empty()));
    assert!(!page.body.contains(session["token"].as_str().expect("a token")));
    let gone = send(
        &router,
        "GET",
        &format!("/v1/oidc/poll?state={}", desktop.state),
        None,
        None,
    )
    .await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
}
