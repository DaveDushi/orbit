-- Agent profiles (step 6): more agents than the default one. Each agent is a bot user of its own
-- (its name is its @handle, its avatar a user avatar) with a profile in one workspace. Rows of the
-- default agent (`agent_bot`) keep NULL in every `agent_user_id` below, so it works as before.

-- `tools` is a JSON array of tool names (a subset of the default agent's tools); Orbit enforces it
-- for the agent's sessions (`agent::scope`). `model`/`thinking` NULL: the installation's choice.
-- `all_channels` 1: every channel; 0: only `agent_profile_channels`. A deleted agent keeps its row
-- and its bot user (suspended), so its messages and DMs stay readable.
CREATE TABLE agent_profiles (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL UNIQUE REFERENCES users(id),
    instructions TEXT NOT NULL DEFAULT '',
    tools TEXT NOT NULL DEFAULT '[]',
    model TEXT,
    thinking TEXT CHECK (thinking IS NULL OR thinking IN ('minimal', 'low', 'medium', 'high', 'xhigh', 'max')),
    all_channels INTEGER NOT NULL DEFAULT 0 CHECK (all_channels IN (0, 1)),
    created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER
);
CREATE INDEX agent_profiles_workspace ON agent_profiles (workspace_id);

CREATE TABLE agent_profile_channels (
    agent_id TEXT NOT NULL REFERENCES agent_profiles(id) ON DELETE CASCADE,
    conversation_id TEXT NOT NULL REFERENCES chat_conversations(id) ON DELETE CASCADE,
    PRIMARY KEY (agent_id, conversation_id)
);

-- Which agent an input, a thread and an agent session belong to; NULL: the default agent.
-- `refused` on an input: the agent may not be used there (`channel`); Orbit answers it in the
-- author's DM with the agent instead of starting a run.
ALTER TABLE agent_inputs ADD COLUMN agent_user_id TEXT;
ALTER TABLE agent_inputs ADD COLUMN refused TEXT;
ALTER TABLE agent_threads ADD COLUMN agent_user_id TEXT;
ALTER TABLE agent_sessions ADD COLUMN agent_user_id TEXT;

-- Each person's answer level for every agent: how much it thinks and writes.
CREATE TABLE agent_preferences (
    user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    answer_level TEXT NOT NULL CHECK (answer_level IN ('short', 'normal', 'thorough')),
    updated_at INTEGER NOT NULL
);
