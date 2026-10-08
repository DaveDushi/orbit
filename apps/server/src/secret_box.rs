//! Secrets at rest (GitHub App keys, Notion import tokens): ChaCha20-Poly1305 under the
//! server's app key (`secrets.app_key`). The stored value is `nonce (12 bytes) || ciphertext`.

use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};

/// Encrypts `value` with a fresh random nonce.
pub(crate) fn encrypt_secret(key: &[u8; 32], value: &str) -> Result<Vec<u8>, ()> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
    let encrypted = cipher.encrypt(&nonce, value.as_bytes()).map_err(|_| ())?;
    let mut result = nonce.to_vec();
    result.extend_from_slice(&encrypted);
    Ok(result)
}

/// Decrypts a value from [`encrypt_secret`]; fails for another key or altered bytes.
pub(crate) fn decrypt_secret(key: &[u8; 32], value: &[u8]) -> Result<String, ()> {
    if value.len() < 28 {
        return Err(());
    }
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let plaintext = cipher
        .decrypt(Nonce::from_slice(&value[..12]), &value[12..])
        .map_err(|_| ())?;
    String::from_utf8(plaintext).map_err(|_| ())
}

/// The key every secret at rest is encrypted with: `secrets.app_key` from the configuration when set, else the
/// `app.key` file beside the database, created with a random key on first start. The file is part of the data, so
/// keep it with backups: without it, saved secrets (SMTP password, GitHub App keys) cannot be read.
pub(crate) fn load_or_create_app_key(
    configured: Option<&str>,
    database_path: &std::path::Path,
) -> Result<[u8; 32], String> {
    use base64::Engine as _;
    use std::io::Write as _;

    let decode = |value: &str, source: &str| {
        base64::engine::general_purpose::STANDARD
            .decode(value.trim())
            .ok()
            .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
            .ok_or_else(|| format!("{source} must be a base64-encoded 32-byte key"))
    };
    if let Some(value) = configured {
        return decode(value, "secrets.app_key");
    }
    let path = database_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("app.key");
    match std::fs::read_to_string(&path) {
        Ok(value) => return decode(&value, &path.display().to_string()),
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(format!("{} could not be read: {error}", path.display()));
        }
        Err(_) => {}
    }
    let key: [u8; 32] = ChaCha20Poly1305::generate_key(&mut OsRng).into();
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let written = options.open(&path).and_then(|mut file| {
        file.write_all(
            base64::engine::general_purpose::STANDARD
                .encode(key)
                .as_bytes(),
        )?;
        file.sync_all()
    });
    match written {
        Ok(()) => Ok(key),
        // Another process created it first: use theirs.
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => decode(
            &std::fs::read_to_string(&path).map_err(|e| e.to_string())?,
            "app.key",
        ),
        Err(error) => Err(format!("{} could not be created: {error}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::{decrypt_secret, encrypt_secret, load_or_create_app_key};

    #[test]
    fn a_generated_app_key_is_stored_once_and_reused() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("orbit.sqlite");
        let first = load_or_create_app_key(None, &database).unwrap();
        let second = load_or_create_app_key(None, &database).unwrap();
        assert_eq!(first, second);
        let sealed = encrypt_secret(&first, "smtp password").unwrap();
        assert_eq!(decrypt_secret(&second, &sealed).unwrap(), "smtp password");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(directory.path().join("app.key"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(load_or_create_app_key(Some("not base64"), &database).is_err());
    }
}
