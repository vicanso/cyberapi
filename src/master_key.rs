//! F18: machine-local master key and AES-256-GCM encryption for local sensitive
//! values (secret variables, OAuth tokens, client certificate passwords, and
//! `server.json` session tokens).
//!
//! Key resolution (modeled on zedis's `master_key.rs`):
//! 1. In tests (`cfg(test)`): a fixed deterministic 256-bit key (never touches
//!    the OS keychain or `data_dir`).
//! 2. In release builds on macOS / Windows (when `CYBERAPI_DATA_DIR` is unset):
//!    OS keychain via `keyring` (`com.bigtree.cyberapi` / `master-key`),
//!    falling back to the `0600` key file on error.
//! 3. In debug builds (`debug_assertions` -> `master-dev.key`), Linux, or when
//!    `CYBERAPI_DATA_DIR` is set: a `0600` key file in `paths::data_dir()`.
//!
//! Ciphertext format: `enc:v1:<base64(nonce12 || ciphertext+tag)>`.
//! Values without the `enc:v1:` prefix are read as plaintext (lazy migration:
//! encrypted on next write). Values with `enc:v1:` that fail decryption (e.g.
//! after losing/resetting the master key) return `None` (treated as unset) and
//! flip [`LOST_KEY_NOTICE`] so the UI displays a non-blocking warning.

use aes_gcm::aead::{Aead as _, KeyInit as _};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::warn;

const PREFIX: &str = "enc:v1:";
const NONCE_LEN: usize = 12;

static RESOLVED_KEY: OnceLock<[u8; 32]> = OnceLock::new();
static LOST_KEY_NOTICE: AtomicBool = AtomicBool::new(false);

fn master_key() -> &'static [u8; 32] {
    RESOLVED_KEY.get_or_init(resolve_master_key)
}

/// Returns `true` once if any encrypted value failed to decrypt under the
/// current master key, so the UI can surface a non-blocking warning.
pub fn take_lost_key_notice() -> bool {
    LOST_KEY_NOTICE.swap(false, Ordering::Relaxed)
}

/// Encrypts `plaintext` with the machine-local master key. Empty strings stay
/// empty.
pub fn encrypt(plaintext: &str) -> String {
    encrypt_with_key(master_key(), plaintext)
}

/// Decrypts `stored` with the machine-local master key.
/// - Empty strings return `Some(String::new())`.
/// - Values without `enc:v1:` are returned as-is (lazy migration from plaintext).
/// - Values with `enc:v1:` that fail decryption return `None` and record
///   [`LOST_KEY_NOTICE`] so the UI notifies the reader to re-enter them.
pub fn decrypt(stored: &str) -> Option<String> {
    let result = decrypt_with_key(master_key(), stored);
    if result.is_none() {
        LOST_KEY_NOTICE.store(true, Ordering::Relaxed);
    }
    result
}

pub fn encrypt_with_key(key: &[u8; 32], plaintext: &str) -> String {
    if plaintext.is_empty() {
        return String::new();
    }
    let mut nonce_bytes = [0u8; NONCE_LEN];
    if let Err(error) = getrandom::fill(&mut nonce_bytes) {
        warn!(%error, "system RNG failed while encrypting sensitive value");
        return plaintext.to_string();
    }
    let cipher = Aes256Gcm::new(key.into());
    let nonce = Nonce::from(nonce_bytes);
    match cipher.encrypt(&nonce, plaintext.as_bytes()) {
        Ok(ciphertext) => {
            let mut combined = Vec::with_capacity(NONCE_LEN + ciphertext.len());
            combined.extend_from_slice(&nonce_bytes);
            combined.extend_from_slice(&ciphertext);
            format!("{PREFIX}{}", BASE64.encode(&combined))
        }
        Err(error) => {
            warn!(%error, "AES-GCM encryption failed");
            plaintext.to_string()
        }
    }
}

