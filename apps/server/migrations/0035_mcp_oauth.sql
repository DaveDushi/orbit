-- Read-only MCP OAuth grants. Store only hashes of bearer secrets.
CREATE TABLE oauth_clients (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    redirect_uris TEXT NOT NULL CHECK (json_valid(redirect_uris)),
    created_at INTEGER NOT NULL
);
CREATE TABLE oauth_grants (
    id TEXT PRIMARY KEY,
    client_id TEXT NOT NULL REFERENCES oauth_clients(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    project_ids TEXT NOT NULL CHECK (json_valid(project_ids)),
    resource TEXT NOT NULL,
    scope TEXT NOT NULL CHECK (scope = 'tasks:read'),
    revoked_at INTEGER,
    created_at INTEGER NOT NULL
);
CREATE INDEX oauth_grants_user ON oauth_grants(user_id, created_at);
CREATE TABLE oauth_requests (
    id TEXT PRIMARY KEY,
    client_id TEXT NOT NULL REFERENCES oauth_clients(id) ON DELETE CASCADE,
    redirect_uri TEXT NOT NULL,
    state TEXT,
    pkce TEXT NOT NULL,
    resource TEXT NOT NULL,
    user_id TEXT REFERENCES users(id) ON DELETE CASCADE,
    csrf_hash BLOB CHECK (csrf_hash IS NULL OR length(csrf_hash) = 32),
    expires_at INTEGER NOT NULL,
    code_hash BLOB UNIQUE CHECK (code_hash IS NULL OR length(code_hash) = 32),
    grant_id TEXT REFERENCES oauth_grants(id) ON DELETE CASCADE,
    used_at INTEGER
);
CREATE INDEX oauth_requests_expiry ON oauth_requests(expires_at);
CREATE TABLE oauth_tokens (
    access_hash BLOB PRIMARY KEY CHECK (length(access_hash) = 32),
    refresh_hash BLOB NOT NULL UNIQUE CHECK (length(refresh_hash) = 32),
    grant_id TEXT NOT NULL REFERENCES oauth_grants(id) ON DELETE CASCADE,
    access_expires_at INTEGER NOT NULL,
    refresh_expires_at INTEGER NOT NULL,
    refresh_used_at INTEGER
);
CREATE INDEX oauth_tokens_grant ON oauth_tokens(grant_id);
