-- SQLite. Postgres: postgres/20261118205820_agent_sandbox_budgets.sql.
--
-- Agent sandbox budgets and off switch (docs/agent-sandbox-design.md, section 5; `agent::sandbox`).

-- 1 when the broker's CPU time of the call went into the month. The broker reports once, also after the sidecar, the
-- off switch or the lost-call job ended the call first: its CPU time still counts, once.
ALTER TABLE agent_sandbox_calls ADD COLUMN cpu_counted INTEGER NOT NULL DEFAULT 0 CHECK (cpu_counted IN (0, 1));

-- Budget notices sent: one per agent, month, counter and level (80 or 100 %), so nobody gets one twice.
CREATE TABLE agent_sandbox_notices (
    agent_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    month TEXT NOT NULL,
    counter TEXT NOT NULL CHECK (counter IN ('cpu', 'egress', 'tokens')),
    level INTEGER NOT NULL CHECK (level IN (80, 100)),
    sent_at INTEGER NOT NULL,
    PRIMARY KEY (agent_user_id, month, counter, level)
);

-- Model tokens of one agent run (the sidecar reports them at the run's end), counted once per run.
CREATE TABLE agent_run_usage (
    run_id TEXT PRIMARY KEY,
    agent_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    month TEXT NOT NULL,
    tokens INTEGER NOT NULL CHECK (tokens >= 0),
    created_at INTEGER NOT NULL
);
CREATE INDEX agent_run_usage_agent ON agent_run_usage (agent_user_id, created_at);

-- Thread sandboxes to stop now (the agent's switch went off, its sandbox tool was unticked, it was deleted). The
-- sidecar takes them with its inputs, asks the broker (`stop_agent`), then deletes the row.
CREATE TABLE agent_sandbox_stops (
    id TEXT PRIMARY KEY,
    agent_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    reason TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
