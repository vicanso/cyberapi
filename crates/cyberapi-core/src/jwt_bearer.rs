//! R6: JWT Bearer signing (`Auth::JwtBearer`), performed right before sending
//! so template expressions (`{{timestamp()}}`, secret variables, etc.) are
//! evaluated fresh on every request.

use crate::auth::KeyPlace;
use base64::Engine as _;
use jsonwebtoken::{Algorithm, EncodingKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Algorithms supported for signing JWT Bearer tokens.
#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum JwtAlgorithm {
    #[default]
    HS256,
    HS384,
    HS512,
    RS256,
    RS384,
    RS512,
    PS256,
    PS384,
    PS512,
    ES256,
    ES384,
    EdDSA,
}

impl JwtAlgorithm {
    pub const ALL: [Self; 12] = [
        Self::HS256,
        Self::HS384,
        Self::HS512,
        Self::RS256,
        Self::RS384,
        Self::RS512,
        Self::PS256,
        Self::PS384,
        Self::PS512,
        Self::ES256,
        Self::ES384,
        Self::EdDSA,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::HS256 => "HS256",
            Self::HS384 => "HS384",
            Self::HS512 => "HS512",
            Self::RS256 => "RS256",
            Self::RS384 => "RS384",
            Self::RS512 => "RS512",
            Self::PS256 => "PS256",
            Self::PS384 => "PS384",
            Self::PS512 => "PS512",
            Self::ES256 => "ES256",
            Self::ES384 => "ES384",
            Self::EdDSA => "EdDSA",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_uppercase().as_str() {
            "HS256" => Some(Self::HS256),
            "HS384" => Some(Self::HS384),
            "HS512" => Some(Self::HS512),
            "RS256" => Some(Self::RS256),
            "RS384" => Some(Self::RS384),
            "RS512" => Some(Self::RS512),
            "PS256" => Some(Self::PS256),
            "PS384" => Some(Self::PS384),
            "PS512" => Some(Self::PS512),
            "ES256" => Some(Self::ES256),
            "ES384" => Some(Self::ES384),
            "EDDSA" => Some(Self::EdDSA),
            _ => None,
        }
    }

    /// HMAC algorithms take a shared secret; asymmetric algorithms take a PEM private key.
    pub fn is_symmetric(self) -> bool {
        matches!(self, Self::HS256 | Self::HS384 | Self::HS512)
    }

    pub fn to_jwt(self) -> Algorithm {
        match self {
            Self::HS256 => Algorithm::HS256,
            Self::HS384 => Algorithm::HS384,
            Self::HS512 => Algorithm::HS512,
            Self::RS256 => Algorithm::RS256,
            Self::RS384 => Algorithm::RS384,
            Self::RS512 => Algorithm::RS512,
            Self::PS256 => Algorithm::PS256,
            Self::PS384 => Algorithm::PS384,
            Self::PS512 => Algorithm::PS512,
            Self::ES256 => Algorithm::ES256,
            Self::ES384 => Algorithm::ES384,
            Self::EdDSA => Algorithm::EdDSA,
        }
    }
}

/// Stored JWT Bearer authentication configuration (templates allowed in every text field).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct JwtBearer {
    pub algorithm: JwtAlgorithm,
    pub secret: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub secret_base64: bool,
    pub payload: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub header: String,
    pub place: KeyPlace,
    /// Header prefix when `place == KeyPlace::Header` (not skipped when empty so an
    /// explicit empty prefix round-trips cleanly).
    pub prefix: String,
    pub query_param: String,
}

impl Default for JwtBearer {
    fn default() -> Self {
        Self {
            algorithm: JwtAlgorithm::HS256,
            secret: String::new(),
            secret_base64: false,
            payload: "{}".into(),
            header: String::new(),
            place: KeyPlace::Header,
            prefix: "Bearer".into(),
            query_param: "token".into(),
        }
    }
}

