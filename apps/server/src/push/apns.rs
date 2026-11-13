//! The iPhone app's pushes: Apple Push Notification service (APNs) with a token-based key (`.p8`),
//! straight from this server (no Expo or other relay).
//!
//! - [`ApnsConfig`]: the `ORBIT_APNS_*` settings. Without them, devices still register, and nothing
//!   is pushed to them.
//! - [`ApnsSender`]: one HTTP/2 request per device, signed with a provider token (ES256 JWT) that
//!   is reused for [`TOKEN_LIFETIME`] (APNs refuses a new one more often than every 20 minutes).
//! - The device rows (`push_devices`) and their delivery, called by [`super::PushService`].

use std::sync::{Arc, Mutex};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64;
use futures_util::future::BoxFuture;
use orbit_platform::{Database, Id, Row, TimestampMillis};
use p256::SecretKey;
use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey};
use p256::pkcs8::DecodePrivateKey;
use serde::Serialize;
use serde_json::json;
use utoipa::ToSchema;

use super::{MAX_FAILURES, Notice, RETRY_AFTER, SEND_TIMEOUT, SendError, TTL_SECONDS};

/// The path of the `.p8` key file from Apple (Certificates, Identifiers & Profiles → Keys).
const KEY_FILE_ENV: &str = "ORBIT_APNS_KEY_FILE";
/// The key's 10-character Key ID.
const KEY_ID_ENV: &str = "ORBIT_APNS_KEY_ID";
/// The Apple Developer Team ID.
const TEAM_ID_ENV: &str = "ORBIT_APNS_TEAM_ID";
/// The app's bundle identifier (the APNs topic); default [`DEFAULT_TOPIC`].
const TOPIC_ENV: &str = "ORBIT_APNS_TOPIC";
/// `true`: the development (sandbox) APNs, for development builds. TestFlight and App Store builds
/// get production tokens, which the sandbox refuses.
const SANDBOX_ENV: &str = "ORBIT_APNS_SANDBOX";
pub const DEFAULT_TOPIC: &str = "com.easytomanage.orbit";

const PRODUCTION_HOST: &str = "https://api.push.apple.com";
const SANDBOX_HOST: &str = "https://api.sandbox.push.apple.com";
/// APNs accepts a provider token for one hour, and a new one at most every 20 minutes.
const TOKEN_LIFETIME: i64 = 50 * 60;
/// The largest payload APNs takes for an alert.
const MAX_PAYLOAD_BYTES: usize = 4096;
/// `apns-collapse-id` is at most 64 bytes.
const MAX_COLLAPSE_ID_BYTES: usize = 64;
/// The phones one user gets pushes on. The oldest go when another registers.
const MAX_DEVICES: i64 = 20;

/// Where a device's pushes go: the notice, sent to the device token.
pub trait DeviceSender: Send + Sync {
    fn send(&self, token: String, notice: Notice) -> BoxFuture<'static, Result<(), SendError>>;
}

#[derive(Clone)]
pub struct ApnsConfig {
    pub key: SecretKey,
    pub key_id: String,
    pub team_id: String,
    pub topic: String,
    pub sandbox: bool,
}

impl ApnsConfig {
    /// The settings from the environment: `Ok(None)` when APNs is not set up (no key file).
    pub fn from_env() -> Result<Option<Self>, String> {
        let var = |key: &str| {
            std::env::var(key)
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        };
        let Some(path) = var(KEY_FILE_ENV) else {
            return Ok(None);
        };
        let pem = std::fs::read_to_string(&path)
            .map_err(|error| format!("{KEY_FILE_ENV}: cannot read {path}: {error}"))?;
        let key = SecretKey::from_pkcs8_pem(pem.trim())
            .map_err(|_| format!("{KEY_FILE_ENV}: {path} is not an APNs .p8 key"))?;
        let key_id =
            var(KEY_ID_ENV).ok_or(format!("{KEY_ID_ENV} is required with {KEY_FILE_ENV}"))?;
        let team_id =
            var(TEAM_ID_ENV).ok_or(format!("{TEAM_ID_ENV} is required with {KEY_FILE_ENV}"))?;
        let sandbox = match var(SANDBOX_ENV).as_deref() {
            None | Some("false" | "0") => false,
            Some("true" | "1") => true,
            Some(other) => return Err(format!("{SANDBOX_ENV}={other}: use true or false")),
        };
        Ok(Some(Self {
            key,
            key_id,
            team_id,
            topic: var(TOPIC_ENV).unwrap_or_else(|| DEFAULT_TOPIC.to_owned()),
            sandbox,
        }))
    }
}

