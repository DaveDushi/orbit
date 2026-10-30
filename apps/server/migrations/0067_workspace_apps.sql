-- Custom web apps of a workspace, opened from Orbit with one login (apps/server/src/apps/). Orbit
-- does not host them. Design: .ai/apps-design.md.

CREATE TABLE workspace_apps (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    -- The path of its page: /apps/<slug>.
    slug TEXT NOT NULL CHECK (length(slug) BETWEEN 1 AND 40),
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 80),
    icon TEXT NOT NULL,
    url TEXT NOT NULL CHECK (length(url) <= 2048),
    -- JSON array: the workspace roles that may list, open and sign in to the app.
    allowed_roles TEXT NOT NULL,
    open_mode TEXT NOT NULL CHECK (open_mode IN ('embed', 'new_tab')),
    -- "Sign in with Orbit": all NULL / '[]' until an admin registers redirect URIs. The secret is
    -- 256 random bits, so a plain SHA-256 is enough.
    oidc_client_id TEXT UNIQUE,
    oidc_secret_hash BLOB,
    oidc_redirect_uris TEXT NOT NULL DEFAULT '[]',
    created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (workspace_id, slug)
);

-- Kept apart from oauth_requests and oauth_tokens: MCP access reads only those tables, so a token
-- of an app can never reach the API or /mcp, and an MCP token can never reach userinfo.
CREATE TABLE app_oidc_codes (
    code_hash BLOB PRIMARY KEY,
    app_id TEXT NOT NULL REFERENCES workspace_apps(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    redirect_uri TEXT NOT NULL,
    scope TEXT NOT NULL,
    nonce TEXT,
    code_challenge TEXT,
    expires_at INTEGER NOT NULL,
    used_at INTEGER
);

CREATE TABLE app_oidc_tokens (
    access_hash BLOB PRIMARY KEY,
    app_id TEXT NOT NULL REFERENCES workspace_apps(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    -- The code that made the token: a second use of the code deletes the token.
    code_hash BLOB NOT NULL,
    scope TEXT NOT NULL,
    expires_at INTEGER NOT NULL
);

CREATE INDEX app_oidc_tokens_user ON app_oidc_tokens (user_id, app_id);
CREATE INDEX app_oidc_codes_user ON app_oidc_codes (user_id, app_id);

-- The one RS256 key of the instance. The first server that needs it makes it; the others read it.
CREATE TABLE oidc_signing_keys (
    slot TEXT PRIMARY KEY CHECK (slot = 'primary'),
    kid TEXT NOT NULL,
    sealed_key BLOB NOT NULL,
    public_jwk TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
