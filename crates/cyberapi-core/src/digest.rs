//! C2: Digest access authentication (RFC 7616, and RFC 2069's older form
//! without `qop`). A request that signs in with Digest goes out once
//! without it; a 401 whose `WWW-Authenticate` asks for Digest is answered
//! by sending it again with an `Authorization` worked from the server's
//! nonce ([`challenge`], [`authorization`]).
//!
//! MD5 and SHA-256, each also `-sess`, with `qop=auth`. `auth-int`, which
//! signs the body too, is not answered: few servers ask for it alone.

use md5::Digest as _;

/// What a request signs in with: its user and password, rendered.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DigestLogin {
    pub username: String,
    pub password: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algorithm {
    Md5,
    Md5Sess,
    Sha256,
    Sha256Sess,
}

impl Algorithm {
    fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_uppercase().as_str() {
            "MD5" => Some(Self::Md5),
            "MD5-SESS" => Some(Self::Md5Sess),
            "SHA-256" => Some(Self::Sha256),
            "SHA-256-SESS" => Some(Self::Sha256Sess),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Md5 => "MD5",
            Self::Md5Sess => "MD5-sess",
            Self::Sha256 => "SHA-256",
            Self::Sha256Sess => "SHA-256-sess",
        }
    }

    fn hash(self, text: &str) -> String {
        let bytes = match self {
            Self::Md5 | Self::Md5Sess => md5::Md5::digest(text.as_bytes()).to_vec(),
            Self::Sha256 | Self::Sha256Sess => sha2::Sha256::digest(text.as_bytes()).to_vec(),
        };
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn per_session(self) -> bool {
        matches!(self, Self::Md5Sess | Self::Sha256Sess)
    }

    fn strength(self) -> u8 {
        match self {
            Self::Md5 | Self::Md5Sess => 0,
            Self::Sha256 | Self::Sha256Sess => 1,
        }
    }
}

/// A server's Digest challenge, one this answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Challenge {
    pub realm: String,
    pub nonce: String,
    pub opaque: Option<String>,
    pub algorithm: Algorithm,
    /// Whether the challenge named its algorithm: the answer names it back.
    pub named_algorithm: bool,
    /// `qop=auth`; without it, RFC 2069's answer.
    pub qop: bool,
    /// The user's name goes hashed.
    pub userhash: bool,
}

/// Why a 401 is not answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unanswered {
    /// It asks for no Digest: the 401 is the answer.
    NotAsked,
    /// It asks for Digest in a way this does not answer: what it asks.
    Unsupported(String),
}

/// The strongest Digest challenge this answers among `headers`'
/// `WWW-Authenticate`s: SHA-256 before MD5.
pub fn challenge(headers: &[(String, String)]) -> Result<Challenge, Unanswered> {
    let mut unsupported = None;
    let mut best: Option<Challenge> = None;
    let asked = headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("www-authenticate"))
        .flat_map(|(_, value)| challenges(value))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("digest"));
    for (_, params) in asked {
        let param = |name: &str| {
            params
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.clone())
        };
        let named = param("algorithm");
        let Some(algorithm) = named.as_deref().map_or(Some(Algorithm::Md5), Algorithm::parse) else {
            unsupported = Some(format!("algorithm {}", named.unwrap_or_default()));
            continue;
        };
        let qops = param("qop");
        let qop = match &qops {
            None => false,
            Some(list) if list.split(',').any(|qop| qop.trim().eq_ignore_ascii_case("auth")) => true,
            Some(list) => {
                unsupported = Some(format!("qop {list}"));
                continue;
            }
        };
        let (Some(realm), Some(nonce)) = (param("realm"), param("nonce")) else {
            unsupported = Some("a challenge without a realm or a nonce".into());
            continue;
        };
        let found = Challenge {
            realm,
            nonce,
            opaque: param("opaque"),
            algorithm,
            named_algorithm: named.is_some(),
            qop,
            userhash: param("userhash").is_some_and(|on| on.eq_ignore_ascii_case("true")),
        };
        if best
            .as_ref()
            .is_none_or(|best| found.algorithm.strength() > best.algorithm.strength())
        {
            best = Some(found);
        }
    }
    match (best, unsupported) {
        (Some(best), _) => Ok(best),
        (None, Some(what)) => Err(Unanswered::Unsupported(what)),
        (None, None) => Err(Unanswered::NotAsked),
    }
}

