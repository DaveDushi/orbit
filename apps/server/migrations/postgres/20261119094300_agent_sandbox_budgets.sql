-- Postgres: the same change as apps/server/migrations/20261119094300_agent_sandbox_budgets.sql

ALTER TABLE agent_sandbox_calls ADD COLUMN cpu_counted BIGINT NOT NULL DEFAULT 0 CHECK (cpu_counted IN (0, 1));

CREATE TABLE agent_sandbox_notices (
    agent_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    month TEXT NOT NULL,
    counter TEXT NOT NULL CHECK (counter IN ('cpu', 'egress', 'tokens')),
    level BIGINT NOT NULL CHECK (level IN (80, 100)),
    sent_at BIGINT NOT NULL,
    PRIMARY KEY (agent_user_id, month, counter, level)
);

CREATE TABLE agent_run_usage (
    run_id TEXT PRIMARY KEY,
    agent_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    month TEXT NOT NULL,
    tokens BIGINT NOT NULL CHECK (tokens >= 0),
    created_at BIGINT NOT NULL
);
CREATE INDEX agent_run_usage_agent ON agent_run_usage (agent_user_id, created_at);

CREATE TABLE agent_sandbox_stops (
    id TEXT PRIMARY KEY,
    agent_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    reason TEXT NOT NULL,
    created_at BIGINT NOT NULL
);