/// The provider token: `{"alg":"ES256","kid":…}.{"iss":team,"iat":…}`, signed with the key.
#[must_use]
pub fn provider_token(key: &SecretKey, key_id: &str, team_id: &str, issued_at: i64) -> String {
    let header = BASE64.encode(json!({ "alg": "ES256", "kid": key_id }).to_string());
    let claims = BASE64.encode(json!({ "iss": team_id, "iat": issued_at }).to_string());
    let signed = format!("{header}.{claims}");
    let signature: Signature = SigningKey::from(key).sign(signed.as_bytes());
    format!("{signed}.{}", BASE64.encode(signature.to_bytes()))
}

/// The APNs body of a notice: the alert, and the page to open (`url`) for the app.
#[must_use]
pub fn payload(notice: &Notice) -> Vec<u8> {
    json!({
        "aps": {
            "alert": { "title": notice.title, "body": notice.body },
            "sound": "default",
            "thread-id": notice.tag,
        },
        "url": notice.url,
        "tag": notice.tag,
    })
    .to_string()
    .into_bytes()
}

/// What an APNs answer means for the device.
pub fn outcome(status: u16, reason: &str) -> Result<(), SendError> {
    match (status, reason) {
        (200, _) => Ok(()),
        // The app was removed, or the token is not one of this app (or of this APNs environment).
        (410, _) | (400, "BadDeviceToken" | "DeviceTokenNotForTopic") => Err(SendError::Gone),
        (403, "ExpiredProviderToken") | (429 | 500..=599, _) => {
            Err(SendError::Busy(format!("APNs {status} {reason}")))
        }
        _ => Err(SendError::Failed(format!("APNs {status} {reason}"))),
    }
}

/// Sends to APNs over HTTP/2.
pub struct ApnsSender {
    config: ApnsConfig,
    client: reqwest::Client,
    /// The provider token and when it was made, in seconds. Shared with the send futures, which
    /// drop it when APNs says it expired.
    token: Arc<Mutex<Option<(String, i64)>>>,
}

impl ApnsSender {
    #[must_use]
    pub fn new(config: ApnsConfig) -> Self {
        let client = reqwest::Client::builder()
            .http2_prior_knowledge()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("the HTTP client of the APNs sender");
        Self {
            config,
            client,
            token: Arc::new(Mutex::new(None)),
        }
    }

    /// The sender of the `ORBIT_APNS_*` settings, or `None` without them (or when they are wrong,
    /// which is logged).
    #[must_use]
    pub fn from_env() -> Option<Arc<dyn DeviceSender>> {
        match ApnsConfig::from_env() {
            Ok(Some(config)) => {
                tracing::info!(
                    topic = config.topic,
                    sandbox = config.sandbox,
                    "iPhone push (APNs) is on"
                );
                Some(Arc::new(Self::new(config)))
            }
            Ok(None) => None,
            Err(error) => {
                tracing::error!(error, "iPhone push (APNs) is off");
                None
            }
        }
    }

    fn bearer(&self, now: i64) -> String {
        let mut token = self.token.lock().expect("APNs token lock");
        match token.as_ref() {
            Some((value, made)) if now - made < TOKEN_LIFETIME => value.clone(),
            _ => {
                let value = provider_token(
                    &self.config.key,
                    &self.config.key_id,
                    &self.config.team_id,
                    now,
                );
                *token = Some((value.clone(), now));
                value
            }
        }
    }
}

impl DeviceSender for ApnsSender {
    fn send(&self, token: String, notice: Notice) -> BoxFuture<'static, Result<(), SendError>> {
        let now = TimestampMillis::now().as_millis() / 1000;
        let bearer = self.bearer(now);
        let host = if self.config.sandbox {
            SANDBOX_HOST
        } else {
            PRODUCTION_HOST
        };
        let url = format!("{host}/3/device/{token}");
        let body = payload(&notice);
        let mut request = self
            .client
            .post(url)
            .header("authorization", format!("bearer {bearer}"))
            .header("apns-topic", &self.config.topic)
            .header("apns-push-type", "alert")
            .header("apns-priority", "10")
            .header(
                "apns-expiration",
                (now + i64::from(TTL_SECONDS)).to_string(),
            )
            .header("content-type", "application/json")
            .timeout(SEND_TIMEOUT);
        if notice.tag.len() <= MAX_COLLAPSE_ID_BYTES {
            request = request.header("apns-collapse-id", notice.tag.clone());
        }
        let cached = Arc::clone(&self.token);
        Box::pin(async move {
            if body.len() > MAX_PAYLOAD_BYTES {
                return Err(SendError::Failed("the notice is too large".to_owned()));
            }
            let response = request
                .body(body)
                .send()
                .await
                .map_err(|_| SendError::Busy("APNs did not answer".to_owned()))?;
            let status = response.status().as_u16();
            let reason = if status == 200 {
                String::new()
            } else {
                response
                    .json::<serde_json::Value>()
                    .await
                    .ok()
                    .and_then(|body| body["reason"].as_str().map(str::to_owned))
                    .unwrap_or_default()
            };
            if reason == "ExpiredProviderToken" {
                // The next send makes a new token.
                *cached.lock().expect("APNs token lock") = None;
            }
            outcome(status, &reason)
        })
    }
}

