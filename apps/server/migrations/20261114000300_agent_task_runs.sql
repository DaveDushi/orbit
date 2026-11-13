-- SQLite. Postgres: postgres/20261114000300_agent_task_runs.sql.
--
-- Agent runs on tasks (`agent::task_runs`). A task assigned to an agent, a comment that mentions
-- an agent, or a person's comment on a task an agent is assigned to queues a run here, in the
-- transaction of that write. The driver is that person. Orbit starts a queued run when no other
-- run of its task is running (one at a time per task); `id` is then also its `agent_inputs`
-- row, and `thread_key` its agent thread. Unassigning the agent or deleting the task cancels it;
-- `stopped_at` is set when the sidecar has stopped a cancelled run that had started.

CREATE TABLE agent_task_runs (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    agent_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    driver_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    cause TEXT NOT NULL CHECK (cause IN ('assigned', 'mentioned', 'commented')),
    comment_id TEXT REFERENCES task_comments(id) ON DELETE SET NULL,
    status TEXT NOT NULL CHECK (status IN ('queued', 'running', 'done', 'cancelled')),
    reason TEXT,
    conversation_id TEXT,
    thread_key TEXT,
    result_comment_id TEXT REFERENCES task_comments(id) ON DELETE SET NULL,
    created_at INTEGER NOT NULL,
    started_at INTEGER,
    finished_at INTEGER,
    stopped_at INTEGER
);

-- One running run per task, on every server.
CREATE UNIQUE INDEX agent_task_runs_one_running ON agent_task_runs (task_id) WHERE status = 'running';
CREATE INDEX agent_task_runs_task ON agent_task_runs (task_id, status);
CREATE INDEX agent_task_runs_queued ON agent_task_runs (created_at) WHERE status = 'queued';
CREATE INDEX agent_task_runs_thread ON agent_task_runs (thread_key);
