//! Secure storage for tokens and cookies.
//!
//! rustypipe keeps its OAuth token and cookie in one cache blob. We encrypt that blob with
//! AES-256-GCM. The key lives in the OS keyring on desktop (Windows Credential Manager,
//! Secret Service); on Android the Kotlin shell wraps it with an Android Keystore key and
//! passes the raw key in at startup.

use std::path::PathBuf;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use rand::RngCore;

pub type Key = [u8; 32];

pub fn random_key() -> Key {
    let mut k = [0u8; 32];
    rand::rng().fill_bytes(&mut k);
    k
}

/// Load the desktop key from the OS keyring, creating it on first run.
///
/// If no keyring service is available (e.g. a headless Linux session) the key falls back to a
/// file readable only by the user, and a warning is logged.
#[cfg(not(target_os = "android"))]
pub fn desktop_key(fallback_dir: &std::path::Path) -> Key {
    let b64 = base64::engine::general_purpose::STANDARD;
    let entry = keyring::Entry::new("StreamPipe", "cache-key");
    if let Ok(entry) = &entry {
        match entry.get_password() {
            Ok(s) => {
                if let Ok(v) = b64.decode(s.trim()) {
                    if let Ok(k) = <Key>::try_from(v.as_slice()) {
                        return k;
                    }
                }
            }
            Err(keyring::Error::NoEntry) => {
                let k = random_key();
                if entry.set_password(&b64.encode(k)).is_ok() {
                    return k;
                }
            }
            Err(e) => tracing::warn!("keyring unavailable: {e}"),
        }
    }
    tracing::warn!("falling back to a key file; tokens are less protected on this system");
    let path = fallback_dir.join("cache.key");
    if let Ok(v) = std::fs::read(&path) {
        if let Ok(k) = <Key>::try_from(v.as_slice()) {
            return k;
        }
    }
    let k = random_key();
    let _ = std::fs::create_dir_all(fallback_dir);
    let _ = std::fs::write(&path, k);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    k
}

/// A file whose contents are AES-256-GCM encrypted: 12-byte nonce, then ciphertext.
pub struct EncryptedFile {
    path: PathBuf,
    cipher: Aes256Gcm,
}

impl EncryptedFile {
    pub fn new(path: PathBuf, key: &Key) -> Self {
        Self {
            path,
            cipher: Aes256Gcm::new(key.into()),
        }
    }

    pub fn read(&self) -> Option<Vec<u8>> {
        let raw = std::fs::read(&self.path).ok()?;
        if raw.len() < 12 {
            return None;
        }
        let (nonce, ct) = raw.split_at(12);
        self.cipher
            .decrypt(Nonce::from_slice(nonce), ct)
            .map_err(|_| tracing::warn!("could not decrypt {}", self.path.display()))
            .ok()
    }

    pub fn write(&self, plain: &[u8]) -> std::io::Result<()> {
        let mut nonce = [0u8; 12];
        rand::rng().fill_bytes(&mut nonce);
        let ct = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce), plain)
            .map_err(|_| std::io::Error::other("encryption failed"))?;
        let mut out = nonce.to_vec();
        out.extend_from_slice(&ct);
        crate::config::write_atomic(&self.path, &out)
    }

    pub fn remove(&self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// rustypipe cache storage backed by an [`EncryptedFile`].
pub struct EncryptedCacheStorage(pub EncryptedFile);

impl rustypipe::cache::CacheStorage for EncryptedCacheStorage {
    fn write(&self, data: &str) {
        if let Err(e) = self.0.write(data.as_bytes()) {
            tracing::error!("could not write session cache: {e}");
        }
    }

    fn read(&self) -> Option<String> {
        self.0.read().and_then(|b| String::from_utf8(b).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_tamper() {
        let dir = tempfile::tempdir().unwrap();
        let key = random_key();
        let f = EncryptedFile::new(dir.path().join("c.bin"), &key);
        f.write(b"secret token").unwrap();
        assert_eq!(f.read().unwrap(), b"secret token");
        let raw = std::fs::read(dir.path().join("c.bin")).unwrap();
        assert!(!raw.windows(6).any(|w| w == b"secret"));
        let other = EncryptedFile::new(dir.path().join("c.bin"), &random_key());
        assert!(other.read().is_none());
    }
}
