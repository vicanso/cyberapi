//! C4: OAuth 2.0. The client credentials and the resource owner password
//! grants (RFC 6749 §4.4, §4.3) need no browser. The authorization code
//! grant with PKCE (RFC 6749 §4.1, RFC 7636, RFC 8252) does: the desktop
//! opens the system browser and a loopback listener, then exchanges the
//! code here ([`OAuthLogin::code_request`]). This module builds that
//! request and the authorize URL; it does not open a browser or listen.
//!
//! A request that signs in this way is sent with `Authorization: Bearer`
//! and a token the service asked the token endpoint for first
//! ([`OAuthLogin::token_request`], [`read_token`]), kept while the app runs
//! and asked again once it is out of date or the API refuses it; a token
//! that came with a refresh token is refreshed with it first (§6). An
//! authorization-code token is not asked for with the grant until the
//! desktop has exchanged a code: with no refresh token, nothing is sent.
//!
//! The client signs in to the token endpoint with Basic, its id and secret
//! form-encoded first (§2.3.1), or with both in the body. An `audience` and
//! a `resource` (RFC 8707), which some servers ask for (Auth0, Azure AD),
//! go in the form when given.
//!
//! The tokens are the reader's, kept as cookies are ([`KeptTokens`]): by a
//! hash of what they were asked with ([`OAuthLogin::key`]) — never the
//! secrets themselves — and never synced or backed up.

use crate::model::HttpMethod;
use crate::send::PreparedRequest;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::Digest as _;
use std::collections::BTreeMap;

/// How a token is had.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Grant {
    /// The client's own: its id and secret.
    #[default]
    ClientCredentials,
    /// A user's: their name and password, with the client's.
    Password,
    /// A user's, in their browser: the authorization code, with PKCE.
    /// The desktop opens the browser and listens on `127.0.0.1`; a page
    /// cannot. Data model 14.
    AuthorizationCode,
}

/// Where the client's id and secret go in the token request.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClientAuth {
    /// A Basic `Authorization` header (`client_secret_basic`).
    #[default]
    Header,
    /// The form's `client_id` and `client_secret` (`client_secret_post`).
    Body,
}

/// What an API, folder or project signs in with: templates, rendered as
/// the request's others are.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OAuth2 {
    pub grant: Grant,
    pub token_url: String,
    /// The authorization endpoint, for [`Grant::AuthorizationCode`]. A
    /// template, like the token URL.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub authorization_url: String,
    /// The loopback port the desktop listens on for the callback. `0` binds
    /// any free port (RFC 8252). Not a template.
    #[serde(default, skip_serializing_if = "port_is_zero")]
    pub redirect_port: u16,
    pub client_id: String,
    pub client_secret: String,
    /// The password grant's.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub username: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub password: String,
    /// Space-separated; none asked for when empty.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub scope: String,
    /// The API the token is for, where the server asks (Auth0's
    /// `audience`, RFC 8707's `resource`); none sent when empty.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub audience: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub resource: String,
    pub client_auth: ClientAuth,
}

fn port_is_zero(port: &u16) -> bool {
    *port == 0
}

impl OAuth2 {
    /// The texts that are templates: all of them. The user's name and
    /// password only for the grant that sends them.
    pub fn templates(&self) -> Vec<&str> {
        let mut texts = vec![
            self.token_url.as_str(),
            &self.client_id,
            &self.client_secret,
            &self.scope,
            &self.audience,
            &self.resource,
        ];
        if self.grant == Grant::Password {
            texts.extend([self.username.as_str(), &self.password]);
        }
        if self.grant == Grant::AuthorizationCode {
            texts.push(self.authorization_url.as_str());
        }
        texts
    }