pub fn decrypt_with_key(key: &[u8; 32], stored: &str) -> Option<String> {
    if stored.is_empty() {
        return Some(String::new());
    }
    let Some(encoded) = stored.strip_prefix(PREFIX) else {
        // Lazy migration: legacy plaintext is returned as-is and encrypted on next write.
        return Some(stored.to_string());
    };
    let bytes = BASE64.decode(encoded.trim()).ok()?;
    if bytes.len() <= NONCE_LEN {
        warn!("encrypted value is truncated; treating as unset");
        return None;
    }
    let (nonce_bytes, ciphertext) = bytes.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new(key.into());
    let nonce = Nonce::try_from(nonce_bytes).ok()?;
    let plaintext = match cipher.decrypt(&nonce, ciphertext) {
        Ok(plain) => plain,
        Err(_) => {
            warn!("encrypted value could not be decrypted with the current master key; treating as unset");
            return None;
        }
    };
    match String::from_utf8(plaintext) {
        Ok(text) => Some(text),
        Err(_) => {
            warn!("decrypted sensitive value is not valid UTF-8; treating as unset");
            None
        }
    }
}

/// [`cyberapi_store_turso::ValueCipher`] backed by the machine-local master key.
pub struct MasterKeyCipher;

impl cyberapi_store_turso::ValueCipher for MasterKeyCipher {
    fn encrypt(&self, plaintext: &str) -> String {
        encrypt(plaintext)
    }

    fn decrypt(&self, stored: &str) -> Option<String> {
        decrypt(stored)
    }
}

#[cfg(test)]
fn resolve_master_key() -> [u8; 32] {
    *b"cyberapi-test-master-key-0123456"
}

#[cfg(not(test))]
fn resolve_master_key() -> [u8; 32] {
    real::resolve()
}

#[cfg(not(test))]
mod real {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;
    use std::path::Path;
    use tracing::warn;

    #[cfg(not(debug_assertions))]
    const KEY_FILE_NAME: &str = "master.key";
    #[cfg(debug_assertions)]
    const KEY_FILE_NAME: &str = "master-dev.key";

    #[cfg(all(any(target_os = "macos", target_os = "windows"), not(debug_assertions)))]
    const KEYRING_SERVICE: &str = "com.bigtree.cyberapi";
    #[cfg(all(any(target_os = "macos", target_os = "windows"), not(debug_assertions)))]
    const KEYRING_ACCOUNT: &str = "master-key";

    pub(super) fn resolve() -> [u8; 32] {
        #[cfg(all(any(target_os = "macos", target_os = "windows"), not(debug_assertions)))]
        {
            if std::env::var_os("CYBERAPI_DATA_DIR").is_none() {
                match keyring_key() {
                    Ok(key) => return key,
                    Err(error) => warn!(%error, "OS keychain unavailable; falling back to key file"),
                }
            }
        }
        match file_key() {
            Ok(key) => key,
            Err(error) => {
                warn!(%error, "master key file unavailable; using ephemeral key for this session");
                generate_key()
            }
        }
    }

    fn generate_key() -> [u8; 32] {
        let mut key = [0u8; 32];
        if let Err(error) = getrandom::fill(&mut key) {
            warn!(%error, "system RNG failed while generating master key");
        }
        key
    }

    fn decode_key(raw: &str) -> Option<[u8; 32]> {
        let bytes = BASE64.decode(raw.trim()).ok()?;
        bytes.as_slice().try_into().ok()
    }

