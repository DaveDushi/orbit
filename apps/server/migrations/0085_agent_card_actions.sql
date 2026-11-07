-- A click or form submit on a card that the team agent posted (a `ui` block) is queued for the
-- agent as kind 'action'. `card_message_id` is the message with the card (for the per-card limit)
-- and `action` the JSON that the sidecar reads: what was clicked, from the stored card, and the
-- form values.
--
-- SQLite cannot alter a CHECK constraint, so agent_inputs is rebuilt as in 0021. Nothing
-- references agent_inputs, and it has no triggers; its one index is recreated below.
PRAGMA defer_foreign_keys = ON;

CREATE TABLE agent_inputs_backup AS SELECT * FROM agent_inputs;
DROP TABLE agent_inputs;

CREATE TABLE agent_inputs (
    message_id TEXT PRIMARY KEY REFERENCES chat_messages(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    thread_key TEXT NOT NULL,
    root_message_id TEXT,
    author_id TEXT NOT NULL REFERENCES users(id),
    kind TEXT NOT NULL CHECK (kind IN ('instruction', 'answer', 'steer', 'action')),
    created_at INTEGER NOT NULL,
    acked_at INTEGER,
    card_message_id TEXT,
    action TEXT
);

INSERT INTO agent_inputs (message_id, workspace_id, conversation_id, thread_key, root_message_id,
                          author_id, kind, created_at, acked_at)
SELECT message_id, workspace_id, conversation_id, thread_key, root_message_id,
       author_id, kind, created_at, acked_at
FROM agent_inputs_backup;

DROP TABLE agent_inputs_backup;

CREATE INDEX agent_inputs_pending ON agent_inputs (message_id) WHERE acked_at IS NULL;
CREATE INDEX agent_inputs_card ON agent_inputs (card_message_id, created_at)
    WHERE card_message_id IS NOT NULL;
