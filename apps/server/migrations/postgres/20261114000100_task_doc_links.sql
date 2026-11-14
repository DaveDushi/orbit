-- Postgres: the same change as ../20261114000100_task_doc_links.sql.

CREATE TABLE task_doc_links (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    page_id TEXT NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at BIGINT NOT NULL,
    UNIQUE (task_id, page_id)
);

CREATE INDEX task_doc_links_page ON task_doc_links (page_id);
