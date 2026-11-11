-- Code mode, saved scripts and routine tests (step 12). Times are UTC milliseconds.

-- A script that an agent saved for a person (Settings → Agentic → Routines): JavaScript that the agent's
-- code mode runs in a sandbox, calling only the agent's own tools. `code_hash` is the SHA-256 (hex) of `code`:
-- a routine's write approval is bound to it.
CREATE TABLE agent_scripts (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    owner_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 80),
    description TEXT NOT NULL DEFAULT '' CHECK (length(description) <= 500),
    code TEXT NOT NULL CHECK (length(code) BETWEEN 1 AND 20000),
    code_hash TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX agent_scripts_name ON agent_scripts (workspace_id, owner_id, lower(name));

-- A routine can run a saved script on each run. Its writes need the creator's yes to that exact script:
-- `script_approved_hash` is the hash they approved; while it differs from the script's hash, runs make no
-- writes. `script_state` keeps the script's `store()` values (JSON) from run to run.
ALTER TABLE agent_routines ADD COLUMN script_id TEXT REFERENCES agent_scripts(id) ON DELETE SET NULL;
ALTER TABLE agent_routines ADD COLUMN script_approved_hash TEXT;
ALTER TABLE agent_routines ADD COLUMN script_state TEXT;

-- One run of a routine (the last 20 per routine): scheduled runs and test runs (`test` = 1, in the creator's DM,
-- with no writes). `id` is the agent input of the run; `message_id` the run's first message (the root of its
-- thread, `thread_key` the agent's thread). `writes`: `normal` (an instruction), `script` (an approved script:
-- its approval-gated writes need no new yes) or `dry` (a test run, or a script changed since its approval:
-- nothing is written, the run says what it would do). The sidecar reports `output`, `error` and `tool_calls`
-- (a JSON list) at the end.
CREATE TABLE agent_routine_runs (
    id TEXT PRIMARY KEY,
    routine_id TEXT NOT NULL REFERENCES agent_routines(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    test INTEGER NOT NULL DEFAULT 0 CHECK (test IN (0, 1)),
    status TEXT NOT NULL CHECK (status IN ('started', 'skipped', 'done', 'failed')),
    writes TEXT NOT NULL CHECK (writes IN ('normal', 'script', 'dry')),
    conversation_id TEXT,
    message_id TEXT,
    thread_key TEXT,
    script_id TEXT,
    script_name TEXT,
    script_hash TEXT,
    script_code TEXT,
    started_at INTEGER NOT NULL,
    finished_at INTEGER,
    output TEXT,
    error TEXT,
    tool_calls TEXT
);
CREATE INDEX agent_routine_runs_routine ON agent_routine_runs (routine_id, started_at);
CREATE INDEX agent_routine_runs_thread ON agent_routine_runs (thread_key);
CREATE INDEX agent_routine_runs_root ON agent_routine_runs (conversation_id, message_id);

-- An agent session of a dry run: Orbit refuses its writes.
ALTER TABLE agent_sessions ADD COLUMN dry_run INTEGER NOT NULL DEFAULT 0;
