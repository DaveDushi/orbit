-- Docs page files can change: `page_files` keeps the current version, and every save moves the
-- version it replaces to `page_file_versions`, so a person can restore it. A version holds its blob
-- like a page file does (see BLOB_REFERENCE_COUNT), so reconciliation does not reclaim it.
ALTER TABLE page_files ADD COLUMN version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE page_files ADD COLUMN updated_by TEXT REFERENCES users(id) ON DELETE SET NULL;
ALTER TABLE page_files ADD COLUMN updated_at INTEGER;

CREATE TABLE page_file_versions (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    page_file_id TEXT NOT NULL REFERENCES page_files(id) ON DELETE CASCADE,
    version INTEGER NOT NULL,
    blob_id TEXT NOT NULL REFERENCES attachment_blobs(id) ON DELETE RESTRICT,
    file_name TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    -- Who made this version, and when.
    created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at INTEGER NOT NULL,
    UNIQUE (page_file_id, version)
);

CREATE INDEX page_file_versions_blob ON page_file_versions (blob_id);
CREATE INDEX page_file_versions_workspace ON page_file_versions (workspace_id);
CREATE INDEX page_file_versions_created_by ON page_file_versions (created_by);

CREATE TRIGGER page_file_versions_scope_insert
BEFORE INSERT ON page_file_versions
WHEN NOT EXISTS (
    SELECT 1 FROM page_files
    WHERE page_files.id = NEW.page_file_id AND page_files.workspace_id = NEW.workspace_id
) OR NOT EXISTS (
    SELECT 1 FROM attachment_blobs
    WHERE attachment_blobs.id = NEW.blob_id AND attachment_blobs.workspace_id = NEW.workspace_id
)
BEGIN
    SELECT RAISE(ABORT, 'page file version scope mismatch');
END;

-- Fires for the cascade from page_files too.
CREATE TRIGGER page_file_versions_quarantine_blob
AFTER DELETE ON page_file_versions
BEGIN
    UPDATE attachment_blobs
    SET quarantine_until = MAX(
        quarantine_until,
        CAST(strftime('%s', 'now') AS INTEGER) * 1000 + 86400000
    )
    WHERE id = OLD.blob_id;
END;

-- A page file can now change its blob, but only to a blob of its workspace.
DROP TRIGGER page_files_scope_immutable;

CREATE TRIGGER page_files_scope_immutable
BEFORE UPDATE OF workspace_id, page_id ON page_files
WHEN NEW.workspace_id <> OLD.workspace_id OR NEW.page_id <> OLD.page_id
BEGIN
    SELECT RAISE(ABORT, 'page file scope is immutable');
END;

CREATE TRIGGER page_files_blob_scope
BEFORE UPDATE OF blob_id ON page_files
WHEN NOT EXISTS (
    SELECT 1 FROM attachment_blobs
    WHERE attachment_blobs.id = NEW.blob_id AND attachment_blobs.workspace_id = NEW.workspace_id
)
BEGIN
    SELECT RAISE(ABORT, 'page file scope mismatch');
END;