    /// Rendered by `render`. The user's name and password only for the
    /// password grant; the authorization URL and the callback port only
    /// for the authorization code.
    pub fn login<E>(&self, render: impl Fn(&str) -> Result<String, E>) -> Result<OAuthLogin, E> {
        let password = self.grant == Grant::Password;
        let code = self.grant == Grant::AuthorizationCode;
        let only = |text: &str| if password { render(text) } else { Ok(String::new()) };
        Ok(OAuthLogin {
            grant: self.grant,
            token_url: render(self.token_url.trim())?,
            authorization_url: if code {
                render(self.authorization_url.trim())?
            } else {
                String::new()
            },
            redirect_port: if code { self.redirect_port } else { 0 },
            client_id: render(&self.client_id)?,
            client_secret: render(&self.client_secret)?,
            username: only(&self.username)?,
            password: only(&self.password)?,
            scope: render(self.scope.trim())?,
            audience: render(self.audience.trim())?,
            resource: render(self.resource.trim())?,
            client_auth: self.client_auth,
        })
    }
}

/// An [`OAuth2`] rendered: what a token is asked with, and what tells one
/// token from another.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct OAuthLogin {
    pub grant: Grant,
    pub token_url: String,
    /// Rendered. Empty unless the grant is [`Grant::AuthorizationCode`].
    pub authorization_url: String,
    /// The port the reader set. `0` asks the desktop to bind any free one.
    /// The port that was bound is not part of the key: the next authorize
    /// may use another.
    pub redirect_port: u16,
    pub client_id: String,
    pub client_secret: String,
    pub username: String,
    pub password: String,
    pub scope: String,
    pub audience: String,
    pub resource: String,
    pub client_auth: ClientAuth,
}

/// The `Authorization` value of a token.
pub fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

impl OAuthLogin {
    /// What its token is kept under: a SHA-256 of every field, so a changed
    /// secret or password asks anew, and none is written down.
    pub fn key(&self) -> String {
        let grant = match self.grant {
            Grant::ClientCredentials => "client_credentials",
            Grant::Password => "password",
            Grant::AuthorizationCode => "authorization_code",
        };
        let client_auth = match self.client_auth {
            ClientAuth::Header => "header",
            ClientAuth::Body => "body",
        };
        // The authorization URL and the port the reader set join the hash
        // only for that grant, so a client-credentials token kept before
        // they existed is still the same token.
        let port = self.redirect_port.to_string();
        let mut fields = vec![
            grant,
            &self.token_url,
            &self.client_id,
            &self.client_secret,
            &self.username,
            &self.password,
            &self.scope,
            &self.audience,
            &self.resource,
            client_auth,
        ];
        if self.grant == Grant::AuthorizationCode {
            fields.push(&self.authorization_url);
            fields.push(&port);
        }
        let mut hash = sha2::Sha256::new();
        for field in fields {
            // Each with its length, so no two lists hash alike.
            hash.update((field.len() as u64).to_be_bytes());
            hash.update(field.as_bytes());
        }
        hash.finalize().iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// The form asking for a token — with `refresh`, the refresh token
    /// asked with in place of the grant — and the client's Basic header
    /// when it signs in there. A client with no secret is a public one: its
    /// id goes in the form whichever way it was told to sign in.
    pub fn token_form(&self, refresh: Option<&str>) -> (String, Option<String>) {
        let mut form = url::form_urlencoded::Serializer::new(String::new());
        match (refresh, self.grant) {
            (Some(refresh), _) => {
                form.append_pair("grant_type", "refresh_token");
                form.append_pair("refresh_token", refresh);
            }
            (None, Grant::ClientCredentials) => {
                form.append_pair("grant_type", "client_credentials");
            }
            (None, Grant::Password) => {
                form.append_pair("grant_type", "password");
                form.append_pair("username", &self.username);
                form.append_pair("password", &self.password);
            }
            // Not a token request. The code is exchanged with [`Self::code_form`].
            (None, Grant::AuthorizationCode) => {
                form.append_pair("grant_type", "authorization_code");
            }
        }
        if !self.scope.is_empty() {
            form.append_pair("scope", &self.scope);
        }
        if !self.audience.is_empty() {
            form.append_pair("audience", &self.audience);
        }
        if !self.resource.is_empty() {
            form.append_pair("resource", &self.resource);
        }
        let authorization = self.client_into(&mut form);
        (form.finish(), authorization)
    }

    /// The form exchanging an authorization `code` (RFC 6749 §4.1.3) with
    /// PKCE's `verifier` (RFC 7636). The scope is not sent again. An
    /// audience and a resource go when set, as on [`Self::token_form`].
    pub fn code_form(&self, code: &str, redirect_uri: &str, verifier: &str) -> (String, Option<String>) {
        let mut form = url::form_urlencoded::Serializer::new(String::new());
        form.append_pair("grant_type", "authorization_code");
        form.append_pair("code", code);
        form.append_pair("redirect_uri", redirect_uri);
        form.append_pair("code_verifier", verifier);
        if !self.audience.is_empty() {
            form.append_pair("audience", &self.audience);
        }
        if !self.resource.is_empty() {
            form.append_pair("resource", &self.resource);
        }
        let authorization = self.client_into(&mut form);
        (form.finish(), authorization)
    }

    /// The client's sign-in, appended to `form` or returned as a Basic
    /// header. A client with no secret is a public one: its id goes in the
    /// form whichever way it was told to sign in.
    fn client_into(&self, form: &mut url::form_urlencoded::Serializer<String>) -> Option<String> {
        let basic = self.client_auth == ClientAuth::Header && !self.client_secret.is_empty();
        if !basic && !self.client_id.is_empty() {
            form.append_pair("client_id", &self.client_id);
            if !self.client_secret.is_empty() {
                form.append_pair("client_secret", &self.client_secret);
            }
        }
        let encoded = |text: &str| url::form_urlencoded::byte_serialize(text.as_bytes()).collect::<String>();
        basic.then(|| {
            let pair = format!("{}:{}", encoded(&self.client_id), encoded(&self.client_secret));
            format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(pair))
        })
    }

