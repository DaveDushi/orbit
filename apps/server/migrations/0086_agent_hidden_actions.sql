-- A click or form submit on the agent's card no longer posts a chat message: it is queued in
-- agent_inputs alone, under an id of its own. So `message_id` stops referencing chat_messages.
-- Rows from 0085 keep the id of the message that reported the click.
--
-- SQLite cannot drop a foreign key, so agent_inputs is rebuilt as in 0085.
PRAGMA defer_foreign_keys = ON;

CREATE TABLE agent_inputs_backup AS SELECT * FROM agent_inputs;
DROP TABLE agent_inputs;

CREATE TABLE agent_inputs (
    message_id TEXT PRIMARY KEY,
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

INSERT INTO agent_inputs SELECT * FROM agent_inputs_backup;

DROP TABLE agent_inputs_backup;

CREATE INDEX agent_inputs_pending ON agent_inputs (message_id) WHERE acked_at IS NULL;
CREATE INDEX agent_inputs_card ON agent_inputs (card_message_id, created_at)
    WHERE card_message_id IS NOT NULL;
