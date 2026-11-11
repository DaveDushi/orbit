-- Routines, reminders and scheduled messages. A server loop (`apps/server/src/scheduled/`) runs what
-- is due; every time is UTC milliseconds.

-- A recurring job of an agent: on a cron schedule in a time zone, the agent runs `prompt` as its
-- creator (with the creator's permissions) and answers in `conversation_id`. `agent_user_id` is
-- the agent's bot user (one agent today). `status` is `active` or `paused`; `paused_reason` says
-- why Orbit paused it (null when a person did). `next_run_at` is null while paused.
-- `pending_run_id` is set while a claimed run is being posted, so a run that a restart interrupts
-- is finished, never posted twice.
CREATE TABLE agent_routines (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    agent_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    creator_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    conversation_id TEXT REFERENCES chat_conversations(id) ON DELETE SET NULL,
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 80),
    prompt TEXT NOT NULL CHECK (length(prompt) BETWEEN 1 AND 4000),
    schedule TEXT NOT NULL,
    time_zone TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('active', 'paused')),
    paused_reason TEXT,
    next_run_at BIGINT,
    pending_run_id TEXT,
    last_run_at BIGINT,
    last_run_status TEXT CHECK (last_run_status IS NULL OR last_run_status IN ('started', 'skipped')),
    last_run_message_id TEXT,
    last_error TEXT,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    CHECK (status = 'active' OR next_run_at IS NULL)
);
CREATE INDEX agent_routines_due ON agent_routines (next_run_at) WHERE status = 'active';
CREATE INDEX agent_routines_workspace ON agent_routines (workspace_id, creator_id);

-- A routine run reaches the agent as an input with no chat message of its own (like a card
-- action, 0086); the prompt comes from the routine.
ALTER TABLE agent_inputs ADD COLUMN routine_id TEXT;

-- A message that a member schedules (`message`: posted as them in `conversation_id`) or a reminder
-- (`reminder`: posted by the agent in the member's DM with it). A sent message's row is deleted;
-- a send that fails stays as `failed` with `error`, so its author sees it.
CREATE TABLE scheduled_messages (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    author_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('message', 'reminder')),
    conversation_id TEXT REFERENCES chat_conversations(id) ON DELETE CASCADE,
    body TEXT NOT NULL CHECK (length(body) BETWEEN 1 AND 4000),
    send_at BIGINT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('scheduled', 'failed')),
    error TEXT,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    CHECK ((kind = 'message') = (conversation_id IS NOT NULL))
);
CREATE INDEX scheduled_messages_due ON scheduled_messages (send_at) WHERE status = 'scheduled';
CREATE INDEX scheduled_messages_author ON scheduled_messages (workspace_id, author_id, send_at);
