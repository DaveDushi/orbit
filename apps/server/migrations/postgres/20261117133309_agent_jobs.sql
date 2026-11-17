-- Postgres: the same change as ../20261117133309_agent_jobs.sql.
-- The team agent's jobs (`agent::jobs`): one approval for a whole job instead of one per write. The
-- driver approves a plan once (what, which service, how many calls, the field mapping, a sample);
-- then the job's script runs, and its writes need no new "yes" while they stay inside the job:
-- this driver and thread, this integration (NULL: Orbit Drive) and tool, this method and path,
-- these GraphQL mutations, this exact script (its hash), at most `max_calls` calls, before
-- `expires_at`, and until the driver writes "stop" or the job ends.

CREATE TABLE agent_jobs (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    conversation_id TEXT NOT NULL REFERENCES chat_conversations(id) ON DELETE CASCADE,
    root_message_id TEXT,
    integration_id TEXT REFERENCES agent_integrations(id) ON DELETE CASCADE,
    tool TEXT NOT NULL,
    method TEXT,
    path TEXT,
    mutations TEXT,
    max_calls BIGINT NOT NULL,
    calls BIGINT NOT NULL DEFAULT 0,
    plan TEXT NOT NULL,
    code TEXT NOT NULL,
    script_hash TEXT NOT NULL,
    plan_hash TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    approved_at BIGINT,
    expires_at BIGINT NOT NULL,
    ended_at BIGINT,
    end_reason TEXT CHECK (end_reason IN ('done', 'failed', 'stopped'))
);

CREATE INDEX agent_jobs_thread ON agent_jobs (conversation_id, user_id);