impl JwtBearer {
    pub fn has_credentials(&self) -> bool {
        !self.secret.trim().is_empty()
    }

    pub fn templates(&self) -> Vec<&str> {
        vec![
            &self.secret,
            &self.payload,
            &self.header,
            &self.prefix,
            &self.query_param,
        ]
    }

    pub fn effective_query_param(&self) -> &str {
        let trimmed = self.query_param.trim();
        if trimmed.is_empty() { "token" } else { trimmed }
    }

    pub fn login<E>(&self, render: impl Fn(&str) -> Result<String, E>) -> Result<JwtBearerLogin, E> {
        Ok(JwtBearerLogin {
            algorithm: self.algorithm,
            secret: render(&self.secret)?,
            secret_base64: self.secret_base64,
            payload: render(&self.payload)?,
            header: render(&self.header)?,
            place: self.place,
            prefix: render(&self.prefix)?,
            query_param: render(&self.query_param)?,
        })
    }
}

/// Rendered JWT Bearer inputs ready for [`sign`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JwtBearerLogin {
    pub algorithm: JwtAlgorithm,
    pub secret: String,
    pub secret_base64: bool,
    pub payload: String,
    pub header: String,
    pub place: KeyPlace,
    pub prefix: String,
    pub query_param: String,
}

impl JwtBearerLogin {
    pub fn effective_query_param(&self) -> &str {
        let trimmed = self.query_param.trim();
        if trimmed.is_empty() { "token" } else { trimmed }
    }
}

/// Formats the `Authorization` header value for a signed JWT (`<prefix> <token>`,
/// or `<token>` alone when `prefix` is blank).
pub fn header_value(prefix: &str, token: &str) -> String {
    let prefix = prefix.trim();
    if prefix.is_empty() {
        token.to_string()
    } else {
        format!("{prefix} {token}")
    }
}

/// Signs a JWT from rendered `login` fields and returns the compact JWS string.
pub fn sign(login: &JwtBearerLogin) -> Result<String, String> {
    if login.secret.trim().is_empty() {
        return Err("JWT Bearer: missing secret or private key".into());
    }

    let mut header_map = parse_json_object(&login.header, "header")?;
    header_map.insert("alg".into(), Value::String(login.algorithm.as_str().into()));
    if !header_map.contains_key("typ") {
        header_map.insert("typ".into(), Value::String("JWT".into()));
    }

    let payload_map = parse_json_object(&login.payload, "payload")?;
    let encoding_key = build_encoding_key(login)?;

    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let header_bytes = serde_json::to_vec(&Value::Object(header_map)).map_err(|error| error.to_string())?;
    let payload_bytes = serde_json::to_vec(&Value::Object(payload_map)).map_err(|error| error.to_string())?;
    let signing_input = format!("{}.{}", b64.encode(header_bytes), b64.encode(payload_bytes));
    let signature = jsonwebtoken::crypto::sign(signing_input.as_bytes(), &encoding_key, login.algorithm.to_jwt())
        .map_err(|error| format!("JWT Bearer: {error}"))?;
    Ok(format!("{signing_input}.{signature}"))
}

fn parse_json_object(raw: &str, label: &str) -> Result<serde_json::Map<String, Value>, String> {
    let stripped = crate::send::strip_json_comments(raw);
    let trimmed = stripped.trim();
    if trimmed.is_empty() {
        return Ok(serde_json::Map::new());
    }
    let value: Value =
        serde_json::from_str(trimmed).map_err(|error| format!("JWT Bearer: invalid {label} JSON: {error}"))?;
    match value {
        Value::Object(map) => Ok(map),
        _ => Err(format!("JWT Bearer: {label} must be a JSON object")),
    }
}