    #[cfg(all(any(target_os = "macos", target_os = "windows"), not(debug_assertions)))]
    fn keyring_key() -> Result<[u8; 32], keyring::Error> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)?;
        match entry.get_password() {
            Ok(stored) => {
                if let Some(key) = decode_key(&stored) {
                    return Ok(key);
                }
                let key = generate_key();
                entry.set_password(&BASE64.encode(key))?;
                Ok(key)
            }
            Err(keyring::Error::NoEntry) => {
                let key = generate_key();
                entry.set_password(&BASE64.encode(key))?;
                Ok(key)
            }
            Err(error) => Err(error),
        }
    }

    fn file_key() -> std::io::Result<[u8; 32]> {
        let dir = crate::paths::data_dir()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no data directory"))?;
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(KEY_FILE_NAME);
        if let Ok(stored) = std::fs::read_to_string(&path)
            && let Some(key) = decode_key(&stored)
        {
            return Ok(key);
        }
        let key = generate_key();
        write_key_file(&path, &key)?;
        Ok(key)
    }

    fn write_key_file(path: &Path, key: &[u8; 32]) -> std::io::Result<()> {
        let encoded = BASE64.encode(key);
        #[cfg(unix)]
        {
            use std::io::Write as _;
            use std::os::unix::fs::OpenOptionsExt as _;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(path)?;
            file.write_all(encoded.as_bytes())?;
            Ok(())
        }
        #[cfg(not(unix))]
        {
            std::fs::write(path, encoded)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cyberapi_core::certificates::{CertificateFormat, ClientCertificate};
    use cyberapi_core::oauth::{KeptToken, KeptTokens};
    use cyberapi_core::prefs::AppPrefs;
    use cyberapi_core::secrets::SecretValues;
    use cyberapi_store::{NewCollection, PrefStore as _};
    use cyberapi_store_turso::{TursoStore, ValueCipher};
    use std::collections::BTreeMap;
    use std::sync::Arc;

    #[test]
    fn encrypt_decrypt_round_trip_and_lazy_plaintext_migration() {
        let key = [7u8; 32];
        let enc = encrypt_with_key(&key, "my-secret-token");
        assert!(enc.starts_with(PREFIX));
        assert_ne!(enc, "my-secret-token");
        assert_eq!(decrypt_with_key(&key, &enc).as_deref(), Some("my-secret-token"));

        // Plaintext without prefix passes through unchanged (lazy migration).
        assert_eq!(decrypt_with_key(&key, "legacy-plain").as_deref(), Some("legacy-plain"));
        assert_eq!(decrypt_with_key(&key, "").as_deref(), Some(""));
        assert_eq!(encrypt_with_key(&key, ""), "");

        // Wrong key returns None (treated as unset).
        let wrong_key = [9u8; 32];
        assert_eq!(decrypt_with_key(&wrong_key, &enc), None);

        // Corrupted ciphertext returns None.
        assert_eq!(decrypt_with_key(&key, "enc:v1:not-valid-base64!!!"), None);
    }

    #[test]
    fn decrypt_failure_sets_and_consumes_lost_key_notice() {
        let _ = take_lost_key_notice();
        let other_key = [42u8; 32];
        let unopenable = encrypt_with_key(&other_key, "lost-secret");
        assert_eq!(decrypt(&unopenable), None);
        assert!(take_lost_key_notice(), "first take sees the lost-key notice");
        assert!(!take_lost_key_notice(), "second take is already consumed");
    }

    struct FixedKeyCipher([u8; 32]);
    impl ValueCipher for FixedKeyCipher {
        fn encrypt(&self, plaintext: &str) -> String {
            encrypt_with_key(&self.0, plaintext)
        }
        fn decrypt(&self, stored: &str) -> Option<String> {
            decrypt_with_key(&self.0, stored)
        }
    }

    #[test]
    fn turso_store_encrypts_secrets_oauth_and_cert_passwords_and_survives_key_loss() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dir.path().join("f18.db");
        let key_a = [1u8; 32];
        let key_b = [2u8; 32];

        // 1. Write legacy plaintext without a cipher.
        let collection_id = pollster::block_on(async {
            let plain_store = TursoStore::open(&db).await.expect("open plain");
            let col = plain_store
                .storage()
                .collections()
                .create(NewCollection {
                    name: "F18".into(),
                    ..Default::default()
                })
                .await
                .expect("create");
            let mut sv = BTreeMap::new();
            sv.insert(cyberapi_core::VariableId::from("v1"), "legacy-secret".into());
            plain_store
                .set_secret_values(col.id.clone(), SecretValues(sv))
                .await
                .expect("set plain secret");
            col.id
        });

        // 2. Re-open with key_a: legacy plaintext reads cleanly, new writes encrypt on disk.
        pollster::block_on(async {
            let store = TursoStore::open_with_cipher(&db, Arc::new(FixedKeyCipher(key_a)))
                .await
                .expect("open cipher A");

            let loaded = store.secret_values(collection_id.clone()).await.expect("read legacy");
            assert_eq!(
                loaded.get(&cyberapi_core::VariableId::from("v1")),
                Some("legacy-secret")
            );

            // Rewrite secret, OAuth token, and client cert password under key_a.
            store
                .set_secret_values(collection_id.clone(), loaded)
                .await
                .expect("rewrite encrypted");

            let mut tokens = BTreeMap::new();
            tokens.insert(
                "k1".into(),
                KeptToken {
                    access_token: "acc-123".into(),
                    refresh_token: Some("ref-456".into()),
                    expires_at: None,
                },
            );
            store.set_oauth_tokens(KeptTokens(tokens)).await.expect("set oauth");

            let mut prefs = AppPrefs::default();
            prefs.sending.certificates.clients.push(ClientCertificate {
                host: "api.test".into(),
                format: CertificateFormat::Pkcs12,
                certificate: "/tmp/client.p12".into(),
                key: String::new(),
                password: "p12-password".into(),
            });
            store.set_app(prefs).await.expect("set app");

            // Reading back through key_a returns plaintext.
            assert_eq!(
                store
                    .secret_values(collection_id.clone())
                    .await
                    .expect("secrets")
                    .get(&cyberapi_core::VariableId::from("v1")),
                Some("legacy-secret")
            );
            let got_tokens = store.oauth_tokens().await.expect("oauth");
            assert_eq!(got_tokens.0["k1"].access_token, "acc-123");
            assert_eq!(got_tokens.0["k1"].refresh_token.as_deref(), Some("ref-456"));
            let got_prefs = store.app().await.expect("app");
            assert_eq!(got_prefs.sending.certificates.clients[0].password, "p12-password");
        });

        // 3. Raw read without cipher confirms ciphertext (`enc:v1:`) is stored on disk.
        pollster::block_on(async {
            let raw = TursoStore::open(&db).await.expect("open raw");
            let raw_secrets = raw.secret_values(collection_id.clone()).await.expect("raw secrets");
            assert!(
                raw_secrets
                    .get(&cyberapi_core::VariableId::from("v1"))
                    .is_some_and(|v| v.starts_with(PREFIX))
            );
            let raw_tokens = raw.oauth_tokens().await.expect("raw oauth");
            assert!(raw_tokens.0["k1"].access_token.starts_with(PREFIX));
            assert!(
                raw_tokens.0["k1"]
                    .refresh_token
                    .as_deref()
                    .is_some_and(|v| v.starts_with(PREFIX))
            );
            let raw_prefs = raw.app().await.expect("raw app");
            assert!(raw_prefs.sending.certificates.clients[0].password.starts_with(PREFIX));
        });

        // 4. Re-open with a different key (simulating lost master key): undecryptable
        // values are treated as unset without erroring.
        pollster::block_on(async {
            let store = TursoStore::open_with_cipher(&db, Arc::new(FixedKeyCipher(key_b)))
                .await
                .expect("open cipher B");
            assert!(
                store
                    .secret_values(collection_id.clone())
                    .await
                    .expect("secrets")
                    .is_empty()
            );
            assert!(store.oauth_tokens().await.expect("oauth").0.is_empty());
            assert!(
                store.app().await.expect("app").sending.certificates.clients[0]
                    .password
                    .is_empty()
            );
        });
    }
}
