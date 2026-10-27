-- Sign-ins that wait for a second factor and passkey ceremonies in progress, so the next request can go
-- to any server (apps/server/src/auth_routes/two_factor.rs, PendingStore). The token is kept as its
-- SHA-256 hash.
CREATE TABLE auth_pending (
    token_hash TEXT PRIMARY KEY,
    expires_at BIGINT NOT NULL,
    payload_json TEXT NOT NULL
);

CREATE INDEX auth_pending_expires ON auth_pending (expires_at);