fn build_encoding_key(login: &JwtBearerLogin) -> Result<EncodingKey, String> {
    match login.algorithm {
        JwtAlgorithm::HS256 | JwtAlgorithm::HS384 | JwtAlgorithm::HS512 => {
            if login.secret_base64 {
                let bytes = decode_base64(login.secret.trim())
                    .ok_or_else(|| "JWT Bearer: secret is not valid Base64".to_string())?;
                Ok(EncodingKey::from_secret(&bytes))
            } else {
                Ok(EncodingKey::from_secret(login.secret.as_bytes()))
            }
        }
        JwtAlgorithm::RS256
        | JwtAlgorithm::RS384
        | JwtAlgorithm::RS512
        | JwtAlgorithm::PS256
        | JwtAlgorithm::PS384
        | JwtAlgorithm::PS512 => EncodingKey::from_rsa_pem(login.secret.trim().as_bytes())
            .map_err(|error| format!("JWT Bearer: invalid RSA PEM key: {error}")),
        JwtAlgorithm::ES256 | JwtAlgorithm::ES384 => EncodingKey::from_ec_pem(login.secret.trim().as_bytes())
            .map_err(|error| format!("JWT Bearer: invalid EC PEM key: {error}")),
        JwtAlgorithm::EdDSA => EncodingKey::from_ed_pem(login.secret.trim().as_bytes())
            .map_err(|error| format!("JWT Bearer: invalid Ed25519 PEM key: {error}")),
    }
}

