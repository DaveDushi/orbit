-- SQLite. Postgres: postgres/20261114000100_task_doc_links.sql.
--
-- Docs pages linked to tasks ("Add doc" in the task detail, "Linked tasks" on a page). One row
-- per task and page. A hard delete of either side (trash purge, private-space owner deleted,
-- workspace deleted) removes the link; a trashed task or page only hides it until a restore.

CREATE TABLE task_doc_links (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    page_id TEXT NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at INTEGER NOT NULL,
    UNIQUE (task_id, page_id)
);

-- "Linked tasks" of a page, and the ON DELETE CASCADE lookup when a page is deleted.
CREATE INDEX task_doc_links_page ON task_doc_links (page_id);
