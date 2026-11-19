-- SQLite. Postgres: postgres/20261119094200_agent_sandbox.sql.
--
-- Agent sandboxes (docs/agent-sandbox-design.md; `agent::sandbox`). Each custom agent may run code in its own
-- gVisor sandbox per chat thread and driver. Orbit stays the judge: it signs a ticket per call (the broker checks it),
-- issues a call token (an agent session bound to the call, deleted at the call's end), and decides each egress host.

-- One row per agent that has sandbox settings. `enabled` 0: no tickets, the proxy denies. Monthly limits per agent;
-- `idle_minutes`: a quiet thread sandbox is deleted after this long (5-120).
CREATE TABLE agent_sandbox_settings (
    agent_user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    enabled INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
    idle_minutes INTEGER NOT NULL DEFAULT 30 CHECK (idle_minutes BETWEEN 5 AND 120),
    cpu_minutes_month INTEGER NOT NULL DEFAULT 600 CHECK (cpu_minutes_month >= 0),
    egress_bytes_month INTEGER NOT NULL DEFAULT 2147483648 CHECK (egress_bytes_month >= 0),
    model_tokens_month INTEGER NOT NULL DEFAULT 20000000 CHECK (model_tokens_month >= 0),
    updated_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    updated_at INTEGER NOT NULL
);

-- One row per sandbox call. `session_id`: the call token's session while the call runs (NULL after its end).
-- `proxy_password_hash`: SHA-256 of the call's proxy password (the password itself is only in the sealed ticket).
-- A call without an end after its timeout + 5 minutes becomes `lost`.
CREATE TABLE agent_sandbox_calls (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    agent_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    driver_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    thread_key TEXT NOT NULL,
    work_id TEXT NOT NULL,
    session_id TEXT,
    proxy_password_hash BLOB NOT NULL CHECK (length(proxy_password_hash) = 32),
    script_sha256 TEXT,
    dry_run INTEGER NOT NULL DEFAULT 0 CHECK (dry_run IN (0, 1)),
    status TEXT NOT NULL CHECK (status IN ('running', 'done', 'lost')),
    timeout_seconds INTEGER NOT NULL,
    started_at INTEGER NOT NULL,
    ended_at INTEGER,
    exit_code INTEGER,
    timed_out INTEGER NOT NULL DEFAULT 0 CHECK (timed_out IN (0, 1)),
    interrupted TEXT,
    cpu_ms INTEGER NOT NULL DEFAULT 0,
    egress_bytes INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX agent_sandbox_calls_agent ON agent_sandbox_calls (agent_user_id, started_at);
CREATE INDEX agent_sandbox_calls_running ON agent_sandbox_calls (started_at) WHERE status = 'running';

-- Internet access rules of an agent: `host` is `api.example.com` or `*.example.com` (subdomains only).
CREATE TABLE agent_egress_rules (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    agent_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    host TEXT NOT NULL,
    port INTEGER NOT NULL DEFAULT 443 CHECK (port BETWEEN 1 AND 65535),
    action TEXT NOT NULL CHECK (action IN ('allow', 'deny')),
    created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at INTEGER NOT NULL,
    last_used_at INTEGER,
    UNIQUE (agent_user_id, host, port)
);

-- Hosts a call wanted and Orbit asked about (`pending`), and the answers: `allowed` (Allow once, for this thread and
-- driver until `expires_at`) or `denied` (for this thread and driver).
CREATE TABLE agent_egress_requests (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    agent_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    driver_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    thread_key TEXT NOT NULL,
    call_id TEXT REFERENCES agent_sandbox_calls(id) ON DELETE SET NULL,
    host TEXT NOT NULL,
    port INTEGER NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'allowed', 'denied')),
    decided_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    decided_at INTEGER,
    expires_at INTEGER,
    created_at INTEGER NOT NULL
);
CREATE INDEX agent_egress_requests_thread ON agent_egress_requests (agent_user_id, thread_key, driver_user_id, host, port);
CREATE INDEX agent_egress_requests_call ON agent_egress_requests (call_id, status);

-- Every egress decision and connection (host and port only: no URLs, paths or bodies). Kept 90 days. The proxy
-- reports an allowed connection's bytes once (`done_at`); `blocked`: allowed by the rules, then refused by the proxy
-- (a non-public address, a TLS name that is not the host).
CREATE TABLE agent_egress_log (
    id TEXT PRIMARY KEY,
    call_id TEXT NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    agent_user_id TEXT NOT NULL,
    driver_user_id TEXT NOT NULL,
    host TEXT NOT NULL,
    port INTEGER NOT NULL,
    decision TEXT NOT NULL CHECK (decision IN ('allow', 'deny', 'ask', 'blocked')),
    reason TEXT NOT NULL,
    rule_id TEXT,
    bytes_up INTEGER NOT NULL DEFAULT 0,
    bytes_down INTEGER NOT NULL DEFAULT 0,
    duration_ms INTEGER NOT NULL DEFAULT 0,
    done_at INTEGER,
    created_at INTEGER NOT NULL
);
CREATE INDEX agent_egress_log_agent ON agent_egress_log (agent_user_id, created_at);

-- Use per agent and calendar month (UTC, `YYYY-MM`), against the monthly limits of the settings.
CREATE TABLE agent_sandbox_usage (
    agent_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    month TEXT NOT NULL,
    cpu_ms INTEGER NOT NULL DEFAULT 0,
    egress_bytes INTEGER NOT NULL DEFAULT 0,
    model_tokens INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (agent_user_id, month)
);

-- A call token is an agent session bound to one sandbox call.
ALTER TABLE agent_sessions ADD COLUMN sandbox_call_id TEXT;
