-- Postgres: the same change as apps/server/migrations/20261118180902_external_agents.sql
-- External agents (phase 1): an agent profile that an outside client (Claude Code, Codex, a server) runs. It connects
-- to Orbit's MCP as the agent's bot user, pulls its inputs with `wait_for_messages` and answers with MCP tools.
-- `kind`: `internal` (the sidecar runs it) or `external`. `scopes`: the space-separated MCP scopes on its card
-- (`tasks:read` always). `allow_dms` 0: DMs with it are refused. `last_seen_at`: its last MCP call.
ALTER TABLE agent_profiles ADD COLUMN kind TEXT NOT NULL DEFAULT 'internal' CHECK (kind IN ('internal', 'external'));
ALTER TABLE agent_profiles ADD COLUMN scopes TEXT NOT NULL DEFAULT '';
ALTER TABLE agent_profiles ADD COLUMN allow_dms BIGINT NOT NULL DEFAULT 1 CHECK (allow_dms IN (0, 1));
ALTER TABLE agent_profiles ADD COLUMN last_seen_at BIGINT;

-- The projects an external agent may use (its tasks and the docs of their teamspaces).
CREATE TABLE agent_profile_projects (
    agent_id TEXT NOT NULL REFERENCES agent_profiles(id) ON DELETE CASCADE,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    PRIMARY KEY (agent_id, project_id)
);

-- An OAuth connection made "as agent": `user_id` is the agent's bot user, `approved_by` the owner or admin who allowed it.
ALTER TABLE oauth_grants ADD COLUMN agent_profile_id TEXT REFERENCES agent_profiles(id) ON DELETE CASCADE;
ALTER TABLE oauth_grants ADD COLUMN approved_by TEXT REFERENCES users(id) ON DELETE SET NULL;
CREATE INDEX oauth_grants_agent ON oauth_grants(agent_profile_id) WHERE agent_profile_id IS NOT NULL;

-- Agent tokens (`orb_agent_…`): shown once, only the SHA-256 hash is kept.
CREATE TABLE agent_tokens (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL REFERENCES agent_profiles(id) ON DELETE CASCADE,
    token_hash BYTEA NOT NULL UNIQUE CHECK (octet_length(token_hash) = 32),
    token_prefix TEXT NOT NULL,
    created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    last_used_at BIGINT,
    revoked_at BIGINT
);
CREATE INDEX agent_tokens_agent ON agent_tokens(agent_id, created_at);

-- Pull delivery of an external agent's inputs: when `wait_for_messages` last returned it and how often.
ALTER TABLE agent_inputs ADD COLUMN delivered_at BIGINT;
ALTER TABLE agent_inputs ADD COLUMN delivery_attempts BIGINT NOT NULL DEFAULT 0;
