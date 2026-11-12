-- Chat media, big chat files in Drive, and file previews (docs/file-previews.md).

-- 1. Drive folders of chat ("Uploads/#sales", a direct message's folder). `chat_conversation_id` is the
-- conversation whose readers see the node: set on the conversation's folder (`chat_root` = 1) and copied to
-- everything below it, also when a node moves in or out. NULL: every member of the workspace sees it.
ALTER TABLE drive_nodes ADD COLUMN chat_conversation_id TEXT;
ALTER TABLE drive_nodes ADD COLUMN chat_root INTEGER NOT NULL DEFAULT 0 CHECK (chat_root IN (0, 1));
CREATE INDEX drive_nodes_chat ON drive_nodes (workspace_id, chat_conversation_id, chat_root);

-- 2. A file of a chat message that is too large for chat and is kept in Drive. As with chat_message_files, it
-- is its uploader's alone until a send attaches it.
CREATE TABLE chat_drive_files (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    -- Both NULL until the message is sent.
    message_id TEXT REFERENCES chat_messages(id) ON DELETE CASCADE,
    conversation_id TEXT REFERENCES chat_conversations(id) ON DELETE CASCADE,
    -- NULL when the Drive file was deleted forever: the message shows "Deleted file".
    node_id TEXT REFERENCES drive_nodes(id) ON DELETE SET NULL,
    file_name TEXT NOT NULL CHECK (length(file_name) BETWEEN 1 AND 255),
    mime_type TEXT NOT NULL,
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    uploaded_by TEXT NOT NULL REFERENCES users(id),
    created_at INTEGER NOT NULL,
    CHECK ((message_id IS NULL) = (conversation_id IS NULL))
);

CREATE INDEX chat_drive_files_message ON chat_drive_files (message_id);
CREATE INDEX chat_drive_files_node ON chat_drive_files (node_id);
CREATE INDEX chat_drive_files_pending ON chat_drive_files (created_at) WHERE message_id IS NULL;

-- 3. The preview of one stored file version: a playable MP4 and a poster of a video, a PDF of an office file. The
-- source is a stored blob (chat files) or a Drive blob; a new version is a new blob, so each version has its own.
CREATE TABLE file_previews (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    source_blob_id TEXT REFERENCES attachment_blobs(id) ON DELETE CASCADE,
    drive_blob_id TEXT REFERENCES drive_blobs(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('video', 'document')),
    -- 'skipped': too large for this server, or the converter is not installed.
    status TEXT NOT NULL CHECK (status IN ('pending', 'ready', 'failed', 'skipped')),
    detail TEXT CHECK (detail IS NULL OR length(detail) <= 500),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK ((source_blob_id IS NULL) <> (drive_blob_id IS NULL)),
    UNIQUE (source_blob_id, kind),
    UNIQUE (drive_blob_id, kind)
);

CREATE INDEX file_previews_workspace ON file_previews (workspace_id);

-- The files a preview made: a stored blob (for a chat file) or an object in the Drive bucket (for a Drive file).
CREATE TABLE file_preview_outputs (
    preview_id TEXT NOT NULL REFERENCES file_previews(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('play', 'poster', 'pdf')),
    blob_id TEXT REFERENCES attachment_blobs(id) ON DELETE CASCADE,
    object_key TEXT CHECK (object_key IS NULL OR length(object_key) <= 1024),
    mime_type TEXT NOT NULL,
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    PRIMARY KEY (preview_id, role),
    CHECK ((blob_id IS NULL) <> (object_key IS NULL))
);

CREATE INDEX file_preview_outputs_blob ON file_preview_outputs (blob_id);
