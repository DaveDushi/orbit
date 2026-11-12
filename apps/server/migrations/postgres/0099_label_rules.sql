-- Label rules: "if a task gets label X, assign person Y" (only adds assignees). A rule goes with
-- its label or its person. SQLite: ../0099_label_rules.sql.
CREATE TABLE label_rules (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    label_id TEXT NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    enabled BIGINT NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    UNIQUE (label_id, user_id)
);

CREATE INDEX label_rules_project ON label_rules (project_id);

-- The project's "owner:<name> labels assign <name>" switch (off without a row).
CREATE TABLE project_label_rule_settings (
    project_id TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    owner_labels BIGINT NOT NULL DEFAULT 0 CHECK (owner_labels IN (0, 1)),
    updated_at BIGINT NOT NULL
);
