//! The team agent, Orbit's side. The agent itself runs in a Node sidecar (`apps/agent`) on Pi
//! Durable; Orbit keeps who it is in chat (the bot user), the queue of chat messages for it, the
//! state of its threads, each person's sealed ChatGPT sign-in, and the short sessions it uses to
//! act as one person. Design and its reasons: `.ai/agent-design.md`.

pub(crate) mod bot;
pub(crate) mod credentials;
pub mod inputs;
pub mod memory;
mod memory_routes;
mod routes;
pub mod scope;
pub(crate) mod sessions;

use std::sync::Arc;

/// The agent's routes: the sidecar's, the ChatGPT connection, and the memory.
pub fn router(state: AgentState) -> axum::Router {
    routes::router(state.clone()).merge(memory_routes::router(state))
}

use crate::auth_routes::CookieMode;
use crate::chat_routes::ChatState;
use crate::repositories::identity::IdentityRepository;

/// How Orbit reaches the sidecar. Without it the agent is off.
#[derive(Clone)]
pub struct AgentConfig {
    /// The sidecar's base URL, for example `http://127.0.0.1:8790`.
    pub url: String,
    /// Shared by Orbit and the sidecar; each sends it to the other.
    pub secret: String,
}

impl std::fmt::Debug for AgentConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentConfig")
            .field("url", &self.url)
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

impl AgentConfig {
    /// `ORBIT_AGENT_URL` with `ORBIT_AGENT_SECRET` (or `ORBIT_AGENT_SECRET_FILE`). `None` when
    /// `ORBIT_AGENT_URL` is not set.
    pub fn from_env() -> Result<Option<Self>, String> {
        let Some(url) = std::env::var("ORBIT_AGENT_URL")
            .ok()
            .filter(|url| !url.trim().is_empty())
        else {
            return Ok(None);
        };
        let secret = match std::env::var("ORBIT_AGENT_SECRET_FILE") {
            Ok(path) => std::fs::read_to_string(&path)
                .map_err(|error| format!("ORBIT_AGENT_SECRET_FILE {path}: {error}"))?,
            Err(_) => std::env::var("ORBIT_AGENT_SECRET").unwrap_or_default(),
        };
        let secret = secret.trim().to_owned();
        if secret.len() < 32 {
            return Err("ORBIT_AGENT_SECRET must have at least 32 characters".to_owned());
        }
        Ok(Some(Self {
            url: url.trim().trim_end_matches('/').to_owned(),
            secret,
        }))
    }
}

/// Everything the agent routes need.
#[derive(Clone)]
pub struct AgentState {
    pub(crate) config: Option<AgentConfig>,
    pub(crate) chat: ChatState,
    pub(crate) identity: Arc<IdentityRepository>,
    pub(crate) cookie_mode: CookieMode,
    /// Seals the ChatGPT credentials; without it nobody can connect.
    pub(crate) app_key: Option<[u8; 32]>,
    pub(crate) http: reqwest::Client,
}

impl AgentState {
    #[must_use]
    pub fn new(
        config: Option<AgentConfig>,
        chat: ChatState,
        identity: Arc<IdentityRepository>,
        cookie_mode: CookieMode,
        app_key: Option<[u8; 32]>,
    ) -> Self {
        Self {
            config,
            chat,
            identity,
            cookie_mode,
            app_key,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(20))
                .build()
                .unwrap_or_default(),
        }
    }

    pub(crate) fn database(&self) -> &orbit_platform::Database {
        self.identity.database()
    }
}