fn decode_base64(encoded: &str) -> Option<Vec<u8>> {
    use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
    STANDARD
        .decode(encoded)
        .or_else(|_| STANDARD_NO_PAD.decode(encoded))
        .or_else(|_| URL_SAFE.decode(encoded))
        .or_else(|_| URL_SAFE_NO_PAD.decode(encoded))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{DecodingKey, Validation, decode};
    use pretty_assertions::assert_eq;
    use serde_json::json;

    const RSA_PRIVATE_PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQDUTWZWiOCZM07b\n\
Nvjh06XRFyVCPdneXD4io+txpsyx585DI7sqC3IlQA36XjS3R9qNUaRmATlGqAnI\n\
kYq9AVy4+lVXeP+SG4w6NOVFOo/CWEp8640sjxCQ2ACK8TULyoGyW5x7pS2u6BXK\n\
0KIEC94xkOMVR85pLeVhP4H43k8pR8fjz0J3QFGpInEAaNq6ah4ndp54e+RxWma0\n\
Xe5Lrgp2H5C5SHTIbviKbOnqsBrvCHivQ58sdpbC4xBRC/Vg939Xtv+nXJDx5o0e\n\
9V7YrOjPQiRn3RPocE6o3MEEUhNu+kfurncAZHG+dsUKapq3SA1zYT1DSMJMC8id\n\
9wTudM/dAgMBAAECggEAAWr8Lx+L/R7ZNQJbezJ4S/Ldkvix3Kjte8zI2s3CtBSJ\n\
jYYo2AYdZzj1wY00zkDEsWhr2H2fF2ZbxBVbNeLyg5PryNTTXZ0mhxEdjoUA0nzI\n\
CtJW+ytwgD3YGGPVOcr3B+kYdZv3ougEioxhk5G6zdDSEANvR7JzlMn82KeXxadL\n\
0q8S94wILdrd0MT6I8rGeURsZ4WgTTg8+pAviD6ED7GXooMqql4I6T/gbaZUeu2N\n\
Y2f1tPkNpSzTGERn6y0hzUbnrk2FUdGrVJLDzUJsLGNyVabe/bpKPX4ayg4GjI2P\n\
5wh/mlmy5/7bAOy0ZYDDqEjQnvSODyyqpSfLcRg2YwKBgQD4dbdfyfcpPOxY9T/V\n\
lD2jD0kBhTQ+vhx5SDM5ckZ3dwcmMXPbBk5W436Ujf9NH1A12F2nAEYbHrJ7KbMn\n\
MgbTxAuoRVVq0ywMqJ9Sw133I4+DcEbu52uvkC0DkJk+qTnktpXelrASTKZc7LGN\n\
OXm9t43iZau/W8s8PWIB9IYg3wKBgQDavsaq2jnUuiOzvhEWaZW9P9emcpQiuciR\n\
nNRn+tbih5QiBF/sJTq1zRJXmLo6p76d+/D/eaJI1sTTnActzyu7IpQxJ8HXgcZn\n\
fkWMBK5yyvW8CM2u6nYihuNReuiM/AUNY/ESpMdsB+s9Vx8IObrML+D7TuRHjm9P\n\
6Vh2ZdD6wwKBgQDXZ6ctYrnnhKwbi7RmBHlTjwLqUIO1CblB5LiUS2CQXh9Sm3ko\n\
IFV2uxCY1mrKgCVYKVc4W+JrzLdMVCDAEH0ex06b+lBNpvr/lolahYpHowrfF05H\n\
/Va9BLAh8REK5EgO48UXD2li1BU/Zu2f1H6ZXjOiXicCZiDS/xyjOU233QKBgDg4\n\
iD8pT2vJW5u64hFsmbvGSNrS4Bf8v/wM3trKsagPSJdloHHFL83CI1C4REUHEAry\n\
0gIXdQpbiMMKex3e/VJflM79dOi6xg+beKyAF4YRfA8UBClMwYgmOw6aFGMH8xR5\n\
91/fUhIGBwQA/EzU6ZnJRH3SzQFHLnzPcJkdmhSZAoGANKzQ4BV49qRXyfPE95Ww\n\
+G0ciffJJAtTnTO2b4YkiDDwTwhzV4YtVN0nOGe7qzC63PmdOKO3EHUXBURtk+Vo\n\
Uv9Dx9fTpgUAAAhNiF8bI8wybclZcxV+Dxk0q77NxSk1AM+bTeTlhq70zaM1k5hq\n\
RoKcGLhysBrpgUlo4wkeDTI=\n\
-----END PRIVATE KEY-----";

    const RSA_PUBLIC_PEM: &str = "-----BEGIN PUBLIC KEY-----\n\
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA1E1mVojgmTNO2zb44dOl\n\
0RclQj3Z3lw+IqPrcabMsefOQyO7KgtyJUAN+l40t0fajVGkZgE5RqgJyJGKvQFc\n\
uPpVV3j/khuMOjTlRTqPwlhKfOuNLI8QkNgAivE1C8qBsluce6UtrugVytCiBAve\n\
MZDjFUfOaS3lYT+B+N5PKUfH489Cd0BRqSJxAGjaumoeJ3aeeHvkcVpmtF3uS64K\n\
dh+QuUh0yG74imzp6rAa7wh4r0OfLHaWwuMQUQv1YPd/V7b/p1yQ8eaNHvVe2Kzo\n\
z0IkZ90T6HBOqNzBBFITbvpH7q53AGRxvnbFCmqat0gNc2E9Q0jCTAvInfcE7nTP\n\
3QIDAQAB\n\
-----END PUBLIC KEY-----";

    const P256_PRIVATE_PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg6gpE7D0A3vQ4Rhwh\n\
iFhB4Ql4ppq+r1BHsP0/yLF561ShRANCAAR4dpLwmRbSAKhxB+6VHNB42u3FMa1p\n\
I2wZ6UEPzhnZbR9tgwtDYXPTC46hs6yfEHpHM99LjQE+hZKrmxdkoZF/\n\
-----END PRIVATE KEY-----";

    const P256_PUBLIC_PEM: &str = "-----BEGIN PUBLIC KEY-----\n\
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEeHaS8JkW0gCocQfulRzQeNrtxTGt\n\
aSNsGelBD84Z2W0fbYMLQ2Fz0wuOobOsnxB6RzPfS40BPoWSq5sXZKGRfw==\n\
-----END PUBLIC KEY-----";

    const P384_PRIVATE_PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
MIG2AgEAMBAGByqGSM49AgEGBSuBBAAiBIGeMIGbAgEBBDAzQBwhe66iZttc5kXv\n\
DifYHVKwFg0s/Iiih6jKKDfjLMkbH6oeEqdYqYx7//3+4cqhZANiAAQnJpU6AXZL\n\
PBOUHNNOtaKpHPpjVVaxmRvpGIXaOeMLmPBi4m7lHpHYBK2YEo233+iUV+n+Eqyz\n\
ahWTYfpPrk5fcLzJlW2o8y3U1nX3o12VKscmFB8qY5QiIRYkv75tV0s=\n\
-----END PRIVATE KEY-----";

    const P384_PUBLIC_PEM: &str = "-----BEGIN PUBLIC KEY-----\n\
MHYwEAYHKoZIzj0CAQYFK4EEACIDYgAEJyaVOgF2SzwTlBzTTrWiqRz6Y1VWsZkb\n\
6RiF2jnjC5jwYuJu5R6R2AStmBKNt9/olFfp/hKss2oVk2H6T65OX3C8yZVtqPMt\n\
1NZ196NdlSrHJhQfKmOUIiEWJL++bVdL\n\
-----END PUBLIC KEY-----";

    const ED25519_PRIVATE_PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
MC4CAQAwBQYDK2VwBCIEILHRdNnFQ5u89TSgCH8PNXpTBXV6TV9jEm5Pr67BZY+S\n\
-----END PRIVATE KEY-----";

    const ED25519_PUBLIC_PEM: &str = "-----BEGIN PUBLIC KEY-----\n\
MCowBQYDK2VwAyEAjEkPk860W+IPIawKrdm87SFkYtrUiwL+seJc9XY0eWY=\n\
-----END PUBLIC KEY-----";

    fn verify_with(token: &str, alg: JwtAlgorithm, key: &DecodingKey) -> (Value, Value) {
        let mut validation = Validation::new(alg.to_jwt());
        validation.validate_exp = false;
        validation.required_spec_claims.clear();
        let data = decode::<Value>(token, key, &validation).expect("valid token");
        let header = serde_json::to_value(data.header).expect("header");
        (header, data.claims)
    }

    #[test]
    fn signs_hmac_variants_including_base64_and_custom_header() {
        for alg in [JwtAlgorithm::HS256, JwtAlgorithm::HS384, JwtAlgorithm::HS512] {
            let login = JwtBearerLogin {
                algorithm: alg,
                secret: "top-secret".into(),
                payload: r#"{"sub":"u1","admin":true}"#.into(),
                header: r#"{"kid":"k-1","typ":"at+jwt"}"#.into(),
                ..Default::default()
            };
            let token = sign(&login).expect("signed");
            let (header, claims) = verify_with(&token, alg, &DecodingKey::from_secret(b"top-secret"));
            assert_eq!(header["alg"], json!(alg.as_str()));
            assert_eq!(header["kid"], json!("k-1"));
            assert_eq!(header["typ"], json!("at+jwt"));
            assert_eq!(claims, json!({"sub": "u1", "admin": true}));
        }

        let raw_secret = b"binary-\x01\x02-secret";
        let login = JwtBearerLogin {
            algorithm: JwtAlgorithm::HS256,
            secret: base64::engine::general_purpose::STANDARD.encode(raw_secret),
            secret_base64: true,
            payload: String::new(),
            header: String::new(),
            ..Default::default()
        };
        let token = sign(&login).expect("signed base64");
        let (header, claims) = verify_with(&token, JwtAlgorithm::HS256, &DecodingKey::from_secret(raw_secret));
        assert_eq!(header["typ"], json!("JWT"));
        assert_eq!(claims, json!({}));
    }

    #[test]
    fn signs_rsa_pss_ecdsa_and_eddsa() {
        let rsa_decode = DecodingKey::from_rsa_pem(RSA_PUBLIC_PEM.as_bytes()).expect("rsa pub");
        for alg in [
            JwtAlgorithm::RS256,
            JwtAlgorithm::RS384,
            JwtAlgorithm::RS512,
            JwtAlgorithm::PS256,
            JwtAlgorithm::PS384,
            JwtAlgorithm::PS512,
        ] {
            let token = sign(&JwtBearerLogin {
                algorithm: alg,
                secret: RSA_PRIVATE_PEM.into(),
                payload: r#"{"sub":"rsa"}"#.into(),
                ..Default::default()
            })
            .expect("rsa signed");
            let (_, claims) = verify_with(&token, alg, &rsa_decode);
            assert_eq!(claims["sub"], json!("rsa"));
        }

        let p256_decode = DecodingKey::from_ec_pem(P256_PUBLIC_PEM.as_bytes()).expect("p256 pub");
        let es256 = sign(&JwtBearerLogin {
            algorithm: JwtAlgorithm::ES256,
            secret: P256_PRIVATE_PEM.into(),
            payload: r#"{"sub":"ec256"}"#.into(),
            ..Default::default()
        })
        .expect("es256 signed");
        assert_eq!(
            verify_with(&es256, JwtAlgorithm::ES256, &p256_decode).1["sub"],
            json!("ec256")
        );

        let p384_decode = DecodingKey::from_ec_pem(P384_PUBLIC_PEM.as_bytes()).expect("p384 pub");
        let es384 = sign(&JwtBearerLogin {
            algorithm: JwtAlgorithm::ES384,
            secret: P384_PRIVATE_PEM.into(),
            payload: r#"{"sub":"ec384"}"#.into(),
            ..Default::default()
        })
        .expect("es384 signed");
        assert_eq!(
            verify_with(&es384, JwtAlgorithm::ES384, &p384_decode).1["sub"],
            json!("ec384")
        );

        let ed_decode = DecodingKey::from_ed_pem(ED25519_PUBLIC_PEM.as_bytes()).expect("ed pub");
        let ed = sign(&JwtBearerLogin {
            algorithm: JwtAlgorithm::EdDSA,
            secret: ED25519_PRIVATE_PEM.into(),
            payload: r#"{"sub":"ed"}"#.into(),
            ..Default::default()
        })
        .expect("eddsa signed");
        assert_eq!(verify_with(&ed, JwtAlgorithm::EdDSA, &ed_decode).1["sub"], json!("ed"));
    }

    #[test]
    fn refuses_bad_inputs_and_preserves_explicit_blank_prefix_in_json() {
        let missing = sign(&JwtBearerLogin::default()).expect_err("missing key");
        assert!(missing.contains("missing secret"));

        let bad_payload = sign(&JwtBearerLogin {
            secret: "s".into(),
            payload: "[1, 2]".into(),
            ..Default::default()
        })
        .expect_err("array payload");
        assert!(bad_payload.contains("payload must be a JSON object"));

        let bad_b64 = sign(&JwtBearerLogin {
            secret: "not!base64!".into(),
            secret_base64: true,
            ..Default::default()
        })
        .expect_err("bad base64");
        assert!(bad_b64.contains("Base64"));

        let bad_pem = sign(&JwtBearerLogin {
            algorithm: JwtAlgorithm::RS256,
            secret: "not-a-pem".into(),
            ..Default::default()
        })
        .expect_err("bad pem");
        assert!(bad_pem.contains("RSA PEM"));

        assert_eq!(header_value("Bearer", "tok"), "Bearer tok");
        assert_eq!(header_value("", "tok"), "tok");

        let explicit_empty = JwtBearer {
            prefix: String::new(),
            ..Default::default()
        };
        let json = serde_json::to_string(&explicit_empty).expect("json");
        let round_trip: JwtBearer = serde_json::from_str(&json).expect("read");
        assert_eq!(round_trip.prefix, "");
        assert_eq!(
            serde_json::from_str::<JwtBearer>("{}").expect("defaults").prefix,
            "Bearer"
        );
    }
}
