-- Workspace logos: one picture per workspace, set by its owners and admins in Settings → General.
-- Stored like profile pictures (user_avatars): the web and mobile apps crop it to a square first, the
-- server accepts PNG, JPEG and WebP of at most 512 KiB (checked by magic bytes, SVG is refused), and the
-- image is an attachment blob of the workspace. Removing or replacing the logo deletes the row; attachment
-- reconcile then deletes the unreferenced blob. `updated_at` versions the image URL.
CREATE TABLE workspace_logos (
    workspace_id TEXT PRIMARY KEY REFERENCES workspaces(id) ON DELETE CASCADE,
    mime_type TEXT NOT NULL CHECK (mime_type IN ('image/png', 'image/jpeg', 'image/webp')),
    blob_id TEXT NOT NULL REFERENCES attachment_blobs(id) ON DELETE RESTRICT,
    updated_at INTEGER NOT NULL
);
CREATE INDEX workspace_logos_blob ON workspace_logos (blob_id);
