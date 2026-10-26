//! C5: the reader's client certificates, by host, and the authorities
//! trusted besides the system's. The settings keep the files' paths, not
//! their contents: a certificate renewed in place is the one the next
//! request sends. They are the reader's, like the proxy (#60): not synced,
//! not backed up.
//!
//! Only the desktop's engine reads these files (`NativeEngine::
//! with_certificates`). The bridge never does, whatever an account's
//! settings say: it would read its own machine's files — its own private
//! key — and hand them to any server.
//!
//! The rules are here; reading the files and the handshake are the
//! engine's (`cyberapi-http`).

use serde::{Deserialize, Serialize};

/// The settings' certificates.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CertificatePrefs {
    /// One per host; the most particular match is the one sent.
    pub clients: Vec<ClientCertificate>,
    /// PEM files of authorities trusted besides the system's.
    pub authorities: Vec<String>,
}

/// How a client certificate is kept.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CertificateFormat {
    /// A certificate (chain) file and a key file.
    #[default]
    Pem,
    /// One `.p12` / `.pfx` file, and its password.
    Pkcs12,
}

impl CertificateFormat {
    pub const ALL: [Self; 2] = [Self::Pem, Self::Pkcs12];
}

/// A client certificate and the host it is sent to.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ClientCertificate {
    /// `api.example.com`, `*.example.com` (the hosts below it), either
    /// with `:port`; without one, any port.
    pub host: String,
    pub format: CertificateFormat,
    /// The certificate, or the PKCS#12 file.
    pub certificate: String,
    /// The private key (PEM).
    pub key: String,
    /// The PKCS#12 file's.
    pub password: String,
}

impl ClientCertificate {
    /// How closely it is for `host` at `port`, if it is: an exact name
    /// before a wildcard, a port named before none.
    fn fit(&self, host: &str, port: Option<u16>) -> Option<u8> {
        let written = self.host.trim().to_ascii_lowercase();
        let (name, its_port) = match written.rsplit_once(':') {
            Some((name, written_port)) if !name.contains(':') => (name.to_string(), Some(written_port.parse().ok()?)),
            _ => (written, None),
        };
        if its_port.is_some() && its_port != port {
            return None;
        }
        let host = host.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
        let exact = match name.strip_prefix("*.") {
            None if name == host => true,
            Some(below) if host.ends_with(&format!(".{below}")) => false,
            _ => return None,
        };
        Some(u8::from(exact) * 2 + u8::from(its_port.is_some()))
    }

    /// Whether it names what it needs: a host, a file, and a PEM key.
    pub fn is_complete(&self) -> bool {
        !self.host.trim().is_empty()
            && !self.certificate.trim().is_empty()
            && (self.format == CertificateFormat::Pkcs12 || !self.key.trim().is_empty())
    }
}

/// What a request's TLS takes from the settings: the files the desktop's
/// engine reads for it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tls {
    pub client: Option<ClientCertificate>,
    pub authorities: Vec<String>,
}

impl Tls {
    /// Nothing besides the system's roots.
    pub fn is_plain(&self) -> bool {
        self.client.is_none() && self.authorities.is_empty()
    }
}

/// What a request to `url` takes: the certificate whose host fits it best,
/// and every authority. A plain `http://` takes none.
pub fn tls_for(prefs: &CertificatePrefs, url: &str) -> Tls {
    let Ok(url) = url::Url::parse(url) else {
        return Tls::default();
    };
    if url.scheme() != "https" {
        return Tls::default();
    }
    let host = url.host_str().unwrap_or_default();
    let port = url.port_or_known_default();
    let client = prefs
        .clients
        .iter()
        .filter(|client| client.is_complete())
        .filter_map(|client| Some((client.fit(host, port)?, client)))
        // The first written of the best.
        .fold(None::<(u8, &ClientCertificate)>, |best, (fit, client)| match best {
            Some((best_fit, _)) if best_fit >= fit => best,
            _ => Some((fit, client)),
        })
        .map(|(_, client)| client.clone());
    Tls {
        client,
        authorities: prefs
            .authorities
            .iter()
            .filter(|path| !path.trim().is_empty())
            .cloned()
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn client(host: &str, certificate: &str) -> ClientCertificate {
        ClientCertificate {
            host: host.into(),
            certificate: certificate.into(),
            key: "key.pem".into(),
            ..Default::default()
        }
    }

    fn chosen(prefs: &CertificatePrefs, url: &str) -> Option<String> {
        tls_for(prefs, url).client.map(|client| client.certificate)
    }

    #[test]
    fn the_most_particular_host_wins() {
        let prefs = CertificatePrefs {
            clients: vec![
                client("*.example.com", "wild"),
                client("api.example.com", "exact"),
                client("api.example.com:8443", "exact-port"),
                client("*.example.com:9443", "wild-port"),
            ],
            authorities: vec!["ca.pem".into(), " ".into()],
        };
        assert_eq!(chosen(&prefs, "https://API.example.com/x").as_deref(), Some("exact"));
        assert_eq!(
            chosen(&prefs, "https://api.example.com:8443/").as_deref(),
            Some("exact-port")
        );
        assert_eq!(chosen(&prefs, "https://a.b.example.com/").as_deref(), Some("wild"));
        assert_eq!(
            chosen(&prefs, "https://a.example.com:9443/").as_deref(),
            Some("wild-port")
        );
        assert_eq!(
            chosen(&prefs, "https://example.com/"),
            None,
            "a wildcard is for the hosts below"
        );
        assert_eq!(chosen(&prefs, "https://other.test/"), None);
        assert_eq!(chosen(&prefs, "http://api.example.com/"), None, "plain HTTP has no TLS");
        assert_eq!(tls_for(&prefs, "https://other.test/").authorities, ["ca.pem"]);
        assert!(tls_for(&prefs, "http://other.test/").is_plain());
    }

    #[test]
    fn a_default_port_is_the_schemes() {
        let prefs = CertificatePrefs {
            clients: vec![client("api.test:443", "443")],
            ..Default::default()
        };
        assert_eq!(chosen(&prefs, "https://api.test/").as_deref(), Some("443"));
        assert_eq!(chosen(&prefs, "https://api.test:8443/"), None);
    }

    #[test]
    fn an_incomplete_one_is_passed_over() {
        let pem_without_key = ClientCertificate {
            key: String::new(),
            ..client("api.test", "no-key")
        };
        let pkcs12 = ClientCertificate {
            format: CertificateFormat::Pkcs12,
            key: String::new(),
            ..client("api.test", "bundle.p12")
        };
        assert!(!pem_without_key.is_complete());
        assert!(pkcs12.is_complete());
        let prefs = CertificatePrefs {
            clients: vec![pem_without_key, pkcs12],
            ..Default::default()
        };
        assert_eq!(chosen(&prefs, "https://api.test/").as_deref(), Some("bundle.p12"));
    }
}
