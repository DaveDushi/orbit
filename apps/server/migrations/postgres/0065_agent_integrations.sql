-- Tools outside Orbit for the team agent (apps/server/src/agent/integrations.rs). Design:
-- .ai/agent-design.md, "Integrations".

-- One remote MCP server or HTTP API of a workspace. Owners and admins set it up; the one shared
-- credential (`auth_header`: `sealed_secret`) is sealed with the app key and never leaves Orbit.
CREATE TABLE agent_integrations (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    -- The prefix of the agent's tool names: `<slug>__<tool>`.
    slug TEXT NOT NULL CHECK (length(slug) BETWEEN 1 AND 32),
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 80),
    kind TEXT NOT NULL CHECK (kind IN ('mcp', 'http')),
    -- MCP: the Streamable HTTP endpoint. HTTP: the base URL every request stays below.
    url TEXT NOT NULL CHECK (length(url) <= 2048),
    auth_header TEXT,
    sealed_secret BYTEA,
    -- JSON arrays: the workspace roles that may use it, the MCP tools that are on, the HTTP
    -- methods that are allowed.
    allowed_roles TEXT NOT NULL,
    enabled_tools TEXT NOT NULL DEFAULT '[]',
    allowed_methods TEXT NOT NULL DEFAULT '["GET"]',
    -- MCP: the server's tools as `tools/list` returned them at the last test (JSON).
    tools TEXT,
    tools_checked_at BIGINT,
    created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    UNIQUE (workspace_id, slug)
);

-- A write the agent asked the driver to allow. It is used once, only for the same driver, thread,
-- integration, tool and arguments, and only after the driver answered "yes" in the thread.
CREATE TABLE agent_integration_approvals (
    id TEXT PRIMARY KEY,
    integration_id TEXT NOT NULL REFERENCES agent_integrations(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    conversation_id TEXT NOT NULL REFERENCES chat_conversations(id) ON DELETE CASCADE,
    root_message_id TEXT,
    tool TEXT NOT NULL,
    arguments_hash TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    used_at BIGINT
);

CREATE INDEX agent_integration_approvals_lookup
    ON agent_integration_approvals (integration_id, user_id, conversation_id);
