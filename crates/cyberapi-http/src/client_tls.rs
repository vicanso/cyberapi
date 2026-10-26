//! C5: the handshake of a request whose settings name a client certificate,
//! or authorities trusted besides the system's. Its files are read for
//! each such request, so a certificate renewed in place is the one sent;
//! one that cannot be read fails the request, saying which and why.
//!
//! A certificate is PEM — the certificate (its chain may follow it) and an
//! unencrypted key, PKCS#8, PKCS#1 or SEC1 — or one PKCS#12 file and its
//! password. Only an engine made `NativeEngine::with_certificates` (the
//! desktop's) comes here.

use crate::SendError;
use crate::native::{AnyCertificate, alpn, system_roots};
use cyberapi_core::certificates::{CertificateFormat, ClientCertificate, Tls};
use cyberapi_core::prefs::HttpVersion;
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use std::sync::Arc;

/// The client's TLS for a request that takes `tls`: the system's roots and
/// its authorities, or any certificate when `check_certs` is off; its
/// client certificate, if it names one.
pub(crate) fn config(
    tls: &Tls,
    check_certs: bool,
    version: HttpVersion,
) -> Result<Arc<rustls::ClientConfig>, SendError> {
    let failed = |message: String| SendError::Tls { message };
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .expect("ring supports the default TLS versions");
    let builder = if check_certs {
        let mut roots = system_roots().clone();
        for path in &tls.authorities {
            for cert in pem_certificates(path).map_err(failed)? {
                roots.add(cert).map_err(|error| failed(format!("{path}: {error}")))?;
            }
        }
        builder.with_root_certificates(roots)
    } else {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AnyCertificate(provider)))
    };
    let mut config = match &tls.client {
        Some(client) => {
            let (chain, key) = identity(client).map_err(failed)?;
            builder
                .with_client_auth_cert(chain, key)
                .map_err(|error| failed(format!("{}: {error}", client.certificate)))?
        }
        None => builder.with_no_client_auth(),
    };
    config.alpn_protocols = alpn(version);
    Ok(Arc::new(config))
}

fn read(path: &str) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|error| format!("{path}: {error}"))
}

/// Every certificate in a PEM file.
fn pem_certificates(path: &str) -> Result<Vec<CertificateDer<'static>>, String> {
    let bytes = read(path)?;
    let certs = CertificateDer::pem_slice_iter(&bytes)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("{path}: {error}"))?;
    if certs.is_empty() {
        return Err(format!("{path}: no PEM certificate in it"));
    }
    Ok(certs)
}

/// The chain a client presents, and its key.
fn identity(client: &ClientCertificate) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>), String> {
    match client.format {
        CertificateFormat::Pem => {
            let chain = pem_certificates(&client.certificate)?;
            let key = PrivateKeyDer::from_pem_slice(&read(&client.key)?)
                .map_err(|error| format!("{}: no unencrypted private key in it ({error})", client.key))?;
            Ok((chain, key))
        }
        CertificateFormat::Pkcs12 => {
            let path = &client.certificate;
            let store = p12_keystore::KeyStore::from_pkcs12(
                &read(path)?,
                &client.password,
                p12_keystore::Pkcs12ImportPolicy::Relaxed,
            )
            .map_err(|error| format!("{path}: {error}"))?;
            let (_, found) = store
                .private_key_chain()
                .ok_or_else(|| format!("{path}: no private key in it"))?;
            let chain: Vec<CertificateDer<'static>> = found
                .certs()
                .iter()
                .map(|cert| CertificateDer::from(cert.as_der().to_vec()))
                .collect();
            if chain.is_empty() {
                return Err(format!("{path}: no certificate with its private key"));
            }
            let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(found.key().as_der().to_vec()));
            Ok((chain, key))
        }
    }
}