    /// The token request, sent as `api` would be — its timeouts,
    /// certificate check, HTTP version, proxy and User-Agent — to the token
    /// endpoint. Its certificate is the caller's to choose, by host.
    pub fn token_request(&self, api: &PreparedRequest, refresh: Option<&str>) -> PreparedRequest {
        let (body, authorization) = self.token_form(refresh);
        self.posted(api, body, authorization)
    }

    /// The token request that exchanges `code`, sent as `api` would be.
    pub fn code_request(
        &self,
        api: &PreparedRequest,
        code: &str,
        redirect_uri: &str,
        verifier: &str,
    ) -> PreparedRequest {
        let (body, authorization) = self.code_form(code, redirect_uri, verifier);
        self.posted(api, body, authorization)
    }

    fn posted(&self, api: &PreparedRequest, body: String, authorization: Option<String>) -> PreparedRequest {
        let mut headers = Vec::new();
        if let Some(agent) = api.header("user-agent") {
            headers.push(("User-Agent".to_string(), agent.to_string()));
        }
        headers.push(("Accept".into(), "application/json".into()));
        headers.push(("Content-Type".into(), "application/x-www-form-urlencoded".into()));
        if let Some(authorization) = authorization {
            headers.push(("Authorization".into(), authorization));
        }
        PreparedRequest {
            method: HttpMethod::Post,
            url: self.token_url.clone(),
            headers,
            body: body.into_bytes(),
            timeouts: api.timeouts,
            accept_invalid_certs: api.accept_invalid_certs,
            http_version: api.http_version,
            proxy: api.proxy.clone(),
            ..Default::default()
        }
    }
}

/// `http://127.0.0.1:{port}/callback` (RFC 8252: the loopback address, not
/// `localhost`).
pub fn redirect_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}/callback")
}

/// The port of a loopback callback URI, or `0` when it is not one.
pub fn redirect_port_of(uri: &str) -> u16 {
    let Ok(url) = url::Url::parse(uri.trim()) else {
        return 0;
    };
    let host = url.host_str().unwrap_or("");
    if !matches!(host, "127.0.0.1" | "localhost" | "[::1]") {
        return 0;
    }
    url.port().unwrap_or(0)
}