/// The `Authorization` that answers `challenge` for a `method` to `uri`
/// (the request target: the path and the query), `cnonce` the client's
/// own random nonce. It is the first request with this nonce: `nc=1`.
pub fn authorization(challenge: &Challenge, method: &str, uri: &str, login: &DigestLogin, cnonce: &str) -> String {
    const NC: &str = "00000001";
    let hash = |text: &str| challenge.algorithm.hash(text);
    let realm = &challenge.realm;
    let nonce = &challenge.nonce;
    let mut ha1 = hash(&format!("{}:{realm}:{}", login.username, login.password));
    if challenge.algorithm.per_session() {
        ha1 = hash(&format!("{ha1}:{nonce}:{cnonce}"));
    }
    let ha2 = hash(&format!("{method}:{uri}"));
    let response = if challenge.qop {
        hash(&format!("{ha1}:{nonce}:{NC}:{cnonce}:auth:{ha2}"))
    } else {
        hash(&format!("{ha1}:{nonce}:{ha2}"))
    };
    let username = if challenge.userhash {
        hash(&format!("{}:{realm}", login.username))
    } else {
        login.username.clone()
    };
    let mut fields = vec![
        format!("username={}", quoted(&username)),
        format!("realm={}", quoted(realm)),
        format!("nonce={}", quoted(nonce)),
        format!("uri={}", quoted(uri)),
    ];
    if challenge.named_algorithm {
        fields.push(format!("algorithm={}", challenge.algorithm.name()));
    }
    fields.push(format!("response={}", quoted(&response)));
    if challenge.qop {
        fields.push("qop=auth".into());
        fields.push(format!("nc={NC}"));
        fields.push(format!("cnonce={}", quoted(cnonce)));
    }
    if let Some(opaque) = &challenge.opaque {
        fields.push(format!("opaque={}", quoted(opaque)));
    }
    if challenge.userhash {
        fields.push("userhash=true".into());
    }
    format!("Digest {}", fields.join(", "))
}

fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The challenges of one `WWW-Authenticate`: each scheme with its
/// parameters, quoted values unquoted. A token68 (Negotiate's, say) is
/// passed over.
fn challenges(value: &str) -> Vec<(String, Vec<(String, String)>)> {
    let chars: Vec<char> = value.chars().collect();
    let mut found: Vec<(String, Vec<(String, String)>)> = Vec::new();
    let mut at = 0;
    let skip = |at: &mut usize, what: &dyn Fn(char) -> bool| {
        while *at < chars.len() && what(chars[*at]) {
            *at += 1;
        }
    };
    loop {
        skip(&mut at, &|c| c.is_whitespace() || c == ',');
        if at >= chars.len() {
            return found;
        }
        let start = at;
        skip(&mut at, &|c| !c.is_whitespace() && c != ',' && c != '=');
        let token: String = chars[start..at].iter().collect();
        skip(&mut at, &|c| c.is_whitespace());
        if at >= chars.len() || chars[at] != '=' {
            found.push((token, Vec::new()));
            continue;
        }
        // `name=value`, or a token68's padding: `abc==`.
        at += 1;
        if at >= chars.len() || chars[at] == '=' || chars[at] == ',' {
            skip(&mut at, &|c| c == '=');
            continue;
        }
        skip(&mut at, &|c| c.is_whitespace());
        let value = if at < chars.len() && chars[at] == '"' {
            at += 1;
            let mut text = String::new();
            while at < chars.len() && chars[at] != '"' {
                if chars[at] == '\\' && at + 1 < chars.len() {
                    at += 1;
                }
                text.push(chars[at]);
                at += 1;
            }
            at += 1;
            text
        } else {
            let start = at;
            skip(&mut at, &|c| !c.is_whitespace() && c != ',');
            chars[start..at].iter().collect()
        };
        if let Some((_, params)) = found.last_mut() {
            params.push((token, value));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn header(value: &str) -> Vec<(String, String)> {
        vec![("WWW-Authenticate".into(), value.into())]
    }

    fn mufasa(password: &str) -> DigestLogin {
        DigestLogin {
            username: "Mufasa".into(),
            password: password.into(),
        }
    }

    /// RFC 7616 §3.9.1: SHA-256 is chosen over MD5, and both answer as
    /// the RFC does.
    #[test]
    fn answers_rfc_7616s_example() {
        let headers = vec![
            (
                "WWW-Authenticate".to_string(),
                r#"Digest realm="http-auth@example.org", qop="auth, auth-int", algorithm=MD5, nonce="7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v", opaque="FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS""#.to_string(),
            ),
            (
                "www-authenticate".to_string(),
                r#"Digest realm="http-auth@example.org", qop="auth, auth-int", algorithm=SHA-256, nonce="7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v", opaque="FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS""#.to_string(),
            ),
        ];
        let cnonce = "f2/wE4q74E6zIJEtWaHKaf5wv/H5QzzpXusqGemxURZJ";
        let sha = challenge(&headers).expect("answered");
        assert_eq!(sha.algorithm, Algorithm::Sha256);
        assert_eq!(
            authorization(&sha, "GET", "/dir/index.html", &mufasa("Circle of Life"), cnonce),
            r#"Digest username="Mufasa", realm="http-auth@example.org", nonce="7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v", uri="/dir/index.html", algorithm=SHA-256, response="753927fa0e85d155564e2e272a28d1802ca10daf4496794697cf8db5856cb6c1", qop=auth, nc=00000001, cnonce="f2/wE4q74E6zIJEtWaHKaf5wv/H5QzzpXusqGemxURZJ", opaque="FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS""#
        );
        let md5 = challenge(&headers[..1]).expect("answered");
        assert!(
            authorization(&md5, "GET", "/dir/index.html", &mufasa("Circle of Life"), cnonce)
                .contains(r#"response="8ca523f5e9506fed4657c9700eebdbec""#)
        );
    }

    /// RFC 2617 §3.5, without naming its algorithm.
    #[test]
    fn answers_rfc_2617s_example() {
        let found = challenge(&header(
            r#"Digest realm="testrealm@host.com", qop="auth,auth-int", nonce="dcd98b7102dd2f0e8b11d0f600bfb0c093", opaque="5ccc069c403ebaf9f0171e9517f40e41""#,
        ))
        .expect("answered");
        let answer = authorization(&found, "GET", "/dir/index.html", &mufasa("Circle Of Life"), "0a4f113b");
        assert!(
            answer.contains(r#"response="6629fae49393a05397450978507c4ef1""#),
            "{answer}"
        );
        assert!(!answer.contains("algorithm="), "not named, not named back");
    }

    #[test]
    fn without_qop_it_answers_as_rfc_2069() {
        let found = challenge(&header(r#"Digest realm="r", nonce="n""#)).expect("answered");
        assert!(!found.qop);
        let answer = authorization(&found, "GET", "/a?b=1", &mufasa("p"), "c");
        let ha1 = Algorithm::Md5.hash("Mufasa:r:p");
        let ha2 = Algorithm::Md5.hash("GET:/a?b=1");
        let response = Algorithm::Md5.hash(&format!("{ha1}:n:{ha2}"));
        assert_eq!(
            answer,
            format!(r#"Digest username="Mufasa", realm="r", nonce="n", uri="/a?b=1", response="{response}""#)
        );
    }

    #[test]
    fn a_session_and_a_hashed_user() {
        let found = challenge(&header(
            r#"Digest realm="r", nonce="n", qop="auth", algorithm=SHA-256-sess, userhash=true"#,
        ))
        .expect("answered");
        let answer = authorization(&found, "POST", "/", &mufasa("p"), "c");
        let hash = |text: &str| Algorithm::Sha256.hash(text);
        let ha1 = hash(&format!("{}:n:c", hash("Mufasa:r:p")));
        let response = hash(&format!("{ha1}:n:00000001:c:auth:{}", hash("POST:/")));
        assert!(answer.contains(&format!(r#"response="{response}""#)), "{answer}");
        assert!(answer.contains(&format!(r#"username="{}""#, hash("Mufasa:r"))));
        assert!(answer.contains("algorithm=SHA-256-sess") && answer.ends_with("userhash=true"));
    }

    #[test]
    fn what_is_not_answered_says_why() {
        assert_eq!(challenge(&header(r#"Basic realm="x""#)), Err(Unanswered::NotAsked));
        assert_eq!(challenge(&[]), Err(Unanswered::NotAsked));
        assert_eq!(
            challenge(&header(r#"Digest realm="r", nonce="n", qop="auth-int""#)),
            Err(Unanswered::Unsupported("qop auth-int".into()))
        );
        assert_eq!(
            challenge(&header(r#"Digest realm="r", nonce="n", algorithm=SHA-512-256"#)),
            Err(Unanswered::Unsupported("algorithm SHA-512-256".into()))
        );
    }

    #[test]
    fn challenges_share_a_header_with_others() {
        let found = challenge(&header(
            r#"Basic realm="b", Bearer error="invalid_token", Digest realm="d\"q", nonce="n", qop=auth"#,
        ))
        .expect("answered");
        assert_eq!(found.realm, "d\"q");
        assert!(found.qop);
        assert_eq!(
            challenges(r#"Negotiate abc==, Digest realm="r""#),
            [
                ("Negotiate".to_string(), Vec::new()),
                ("Digest".to_string(), vec![("realm".to_string(), "r".to_string())]),
            ]
        );
    }
}
