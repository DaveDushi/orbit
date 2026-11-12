-- GitHub label sync (one way, GitHub → Orbit). A row means the GitHub sync added this label to
-- the task, so the sync removes it again when GitHub drops it. Labels people added in Orbit have
-- no row and stay. Postgres: postgres/0097_github_task_labels.sql.
CREATE TABLE github_task_labels (
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    label_id TEXT NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
    PRIMARY KEY (task_id, label_id)
);

CREATE INDEX github_task_labels_label ON github_task_labels (label_id);