/// PKCE's `code_verifier`: base64url without padding of `entropy` (RFC 7636).
/// 32 bytes become the 43 characters the RFC's shorter end asks for.
pub fn code_verifier(entropy: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(entropy)
}

/// PKCE's S256 `code_challenge`: base64url without padding of the SHA-256
/// of the verifier's ASCII (RFC 7636 §4.2).
pub fn code_challenge(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()))
}

/// The browser's authorization URL: `authorization_url` with the code
/// grant's query added to whatever query it already has.
pub fn authorize_url(login: &OAuthLogin, redirect_uri: &str, state: &str, challenge: &str) -> Result<String, String> {
    let mut url = url::Url::parse(login.authorization_url.trim()).map_err(|error| {
        format!(
            "OAuth 2.0: the authorization URL {} is not one: {error}",
            login.authorization_url
        )
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!(
            "OAuth 2.0: the authorization URL {} is no http or https address",
            login.authorization_url
        ));
    }
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("response_type", "code");
        query.append_pair("client_id", &login.client_id);
        query.append_pair("redirect_uri", redirect_uri);
        query.append_pair("state", state);
        query.append_pair("code_challenge", challenge);
        query.append_pair("code_challenge_method", "S256");
        if !login.scope.is_empty() {
            query.append_pair("scope", &login.scope);
        }
        if !login.audience.is_empty() {
            query.append_pair("audience", &login.audience);
        }
        if !login.resource.is_empty() {
            query.append_pair("resource", &login.resource);
        }
    }
    Ok(url.to_string())
}

/// C4: the reader's tokens, by [`OAuthLogin::key`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct KeptTokens(pub BTreeMap<String, KeptToken>);

/// A token as kept.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeptToken {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// When it ends, in seconds since the epoch; none, until the API
    /// refuses it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
}

/// What the token endpoint gave.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Token {
    pub access_token: String,
    /// Seconds from when it was given; none, until the API refuses it.
    pub expires_in: Option<u64>,
    pub refresh_token: Option<String>,
}

