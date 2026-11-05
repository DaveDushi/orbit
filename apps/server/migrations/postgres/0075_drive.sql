-- Orbit Drive v1: one drive per workspace. The bytes live in the workspace's S3 bucket (else the installation's);
-- browsers send and fetch them with presigned URLs, so Orbit only stores names, the tree and object keys here.

-- One object in the bucket. `r2_key` never contains the path, so rename and move change no object.
-- While `status` is 'uploading', the upload_* columns say where the file goes when the upload completes, and
-- `upload_id` is the S3 multipart upload (NULL for a single PUT). 'deleted' rows wait for their object to be
-- deleted from the bucket.
CREATE TABLE drive_blobs (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    r2_key TEXT NOT NULL UNIQUE CHECK (length(r2_key) BETWEEN 1 AND 1024),
    size BIGINT NOT NULL CHECK (size >= 0),
    mime TEXT NOT NULL CHECK (length(mime) BETWEEN 1 AND 255),
    -- sha256 (hex) or a legacy quickXorHash; NULL when unknown.
    content_hash TEXT CHECK (content_hash IS NULL OR length(content_hash) <= 128),
    status TEXT NOT NULL CHECK (status IN ('uploading', 'ready', 'deleted')),
    upload_id TEXT CHECK (upload_id IS NULL OR length(upload_id) <= 1024),
    part_size BIGINT CHECK (part_size IS NULL OR part_size > 0),
    upload_parent_id TEXT,
    upload_name TEXT CHECK (upload_name IS NULL OR length(upload_name) BETWEEN 1 AND 255),
    upload_conflict TEXT CHECK (upload_conflict IS NULL OR upload_conflict IN ('fail', 'rename', 'replace')),
    created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);

CREATE INDEX drive_blobs_status ON drive_blobs (status, updated_at);
CREATE INDEX drive_blobs_workspace ON drive_blobs (workspace_id, status);

-- A folder or file. `parent_id` NULL is the drive's top level. Only the top node of a trashed tree has
-- `trashed_at`; its descendants are hidden with it and come back with it.
CREATE TABLE drive_nodes (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    parent_id TEXT REFERENCES drive_nodes(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('folder', 'file')),
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 255),
    name_lower TEXT NOT NULL,
    blob_id TEXT REFERENCES drive_blobs(id) ON DELETE RESTRICT,
    size BIGINT CHECK (size IS NULL OR size >= 0),
    mime TEXT,
    created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at BIGINT NOT NULL,
    updated_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    updated_at BIGINT NOT NULL,
    trashed_at BIGINT,
    trashed_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    source_kind TEXT NOT NULL DEFAULT 'upload' CHECK (source_kind IN ('upload', 'wellrox_cloud')),
    -- The node's id in the system it was imported from; a second import skips it.
    source_ref TEXT CHECK (source_ref IS NULL OR length(source_ref) <= 255),
    source_created_at BIGINT,
    source_modified_at BIGINT,
    CHECK ((kind = 'file') = (blob_id IS NOT NULL)),
    CHECK (parent_id IS NULL OR parent_id <> id)
);

CREATE UNIQUE INDEX drive_nodes_live_name ON drive_nodes (workspace_id, COALESCE(parent_id, ''), name_lower)
    WHERE trashed_at IS NULL;
CREATE INDEX drive_nodes_children ON drive_nodes (workspace_id, parent_id, trashed_at, name_lower);
CREATE INDEX drive_nodes_updated ON drive_nodes (workspace_id, updated_at);
CREATE INDEX drive_nodes_mime ON drive_nodes (workspace_id, mime);
CREATE INDEX drive_nodes_trash ON drive_nodes (workspace_id, trashed_at) WHERE trashed_at IS NOT NULL;
CREATE INDEX drive_nodes_parent ON drive_nodes (parent_id);
CREATE INDEX drive_nodes_blob ON drive_nodes (blob_id);
CREATE UNIQUE INDEX drive_nodes_source ON drive_nodes (workspace_id, source_kind, source_ref);

-- What the browser measured for an image or video, and its small preview objects.
CREATE TABLE drive_media (
    blob_id TEXT PRIMARY KEY REFERENCES drive_blobs(id) ON DELETE CASCADE,
    width BIGINT CHECK (width IS NULL OR width > 0),
    height BIGINT CHECK (height IS NULL OR height > 0),
    duration_ms BIGINT CHECK (duration_ms IS NULL OR duration_ms >= 0),
    codec TEXT CHECK (codec IS NULL OR length(codec) <= 100),
    taken_at BIGINT,
    thumb_key TEXT CHECK (thumb_key IS NULL OR length(thumb_key) <= 1024),
    poster_key TEXT CHECK (poster_key IS NULL OR length(poster_key) <= 1024),
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'ready', 'failed')),
    updated_at BIGINT NOT NULL
);

-- A public link to one file or folder. Only the token's hash is stored.
CREATE TABLE drive_shares (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    node_id TEXT NOT NULL REFERENCES drive_nodes(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at BIGINT NOT NULL,
    expires_at BIGINT,
    password_hash TEXT,
    download_count BIGINT NOT NULL DEFAULT 0 CHECK (download_count >= 0),
    revoked_at BIGINT
);

CREATE INDEX drive_shares_node ON drive_shares (node_id);
CREATE INDEX drive_shares_workspace ON drive_shares (workspace_id, created_at);