/// A phone the user gets pushes on.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct PushDeviceRecord {
    #[schema(value_type = String)]
    pub id: Id,
    pub label: String,
    #[schema(value_type = String, format = DateTime)]
    pub created_at: TimestampMillis,
}

/// The token in the form APNs takes (lowercase hex), or `None` when it is not a device token.
#[must_use]
pub fn normalized_token(token: &str) -> Option<String> {
    let token = token.trim();
    ((32..=200).contains(&token.len()) && token.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| token.to_ascii_lowercase())
}

/// Stores the device for the user and the session. A token has one row: it moves to whoever
/// registers it last. A user keeps the newest [`MAX_DEVICES`] only.
pub async fn register(
    database: &Database,
    user_id: Id,
    session_id: Id,
    token: &str,
    label: &str,
    now: TimestampMillis,
) -> Result<PushDeviceRecord, sqlx::Error> {
    let mut tx = database.pool().begin().await?;
    let row = sqlx::query(
        "INSERT INTO push_devices (id, user_id, session_id, token, label, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (token) DO UPDATE SET \
         user_id = excluded.user_id, session_id = excluded.session_id, label = excluded.label, \
         failures = 0 RETURNING id, created_at",
    )
    .bind(Id::new_v7().to_string())
    .bind(user_id.to_string())
    .bind(session_id.to_string())
    .bind(token)
    .bind(label)
    .bind(now.as_millis())
    .fetch_one(&mut *tx)
    .await?;
    let id: String = row.get("id");
    sqlx::query(
        "DELETE FROM push_devices WHERE user_id = $1 AND id NOT IN ( \
         SELECT id FROM push_devices WHERE user_id = $1 \
         ORDER BY id = $2 DESC, created_at DESC, id DESC LIMIT $3)",
    )
    .bind(user_id.to_string())
    .bind(&id)
    .bind(MAX_DEVICES)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(PushDeviceRecord {
        id: id
            .parse()
            .map_err(|error| sqlx::Error::Decode(Box::new(error)))?,
        label: label.to_owned(),
        created_at: TimestampMillis::from_millis(row.get("created_at")),
    })
}

/// Removes the user's device with this token; nothing happens when it is not theirs.
pub async fn unregister(database: &Database, user_id: Id, token: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM push_devices WHERE token = $1 AND user_id = $2")
        .bind(token)
        .bind(user_id.to_string())
        .execute(database.pool())
        .await?;
    Ok(())
}

/// Pushes to every device of the user whose session is still good; returns how many took it. A
/// device that APNs no longer knows is removed, and one that fails again and again too.
pub async fn deliver(
    database: &Database,
    sender: &Arc<dyn DeviceSender>,
    user_id: Id,
    notice: &Notice,
) -> Result<usize, sqlx::Error> {
    let now = TimestampMillis::now().as_millis();
    let rows = sqlx::query(
        "SELECT devices.id, devices.token FROM push_devices AS devices \
         JOIN sessions ON sessions.id = devices.session_id \
         WHERE devices.user_id = $1 AND sessions.revoked_at IS NULL \
         AND sessions.idle_expires_at > $2 AND sessions.absolute_expires_at > $3",
    )
    .bind(user_id.to_string())
    .bind(now)
    .bind(now)
    .fetch_all(database.pool())
    .await?;
    let sends = rows.into_iter().map(|row| {
        let (id, token): (String, String) = (row.get("id"), row.get("token"));
        let (sender, notice) = (Arc::clone(sender), notice.clone());
        async move {
            let mut result = sender.send(token.clone(), notice.clone()).await;
            // A busy APNs gets the push once more.
            if matches!(result, Err(SendError::Busy(_))) {
                tokio::time::sleep(RETRY_AFTER).await;
                result = sender.send(token, notice).await;
            }
            (id, result)
        }
    });
    let mut sent = 0;
    for (id, result) in futures_util::future::join_all(sends).await {
        match result {
            Ok(()) => {
                sent += 1;
                sqlx::query("UPDATE push_devices SET failures = 0 WHERE id = $1 AND failures > 0")
                    .bind(id)
                    .execute(database.pool())
                    .await?;
            }
            Err(SendError::Gone) => {
                sqlx::query("DELETE FROM push_devices WHERE id = $1")
                    .bind(id)
                    .execute(database.pool())
                    .await?;
            }
            Err(SendError::Failed(reason) | SendError::Busy(reason)) => {
                tracing::warn!(reason, "iPhone push not sent");
                sqlx::query("UPDATE push_devices SET failures = failures + 1 WHERE id = $1")
                    .bind(&id)
                    .execute(database.pool())
                    .await?;
                sqlx::query("DELETE FROM push_devices WHERE id = $1 AND failures >= $2")
                    .bind(&id)
                    .bind(MAX_FAILURES)
                    .execute(database.pool())
                    .await?;
            }
        }
    }
    Ok(sent)
}

