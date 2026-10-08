-- Instance-wide settings that the root user changes in Admin: open registration and the outgoing mail server.
-- One row. Registration is closed by default, so after setup only invitations add accounts.
CREATE TABLE instance_settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    registration_open INTEGER NOT NULL DEFAULT 0 CHECK (registration_open IN (0, 1)),
    smtp_host TEXT CHECK (smtp_host IS NULL OR length(smtp_host) BETWEEN 1 AND 253),
    smtp_port INTEGER CHECK (smtp_port IS NULL OR smtp_port BETWEEN 1 AND 65535),
    smtp_security TEXT CHECK (smtp_security IS NULL OR smtp_security IN ('tls', 'starttls', 'none')),
    smtp_username TEXT CHECK (smtp_username IS NULL OR length(smtp_username) BETWEEN 1 AND 320),
    -- ChaCha20-Poly1305 under the server app key (secret_box.rs), never plain text.
    smtp_password BLOB,
    smtp_from_address TEXT CHECK (smtp_from_address IS NULL OR length(smtp_from_address) BETWEEN 3 AND 320),
    smtp_from_name TEXT CHECK (smtp_from_name IS NULL OR length(smtp_from_name) BETWEEN 1 AND 120),
    updated_at INTEGER NOT NULL,
    -- SMTP is configured exactly when host, port, security and sender are all set.
    CHECK (
        (smtp_host IS NULL AND smtp_port IS NULL AND smtp_security IS NULL AND smtp_from_address IS NULL)
        OR (smtp_host IS NOT NULL AND smtp_port IS NOT NULL AND smtp_security IS NOT NULL
            AND smtp_from_address IS NOT NULL)
    )
);

INSERT INTO instance_settings (id, updated_at) VALUES (1, 0);

-- Open registration: an emailed one-time link proves the address before the account exists.
CREATE TABLE registration_tokens (
    token_hash BLOB PRIMARY KEY CHECK (length(token_hash) = 32),
    normalized_email TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX registration_tokens_expiry ON registration_tokens (expires_at);
CREATE INDEX registration_tokens_email ON registration_tokens (normalized_email);
