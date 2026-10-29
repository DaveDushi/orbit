-- The team agent's instructions of one workspace (apps/server/src/agent/settings.rs). Owners and
-- admins write them; the sidecar puts them into the system prompt of each run in the workspace.
CREATE TABLE agent_team_instructions (
    workspace_id TEXT PRIMARY KEY REFERENCES workspaces(id) ON DELETE CASCADE,
    body TEXT NOT NULL,
    updated_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    updated_at BIGINT NOT NULL
);