#[cfg(test)]
mod tests {
    use p256::ecdsa::VerifyingKey;
    use p256::ecdsa::signature::Verifier;
    use p256::elliptic_curve::Generate;

    use super::*;

    #[test]
    fn the_provider_token_names_the_key_and_team_and_is_signed_by_the_key() {
        let key = SecretKey::generate();
        let token = provider_token(&key, "ABC123DEFG", "TEAM123456", 1_700_000_000);
        let mut parts = token.split('.');
        let (header, claims, signature) = (
            parts.next().unwrap(),
            parts.next().unwrap(),
            parts.next().unwrap(),
        );
        let decode = |part: &str| -> serde_json::Value {
            serde_json::from_slice(&BASE64.decode(part).unwrap()).unwrap()
        };
        assert_eq!(
            decode(header),
            json!({ "alg": "ES256", "kid": "ABC123DEFG" })
        );
        assert_eq!(
            decode(claims),
            json!({ "iss": "TEAM123456", "iat": 1_700_000_000 })
        );
        let signature = Signature::from_slice(&BASE64.decode(signature).unwrap()).unwrap();
        VerifyingKey::from(&SigningKey::from(&key))
            .verify(format!("{header}.{claims}").as_bytes(), &signature)
            .unwrap();
    }

    #[test]
    fn a_device_that_apns_no_longer_knows_is_gone_and_a_busy_apns_is_tried_again() {
        assert_eq!(outcome(200, ""), Ok(()));
        assert_eq!(outcome(410, "Unregistered"), Err(SendError::Gone));
        assert_eq!(outcome(400, "BadDeviceToken"), Err(SendError::Gone));
        assert_eq!(outcome(400, "DeviceTokenNotForTopic"), Err(SendError::Gone));
        assert!(matches!(
            outcome(429, "TooManyRequests"),
            Err(SendError::Busy(_))
        ));
        assert!(matches!(
            outcome(503, "ServiceUnavailable"),
            Err(SendError::Busy(_))
        ));
        assert!(matches!(
            outcome(403, "ExpiredProviderToken"),
            Err(SendError::Busy(_))
        ));
        // A key or topic problem is the server's, not the device's: the device stays for now.
        assert!(matches!(
            outcome(403, "InvalidProviderToken"),
            Err(SendError::Failed(_))
        ));
        assert!(matches!(
            outcome(400, "BadCollapseId"),
            Err(SendError::Failed(_))
        ));
    }

    #[test]
    fn only_hex_device_tokens_are_taken_in_lowercase() {
        let token = "A1".repeat(32);
        assert_eq!(
            normalized_token(&format!(" {token} ")),
            Some("a1".repeat(32))
        );
        assert_eq!(normalized_token("abc"), None);
        assert_eq!(normalized_token(&"zz".repeat(32)), None);
        assert_eq!(normalized_token(&"a".repeat(201)), None);
        assert_eq!(normalized_token("../../3/device/x"), None);
    }

    #[test]
    fn the_payload_has_the_alert_and_the_page_to_open() {
        let notice = Notice {
            title: "Ada".to_owned(),
            body: "hello".to_owned(),
            url: "/chat/1".to_owned(),
            tag: "chat:1".to_owned(),
            sound: "message",
            icon: None,
        };
        let body: serde_json::Value = serde_json::from_slice(&payload(&notice)).unwrap();
        assert_eq!(body["aps"]["alert"]["title"], "Ada");
        assert_eq!(body["aps"]["alert"]["body"], "hello");
        assert_eq!(body["aps"]["thread-id"], "chat:1");
        assert_eq!(body["url"], "/chat/1");
    }
}
