-- "Connect" for remote MCP servers that use OAuth (apps/server/src/agent/mcp_oauth.rs): one shared
-- sign-in per integration. The tokens and the registered client are sealed with the app key in
-- `oauth_sealed_connection`, apart from `sealed_secret`, so an older binary sends no credential.
-- The other columns are what admins see: who is connected, until when, and whether a refresh failed.
ALTER TABLE agent_integrations ADD COLUMN oauth_sealed_connection BYTEA;
ALTER TABLE agent_integrations ADD COLUMN oauth_account TEXT;
ALTER TABLE agent_integrations ADD COLUMN oauth_connected_at BIGINT;
ALTER TABLE agent_integrations ADD COLUMN oauth_grant_expires_at BIGINT;
ALTER TABLE agent_integrations ADD COLUMN oauth_invalid_at BIGINT;
-- When the admins were told that the grant ends soon; a new connection clears it.
ALTER TABLE agent_integrations ADD COLUMN oauth_expiry_notice_at BIGINT;

-- A started Connect: the SHA-256 of `state`, bound to the admin's session, used once, short-lived.
-- `sealed` holds the PKCE verifier and the registered client.
CREATE TABLE agent_integration_oauth_states (
    state_hash TEXT PRIMARY KEY,
    integration_id TEXT NOT NULL REFERENCES agent_integrations(id) ON DELETE CASCADE,
    session_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    sealed BYTEA NOT NULL,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL
);

CREATE INDEX agent_integration_oauth_states_expiry ON agent_integration_oauth_states (expires_at);
