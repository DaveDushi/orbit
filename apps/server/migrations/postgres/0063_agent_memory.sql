-- The agent's long-term memory (apps/server/src/agent/memory.rs). Design: .ai/agent-design.md, "Memory".

-- One memory: the team memory of a workspace (`owner_user_id` null) or one person's private memory
-- in it. `size` is the number of memories ever noted: the next position.
CREATE TABLE agent_memory_scopes (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    owner_user_id TEXT REFERENCES users(id) ON DELETE CASCADE,
    size BIGINT NOT NULL DEFAULT 0,
    created_at BIGINT NOT NULL
);

CREATE UNIQUE INDEX agent_memory_scopes_team ON agent_memory_scopes (workspace_id)
    WHERE owner_user_id IS NULL;
CREATE UNIQUE INDEX agent_memory_scopes_private ON agent_memory_scopes (workspace_id, owner_user_id)
    WHERE owner_user_id IS NOT NULL;

-- The log: one line per memory at its position. Rows are never changed, except that a delete
-- erases `body` and sets `deleted_at`, so positions stay valid for the summaries.
CREATE TABLE agent_memories (
    scope_id TEXT NOT NULL REFERENCES agent_memory_scopes(id) ON DELETE CASCADE,
    position BIGINT NOT NULL,
    body TEXT NOT NULL,
    author_id TEXT REFERENCES users(id) ON DELETE SET NULL,
    source_conversation_id TEXT,
    source_root_message_id TEXT,
    created_at BIGINT NOT NULL,
    deleted_at BIGINT,
    deleted_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    PRIMARY KEY (scope_id, position)
);

-- The summary of block [lo, lo + size): a cache, rebuilt from the log. A row without a body is a
-- claim: one run writes it while `claimed_until` has not passed.
CREATE TABLE agent_memory_summaries (
    scope_id TEXT NOT NULL REFERENCES agent_memory_scopes(id) ON DELETE CASCADE,
    lo BIGINT NOT NULL,
    size BIGINT NOT NULL,
    body TEXT,
    claim TEXT,
    claimed_until BIGINT,
    author_id TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at BIGINT NOT NULL,
    PRIMARY KEY (scope_id, lo, size)
);
