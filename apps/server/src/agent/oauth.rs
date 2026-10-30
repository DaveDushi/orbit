//! OAuth client credentials for HTTP agent tools: Orbit gets the access token with the sealed
//! client ID and secret and renews it. The token stays in memory, per server: a new server just
//! asks for a new one.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use url::Url;

use super::outbound::{Credential, Outbound, OutboundError};

/// A token is renewed this long before it expires.
const RENEW_BEFORE_SECONDS: u64 = 60;

/// The client ID and secret, as they are sealed in `oauth_sealed_client`.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct OAuthClient {
    pub client_id: String,
    pub client_secret: String,
}

impl std::fmt::Debug for OAuthClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("OAuthClient([REDACTED])")
    }
}

/// An access token, only in memory.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct AccessToken(String);

impl AccessToken {
    pub(crate) fn new(token: String) -> Self {
        Self(token)
    }

    /// The header of one API call: `Bearer` only in `Authorization`, else the bare token.
    pub(crate) fn credential(&self, header: &str) -> Credential {
        Credential {
            name: header.to_owned(),
            value: if header.eq_ignore_ascii_case("authorization") {
                format!("Bearer {}", self.0)
            } else {
                self.0.clone()
            },
        }
    }
}

impl std::fmt::Debug for AccessToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AccessToken([REDACTED])")
    }
}

struct Cached {
    token: AccessToken,
    renew_at: Instant,
}

/// One integration's token. The async lock makes concurrent calls wait for one token request.
type Slot = Arc<tokio::sync::Mutex<Option<Cached>>>;

/// The tokens of this server, per integration and its `updated_at`: a config change on any server
/// is a new key, so the old token is not used again.
#[derive(Clone, Default)]
pub(crate) struct TokenCache(Arc<Mutex<HashMap<(String, i64), Slot>>>);

impl TokenCache {
    fn slot(&self, integration_id: &str, updated_at: i64) -> Slot {
        let mut slots = self.0.lock().unwrap_or_else(|error| error.into_inner());
        let now = Instant::now();
        // Old versions, and integrations that another server deleted, leave when nobody uses them.
        slots.retain(|(id, version), slot| {
            let current = id == integration_id && *version == updated_at;
            let in_use = Arc::strong_count(slot) > 1;
            let fresh = id != integration_id
                && slot.try_lock().is_ok_and(|cached| {
                    cached.as_ref().is_some_and(|cached| cached.renew_at > now)
                });
            current || in_use || fresh
        });
        Arc::clone(
            slots
                .entry((integration_id.to_owned(), updated_at))
                .or_default(),
        )
    }

    /// The cached token, or a new one. `failed` is a token the API refused: it is dropped only if
    /// it is still the cached one, so two concurrent 401s cause one renewal.
    pub(crate) async fn get(
        &self,
        outbound: &Outbound,
        (integration_id, updated_at): (&str, i64),
        (token_url, client): (&Url, &OAuthClient),
        failed: Option<&AccessToken>,
    ) -> Result<AccessToken, OutboundError> {
        let slot = self.slot(integration_id, updated_at);
        let mut cached = slot.lock().await;
        if let Some(current) = cached.as_ref()
            && current.renew_at > Instant::now()
            && failed != Some(&current.token)
        {
            return Ok(current.token.clone());
        }
        *cached = None;
        let (token, expires_in) = outbound.client_credentials(token_url, client).await?;
        *cached = expires_in
            .filter(|seconds| *seconds > RENEW_BEFORE_SECONDS)
            .and_then(|seconds| {
                Instant::now().checked_add(Duration::from_secs(seconds - RENEW_BEFORE_SECONDS))
            })
            .map(|renew_at| Cached {
                token: token.clone(),
                renew_at,
            });
        Ok(token)
    }

    /// The integration is deleted.
    pub(crate) fn forget(&self, integration_id: &str) {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .retain(|(id, _), _| id != integration_id);
    }
}

#[cfg(test)]
mod tests {
    use super::{AccessToken, OAuthClient};

    #[test]
    fn debug_output_redacts_tokens_and_clients() {
        let token = AccessToken::new("token-value".to_owned());
        let client = OAuthClient {
            client_id: "client-value".to_owned(),
            client_secret: "secret-value".to_owned(),
        };
        let text = format!("{token:?} {client:?}");
        assert!(!text.contains("value"), "{text}");
    }

    #[test]
    fn bearer_only_in_the_authorization_header() {
        let token = AccessToken::new("abc".to_owned());
        assert_eq!(token.credential("authorization").value, "Bearer abc");
        assert_eq!(token.credential("X-Shopify-Access-Token").value, "abc");
    }
}
