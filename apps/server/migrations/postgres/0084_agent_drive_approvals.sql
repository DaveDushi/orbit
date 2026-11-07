-- The team agent's Drive actions that need the driver's "yes" in the thread: moving to the trash and
-- making a public link. As agent_integration_approvals, but for Orbit's own Drive: one row is one
-- approval of one action (the tool and the hash of its arguments), used at most once.
CREATE TABLE agent_drive_approvals (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    conversation_id TEXT NOT NULL REFERENCES chat_conversations(id) ON DELETE CASCADE,
    root_message_id TEXT,
    tool TEXT NOT NULL,
    arguments_hash TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    used_at BIGINT
);

CREATE INDEX agent_drive_approvals_lookup
    ON agent_drive_approvals (workspace_id, user_id, conversation_id);
