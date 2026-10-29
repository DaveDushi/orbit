//! Each person's ChatGPT sign-in, sealed with the app key. Only the sidecar reads the open
//! credential (to call the model and to refresh it); the browser only learns whether it exists.

use orbit_platform::{Database, Id, Row, TimestampMillis};
use serde::Serialize;
use serde_json::Value;

use crate::secret_box::{decrypt_secret, encrypt_secret};

/// What a person sees about their own connection.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ConnectionStatus {
    pub connected: bool,
    /// A refresh failed: the person must sign in again.
    pub invalid: bool,
    pub account_id: Option<String>,
    pub connected_at: Option<i64>,
}

#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("the stored credential cannot be opened with this app key")]
    Unreadable,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// Seals a credential (any JSON) for storage.
pub(crate) fn seal(key: &[u8; 32], credential: &Value) -> Result<Vec<u8>, CredentialError> {
    encrypt_secret(key, &credential.to_string()).map_err(|()| CredentialError::Unreadable)
}

/// Opens a sealed credential; fails for another key or changed bytes.
pub(crate) fn open(key: &[u8; 32], sealed: &[u8]) -> Result<Value, CredentialError> {
    let text = decrypt_secret(key, sealed).map_err(|()| CredentialError::Unreadable)?;
    serde_json::from_str(&text).map_err(|_| CredentialError::Unreadable)
}

pub(crate) async fn status(
    database: &Database,
    user_id: Id,
) -> Result<ConnectionStatus, sqlx::Error> {
    let row = sqlx::query(
        "SELECT account_id, invalid_at, created_at FROM agent_credentials WHERE user_id = $1",
    )
    .bind(user_id.to_string())
    .fetch_optional(database.pool())
    .await?;
    Ok(
        row.map_or_else(ConnectionStatus::default, |row| ConnectionStatus {
            connected: true,
            invalid: row.get::<Option<i64>, _>("invalid_at").is_some(),
            account_id: row.get("account_id"),
            connected_at: Some(row.get("created_at")),
        }),
    )
}

/// The open credential, or `None` when the person has not connected (or must connect again).
pub(crate) async fn read(
    database: &Database,
    key: &[u8; 32],
    user_id: Id,
) -> Result<Option<Value>, CredentialError> {
    let sealed = sqlx::query_scalar::<_, Vec<u8>>(
        "SELECT sealed FROM agent_credentials WHERE user_id = $1 AND invalid_at IS NULL",
    )
    .bind(user_id.to_string())
    .fetch_optional(database.pool())
    .await?;
    sealed.map(|sealed| open(key, &sealed)).transpose()
}

/// Saves a new or refreshed credential; a saved credential is valid again.
pub(crate) async fn write(
    database: &Database,
    key: &[u8; 32],
    user_id: Id,
    credential: &Value,
    account_id: Option<&str>,
) -> Result<(), CredentialError> {
    let sealed = seal(key, credential)?;
    let now = TimestampMillis::now().as_millis();
    sqlx::query(
        "INSERT INTO agent_credentials (user_id, sealed, account_id, invalid_at, created_at, updated_at) \
         VALUES ($1, $2, $3, NULL, $4, $4) \
         ON CONFLICT (user_id) DO UPDATE SET sealed = excluded.sealed, \
         account_id = COALESCE(excluded.account_id, agent_credentials.account_id), \
         invalid_at = NULL, updated_at = excluded.updated_at",
    )
    .bind(user_id.to_string())
    .bind(sealed)
    .bind(account_id)
    .bind(now)
    .execute(database.pool())
    .await?;
    Ok(())
}

pub(crate) async fn mark_invalid(database: &Database, user_id: Id) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE agent_credentials SET invalid_at = $1 WHERE user_id = $2 AND invalid_at IS NULL",
    )
    .bind(TimestampMillis::now().as_millis())
    .bind(user_id.to_string())
    .execute(database.pool())
    .await?;
    Ok(())
}

pub(crate) async fn delete(database: &Database, user_id: Id) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM agent_credentials WHERE user_id = $1")
        .bind(user_id.to_string())
        .execute(database.pool())
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{open, seal};

    #[test]
    fn a_sealed_credential_hides_the_tokens_and_opens_only_with_its_key() {
        let credential = json!({
            "type": "oauth",
            "access": "secret-access-token",
            "refresh": "secret-refresh-token",
            "expires": 1_900_000_000_000_i64,
        });
        let sealed = seal(&[7; 32], &credential).unwrap();
        let text = String::from_utf8_lossy(&sealed);
        assert!(!text.contains("secret-access-token"));
        assert!(!text.contains("secret-refresh-token"));
        assert_eq!(open(&[7; 32], &sealed).unwrap(), credential);
        assert!(open(&[8; 32], &sealed).is_err());

        let mut changed = sealed.clone();
        let last = changed.len() - 1;
        changed[last] ^= 1;
        assert!(open(&[7; 32], &changed).is_err());
    }

    #[test]
    fn sealing_twice_gives_different_bytes() {
        let credential = json!({"access": "a"});
        assert_ne!(
            seal(&[7; 32], &credential).unwrap(),
            seal(&[7; 32], &credential).unwrap()
        );
    }
}
