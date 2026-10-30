-- See the SQLite migration of the same number.
ALTER TABLE page_files ADD COLUMN version BIGINT NOT NULL DEFAULT 1;
ALTER TABLE page_files ADD COLUMN updated_by TEXT REFERENCES users(id) ON DELETE SET NULL;
ALTER TABLE page_files ADD COLUMN updated_at BIGINT;

CREATE TABLE page_file_versions (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    page_file_id TEXT NOT NULL REFERENCES page_files(id) ON DELETE CASCADE,
    version BIGINT NOT NULL,
    blob_id TEXT NOT NULL REFERENCES attachment_blobs(id) ON DELETE RESTRICT,
    file_name TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    size_bytes BIGINT NOT NULL CHECK (size_bytes >= 0),
    created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at BIGINT NOT NULL,
    UNIQUE (page_file_id, version)
);

CREATE INDEX page_file_versions_blob ON page_file_versions (blob_id);
CREATE INDEX page_file_versions_workspace ON page_file_versions (workspace_id);
CREATE INDEX page_file_versions_created_by ON page_file_versions (created_by);

CREATE FUNCTION page_file_versions_scope_insert_fn() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM page_files
        WHERE page_files.id = NEW.page_file_id AND page_files.workspace_id = NEW.workspace_id
    ) OR NOT EXISTS (
        SELECT 1 FROM attachment_blobs
        WHERE attachment_blobs.id = NEW.blob_id AND attachment_blobs.workspace_id = NEW.workspace_id
    ) THEN
        RAISE EXCEPTION 'page file version scope mismatch';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER page_file_versions_scope_insert
    BEFORE INSERT ON page_file_versions
    FOR EACH ROW
    EXECUTE FUNCTION page_file_versions_scope_insert_fn();

CREATE FUNCTION page_file_versions_quarantine_blob_fn() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    UPDATE attachment_blobs
    SET quarantine_until = GREATEST(
        quarantine_until,
        floor(EXTRACT(EPOCH FROM clock_timestamp()))::BIGINT * 1000 + 86400000
    )
    WHERE id = OLD.blob_id;
    RETURN NULL;
END;
$$;
CREATE TRIGGER page_file_versions_quarantine_blob
    AFTER DELETE ON page_file_versions
    FOR EACH ROW
    EXECUTE FUNCTION page_file_versions_quarantine_blob_fn();

DROP TRIGGER page_files_scope_immutable ON page_files;
CREATE TRIGGER page_files_scope_immutable
    BEFORE UPDATE OF workspace_id, page_id ON page_files
    FOR EACH ROW
    WHEN (NEW.workspace_id <> OLD.workspace_id OR NEW.page_id <> OLD.page_id)
    EXECUTE FUNCTION page_files_scope_immutable_fn();

CREATE FUNCTION page_files_blob_scope_fn() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM attachment_blobs
        WHERE attachment_blobs.id = NEW.blob_id AND attachment_blobs.workspace_id = NEW.workspace_id
    ) THEN
        RAISE EXCEPTION 'page file scope mismatch';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER page_files_blob_scope
    BEFORE UPDATE OF blob_id ON page_files
    FOR EACH ROW
    EXECUTE FUNCTION page_files_blob_scope_fn();
