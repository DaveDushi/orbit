-- Postgres: the same change as apps/server/migrations/20261113190031_workspace_logos.sql
CREATE TABLE workspace_logos (
    workspace_id TEXT PRIMARY KEY REFERENCES workspaces(id) ON DELETE CASCADE,
    mime_type TEXT NOT NULL CHECK (mime_type IN ('image/png', 'image/jpeg', 'image/webp')),
    blob_id TEXT NOT NULL REFERENCES attachment_blobs(id) ON DELETE RESTRICT,
    updated_at BIGINT NOT NULL
);
CREATE INDEX workspace_logos_blob ON workspace_logos (blob_id);
