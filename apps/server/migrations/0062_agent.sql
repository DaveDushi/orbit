-- The team agent (apps/agent, apps/server/src/agent/). Design: .ai/agent-design.md.

-- The agent's chat identity: one bot user for the installation. It has no password and no
-- sessions; it only writes chat messages that the sidecar asks for.
CREATE TABLE agent_bot (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    user_id TEXT NOT NULL UNIQUE REFERENCES users(id)
);

-- Each person's ChatGPT sign-in: the OAuth credential as JSON, sealed with the app key
-- (secret_box). `invalid_at` is set when a refresh fails, so the person signs in again.
CREATE TABLE agent_credentials (
    user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    sealed BLOB NOT NULL,
    account_id TEXT,
    invalid_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

-- A chat thread the agent works in. `thread_key` is the thread's root message id, or the
-- conversation id of a DM with the agent (its main list is one thread). `driver_user_id` is the
-- person whose run is current or waits for an answer.
CREATE TABLE agent_threads (
    thread_key TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    conversation_id TEXT NOT NULL REFERENCES chat_conversations(id) ON DELETE CASCADE,
    root_message_id TEXT,
    status TEXT NOT NULL CHECK (status IN ('idle', 'running', 'waiting')),
    driver_user_id TEXT REFERENCES users(id),
    waiting_for_user_id TEXT REFERENCES users(id),
    updated_at INTEGER NOT NULL
);

CREATE INDEX agent_threads_workspace ON agent_threads (workspace_id, status);

-- Chat messages for the agent, written in the transaction of the message. The sidecar reads the
-- rows without `acked_at` in id order and acknowledges each one after Pi stored it.
CREATE TABLE agent_inputs (
    message_id TEXT PRIMARY KEY REFERENCES chat_messages(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    thread_key TEXT NOT NULL,
    root_message_id TEXT,
    author_id TEXT NOT NULL REFERENCES users(id),
    kind TEXT NOT NULL CHECK (kind IN ('instruction', 'answer', 'steer')),
    created_at INTEGER NOT NULL,
    acked_at INTEGER
);

CREATE INDEX agent_inputs_pending ON agent_inputs (message_id) WHERE acked_at IS NULL;

-- Sessions that Orbit made for the agent to act as one person in one workspace. Their tokens
-- start with `orbit-agent.`; `agent::scope` limits them to that workspace's work routes.
CREATE TABLE agent_sessions (
    session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    expires_at INTEGER NOT NULL
);

CREATE INDEX agent_sessions_user ON agent_sessions (user_id, workspace_id, expires_at);
