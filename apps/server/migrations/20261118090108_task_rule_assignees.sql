-- The assignees that a label rule added (apps/server/src/repositories/label_rules.rs): when GitHub
-- removes the label, the sync removes the person too. A hand-set assignee never has a row.
-- Postgres: postgres/20261118090108_task_rule_assignees.sql.
CREATE TABLE task_rule_assignees (
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    label_id TEXT NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
    PRIMARY KEY (task_id, user_id)
);

-- Earlier rule assignments, from their history rows: only while the label is still on the task,
-- the person is still assigned, and nobody changed the task's assignees by hand since.
INSERT INTO task_rule_assignees (task_id, user_id, label_id)
SELECT audit_events.resource_id, task_assignees.user_id, task_labels.label_id
FROM audit_events
JOIN task_assignees ON task_assignees.task_id = audit_events.resource_id
    AND task_assignees.user_id = json_extract(audit_events.metadata_json, '$.assignee_id')
JOIN task_labels ON task_labels.task_id = audit_events.resource_id
JOIN labels ON labels.id = task_labels.label_id
    AND lower(labels.name) = lower(json_extract(audit_events.metadata_json, '$.label_name'))
WHERE audit_events.action = 'task.assigned_by_rule' AND audit_events.resource_type = 'task'
AND NOT EXISTS (
    SELECT 1 FROM audit_events edits
    WHERE edits.resource_type = 'task' AND edits.resource_id = audit_events.resource_id
    AND edits.action = 'task.updated' AND edits.occurred_at >= audit_events.occurred_at
    AND json_extract(edits.metadata_json, '$.changes.assignees') IS NOT NULL
)
ON CONFLICT DO NOTHING;
