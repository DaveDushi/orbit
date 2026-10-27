-- One instance-wide schedule per job kind, so servers that start at the same time cannot each add one
-- (apps/server/src/app.rs, ensure_schedule). Keeps the oldest row of any duplicates.
DELETE FROM schedules
WHERE workspace_id IS NULL
  AND id NOT IN (SELECT MIN(id) FROM schedules WHERE workspace_id IS NULL GROUP BY job_kind);

CREATE UNIQUE INDEX schedules_instance_kind ON schedules (job_kind) WHERE workspace_id IS NULL;