#[derive(Deserialize)]
struct Answer {
    access_token: Option<String>,
    expires_in: Option<serde_json::Value>,
    refresh_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

/// The token in the endpoint's answer (§5.1): JSON, or the form some older
/// servers answer with. `Err` says why there is none: the endpoint's error
/// (§5.2) and its description, or the start of what it said.
pub fn read_token(status: u16, body: &[u8]) -> Result<Token, String> {
    let answer = serde_json::from_slice::<Answer>(body).ok().or_else(|| {
        let pairs: std::collections::HashMap<String, String> = url::form_urlencoded::parse(body).into_owned().collect();
        (pairs.contains_key("access_token") || pairs.contains_key("error")).then(|| Answer {
            access_token: pairs.get("access_token").cloned(),
            expires_in: pairs
                .get("expires_in")
                .map(|text| serde_json::Value::String(text.clone())),
            refresh_token: pairs.get("refresh_token").cloned(),
            error: pairs.get("error").cloned(),
            error_description: pairs.get("error_description").cloned(),
        })
    });
    let said = || {
        let text = String::from_utf8_lossy(body);
        let text = text.trim();
        match text.char_indices().nth(200) {
            Some((at, _)) => format!("{}…", &text[..at]),
            None => text.to_string(),
        }
    };
    let ok = (200..300).contains(&status);
    match answer {
        Some(Answer {
            error: Some(error),
            error_description,
            ..
        }) => Err(match error_description {
            Some(description) => format!("{status} {error}: {description}"),
            None => format!("{status} {error}"),
        }),
        Some(Answer {
            access_token: Some(access_token),
            expires_in,
            refresh_token,
            ..
        }) if ok && !access_token.is_empty() => Ok(Token {
            access_token,
            // A number, or a number in a string.
            expires_in: expires_in.and_then(|value| match value {
                serde_json::Value::Number(number) => number.as_u64(),
                serde_json::Value::String(text) => text.trim().parse().ok(),
                _ => None,
            }),
            refresh_token: refresh_token.filter(|token| !token.is_empty()),
        }),
        _ if ok => Err(format!("{status}, with no access_token: {}", said())),
        _ => Err(format!("{status}: {}", said())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prefs::Timeouts;
    use pretty_assertions::assert_eq;
    use std::convert::Infallible;

    fn login(grant: Grant, client_auth: ClientAuth) -> OAuthLogin {
        OAuth2 {
            grant,
            token_url: " https://id.test/token ".into(),
            client_id: "app id".into(),
            client_secret: "{{S}}".into(),
            username: "me".into(),
            password: "p&w".into(),
            scope: "read write".into(),
            client_auth,
            ..Default::default()
        }
        .login(|text| Ok::<_, Infallible>(text.replace("{{S}}", "s:cret")))
        .expect("rendered")
    }

    #[test]
    fn the_client_signs_in_with_basic_or_in_the_form() {
        let (form, authorization) = login(Grant::ClientCredentials, ClientAuth::Header).token_form(None);
        assert_eq!(form, "grant_type=client_credentials&scope=read+write");
        // Each form-encoded first (RFC 6749 §2.3.1).
        let basic = base64::engine::general_purpose::STANDARD.encode("app+id:s%3Acret");
        assert_eq!(authorization, Some(format!("Basic {basic}")));

        let (form, authorization) = login(Grant::ClientCredentials, ClientAuth::Body).token_form(None);
        assert_eq!(
            form,
            "grant_type=client_credentials&scope=read+write&client_id=app+id&client_secret=s%3Acret"
        );
        assert_eq!(authorization, None);
    }

    #[test]
    fn the_password_grant_sends_the_user_and_a_public_client_its_id() {
        let mut public = login(Grant::Password, ClientAuth::Header);
        public.client_secret.clear();
        let (form, authorization) = public.token_form(None);
        assert_eq!(
            form,
            "grant_type=password&username=me&password=p%26w&scope=read+write&client_id=app+id"
        );
        assert_eq!(authorization, None);
        // The user's only for the grant that sends them.
        assert_eq!(login(Grant::ClientCredentials, ClientAuth::Header).username, "");
    }

    #[test]
    fn an_audience_and_a_resource_go_when_given() {
        let mut login = login(Grant::ClientCredentials, ClientAuth::Header);
        login.audience = "https://api.test".into();
        login.resource = "urn:api".into();
        let (form, _) = login.token_form(None);
        assert_eq!(
            form,
            "grant_type=client_credentials&scope=read+write&audience=https%3A%2F%2Fapi.test&resource=urn%3Aapi"
        );
    }

    #[test]
    fn a_token_is_kept_under_a_hash_of_what_asked_for_it() {
        let login = login(Grant::Password, ClientAuth::Header);
        let key = login.key();
        assert_eq!(key.len(), 64);
        assert!(!key.contains("s:cret") && !key.contains("p&w"));
        assert_eq!(key, login.clone().key(), "the same, the same key");
        let mut other = login.clone();
        other.password = "changed".into();
        assert_ne!(other.key(), key, "a changed password asks anew");
        let mut moved = login;
        moved.username = "mep&w".into();
        moved.password = String::new();
        assert_ne!(moved.key(), key, "fields do not run into each other");
        let kept = KeptTokens(BTreeMap::from([(
            key.clone(),
            KeptToken {
                access_token: "t".into(),
                refresh_token: None,
                expires_at: Some(100),
            },
        )]));
        let json = serde_json::to_string(&kept).expect("JSON");
        assert_eq!(json, format!(r#"{{"{key}":{{"accessToken":"t","expiresAt":100}}}}"#));
        assert_eq!(serde_json::from_str::<KeptTokens>(&json).expect("read"), kept);
    }

    #[test]
    fn a_refresh_asks_with_the_refresh_token() {
        let (form, authorization) = login(Grant::Password, ClientAuth::Header).token_form(Some("r1"));
        assert_eq!(form, "grant_type=refresh_token&refresh_token=r1&scope=read+write");
        assert!(authorization.is_some());
    }

    #[test]
    fn the_token_request_goes_as_the_api_would() {
        let api = PreparedRequest {
            url: "https://api.test/a".into(),
            headers: vec![
                ("User-Agent".into(), "CyberAPI/1".into()),
                ("X-Other".into(), "x".into()),
            ],
            timeouts: Timeouts::default(),
            accept_invalid_certs: true,
            ..Default::default()
        };
        let asked = login(Grant::ClientCredentials, ClientAuth::Body).token_request(&api, None);
        assert_eq!(asked.method, HttpMethod::Post);
        assert_eq!(asked.url, "https://id.test/token");
        assert_eq!(
            asked.headers,
            vec![
                ("User-Agent".to_string(), "CyberAPI/1".to_string()),
                ("Accept".into(), "application/json".into()),
                ("Content-Type".into(), "application/x-www-form-urlencoded".into()),
            ]
        );
        assert!(asked.accept_invalid_certs);
        assert!(String::from_utf8_lossy(&asked.body).starts_with("grant_type=client_credentials"));
    }

    #[test]
    fn the_answer_is_read_as_json_or_a_form() {
        assert_eq!(
            read_token(
                200,
                br#"{"access_token":"t1","token_type":"bearer","expires_in":3600,"refresh_token":"r"}"#
            ),
            Ok(Token {
                access_token: "t1".into(),
                expires_in: Some(3600),
                refresh_token: Some("r".into()),
            })
        );
        assert_eq!(
            read_token(200, br#"{"access_token":"t1","expires_in":"60"}"#)
                .expect("token")
                .expires_in,
            Some(60)
        );
        assert_eq!(
            read_token(200, b"access_token=t2&token_type=bearer&scope=x"),
            Ok(Token {
                access_token: "t2".into(),
                ..Default::default()
            })
        );
    }

    #[test]
    fn why_there_is_no_token_is_said() {
        assert_eq!(
            read_token(
                401,
                br#"{"error":"invalid_client","error_description":"Client authentication failed"}"#
            ),
            Err("401 invalid_client: Client authentication failed".into())
        );
        assert_eq!(read_token(400, b"error=invalid_grant"), Err("400 invalid_grant".into()));
        assert_eq!(
            read_token(500, b" Internal Server Error "),
            Err("500: Internal Server Error".into())
        );
        assert_eq!(
            read_token(200, br#"{"token":"x"}"#),
            Err(r#"200, with no access_token: {"token":"x"}"#.into())
        );
        let long = "x".repeat(300);
        assert_eq!(
            read_token(502, long.as_bytes()),
            Err(format!("502: {}…", "x".repeat(200)))
        );
    }

    #[test]
    fn it_is_kept_by_its_fields() {
        let oauth = OAuth2 {
            token_url: "https://id.test/token".into(),
            client_id: "a".into(),
            ..Default::default()
        };
        let json = serde_json::to_string(&oauth).expect("JSON");
        assert_eq!(
            json,
            r#"{"grant":"clientCredentials","tokenUrl":"https://id.test/token","clientId":"a","clientSecret":"","clientAuth":"header"}"#
        );
        assert_eq!(serde_json::from_str::<OAuth2>(&json).expect("read"), oauth);
        let code = OAuth2 {
            grant: Grant::AuthorizationCode,
            authorization_url: "https://id.test/authorize".into(),
            redirect_port: 8080,
            token_url: "https://id.test/token".into(),
            client_id: "app".into(),
            ..Default::default()
        };
        let json = serde_json::to_string(&code).expect("JSON");
        assert!(json.contains(r#""grant":"authorizationCode""#), "{json}");
        assert!(
            json.contains(r#""authorizationUrl":"https://id.test/authorize""#),
            "{json}"
        );
        assert!(json.contains(r#""redirectPort":8080"#), "{json}");
        assert_eq!(serde_json::from_str::<OAuth2>(&json).expect("read"), code);
        assert_eq!(
            serde_json::from_str::<OAuth2>(r#"{"grant":"authorizationCode"}"#)
                .expect("an old shape, plus the new grant"),
            OAuth2 {
                grant: Grant::AuthorizationCode,
                ..Default::default()
            }
        );
    }

    /// RFC 7636 §4.2's worked example, and a verifier of 32 bytes.
    #[test]
    fn pkce_is_s256_of_the_verifier() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(code_challenge(verifier), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        let made = code_verifier(&[7u8; 32]);
        assert_eq!(made.len(), 43);
        assert!(
            made.chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
        );
    }

    #[test]
    fn the_code_is_exchanged_without_the_scope_and_the_browser_url_carries_pkce() {
        let mut login = login(Grant::AuthorizationCode, ClientAuth::Header);
        login.authorization_url = "https://id.test/authorize?audience=kept".into();
        login.redirect_port = 0;
        login.audience = "https://api.test".into();
        let (form, authorization) = login.code_form("the code", "http://127.0.0.1:9/callback", "verifier");
        assert_eq!(
            form,
            "grant_type=authorization_code&code=the+code&redirect_uri=http%3A%2F%2F127.0.0.1%3A9%2Fcallback&code_verifier=verifier&audience=https%3A%2F%2Fapi.test"
        );
        assert!(authorization.is_some());
        assert!(!form.contains("scope="), "the scope is not sent again");
        // Without a code this is not the client-credentials grant.
        let (plain, _) = login.token_form(None);
        assert!(plain.starts_with("grant_type=authorization_code"));
        assert!(!plain.contains("client_credentials"));
        let (refresh, _) = login.token_form(Some("r1"));
        assert!(refresh.starts_with("grant_type=refresh_token"));

        let url = authorize_url(&login, &redirect_uri(9), "st", "ch").expect("url");
        let url = url::Url::parse(&url).expect("parsed");
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(url.path(), "/authorize");
        assert_eq!(query.get("response_type").map(String::as_str), Some("code"));
        assert_eq!(query.get("client_id").map(String::as_str), Some("app id"));
        assert_eq!(
            query.get("redirect_uri").map(String::as_str),
            Some("http://127.0.0.1:9/callback")
        );
        assert_eq!(query.get("state").map(String::as_str), Some("st"));
        assert_eq!(query.get("code_challenge").map(String::as_str), Some("ch"));
        assert_eq!(query.get("code_challenge_method").map(String::as_str), Some("S256"));
        assert_eq!(query.get("scope").map(String::as_str), Some("read write"));
        // The login's audience is added. The one already on the URL stays.
        let raw = url.query().unwrap_or("");
        assert!(raw.contains("audience=kept"), "{raw}");
        assert!(raw.contains("audience=https%3A%2F%2Fapi.test"), "{raw}");
        assert_eq!(redirect_port_of("http://127.0.0.1:8080/callback"), 8080);
        assert_eq!(redirect_port_of("http://localhost:8080/callback"), 8080);
        assert_eq!(redirect_port_of("https://id.test/callback"), 0);
    }

    #[test]
    fn a_client_credentials_token_is_not_retold_by_the_authorization_url() {
        let login = login(Grant::ClientCredentials, ClientAuth::Header);
        let mut with_url = login.clone();
        with_url.authorization_url = "https://id.test/authorize".into();
        with_url.redirect_port = 8080;
        assert_eq!(with_url.key(), login.key());
        let mut code = login;
        code.grant = Grant::AuthorizationCode;
        code.authorization_url = "https://id.test/authorize".into();
        let moved = {
            let mut other = code.clone();
            other.authorization_url = "https://id.test/other".into();
            other
        };
        assert_ne!(moved.key(), code.key());
        code.redirect_port = 9;
        let other_port = {
            let mut other = code.clone();
            other.redirect_port = 10;
            other
        };
        assert_ne!(other_port.key(), code.key());
    }
}
