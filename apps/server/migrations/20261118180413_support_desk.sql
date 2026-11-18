-- Support mailboxes (helpdesk phase 1, docs/support.md): a ticket is a task in a project with a
-- connected mailbox. New tables only. Postgres: postgres/20261118180413_support_desk.sql.

-- One mailbox per project. The inbound URL holds a secret; only its SHA-256 is stored. The SMTP
-- password is encrypted with the app key (secret_box), like the instance SMTP password. The status
-- ids are not foreign keys: a deleted status falls back to the project's default status.
CREATE TABLE support_mailboxes (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    project_id TEXT NOT NULL UNIQUE REFERENCES projects(id) ON DELETE CASCADE,
    address TEXT NOT NULL CHECK (length(address) BETWEEN 3 AND 320),
    display_name TEXT NOT NULL CHECK (length(display_name) <= 80),
    secret_hash BLOB NOT NULL CHECK (length(secret_hash) = 32),
    smtp_host TEXT,
    smtp_port INTEGER,
    smtp_security TEXT CHECK (smtp_security IS NULL OR smtp_security IN ('tls', 'starttls', 'none')),
    smtp_username TEXT,
    smtp_password BLOB,
    open_status_id TEXT NOT NULL,
    -- NULL: a reply does not move the ticket.
    pending_status_id TEXT,
    default_assignee_id TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_by TEXT NOT NULL REFERENCES users(id),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    rotated_at INTEGER,
    last_received_at INTEGER
);

CREATE INDEX support_mailboxes_workspace ON support_mailboxes (workspace_id, project_id);

-- Customers. They are not Orbit users and never sign in. `email` is lowercase.
CREATE TABLE support_contacts (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    email TEXT NOT NULL CHECK (length(email) BETWEEN 3 AND 320),
    name TEXT NOT NULL DEFAULT '' CHECK (length(name) <= 200),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (workspace_id, email)
);

-- The task of a ticket, its mailbox and its customer. A deleted mailbox keeps its tickets.
CREATE TABLE support_tickets (
    task_id TEXT PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    mailbox_id TEXT REFERENCES support_mailboxes(id) ON DELETE SET NULL,
    contact_id TEXT NOT NULL REFERENCES support_contacts(id) ON DELETE CASCADE,
    subject TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX support_tickets_contact ON support_tickets (contact_id, created_at);
CREATE INDEX support_tickets_mailbox ON support_tickets (mailbox_id, task_id);

-- Emails of a ticket: `in` from the customer, `out` a reply of a member. `body_html` is already
-- sanitized (ammonia). `attachments_json` lists its task attachments (attachment_references).
CREATE TABLE support_messages (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    mailbox_id TEXT REFERENCES support_mailboxes(id) ON DELETE SET NULL,
    direction TEXT NOT NULL CHECK (direction IN ('in', 'out')),
    author_id TEXT REFERENCES users(id) ON DELETE SET NULL,
    from_address TEXT NOT NULL,
    from_name TEXT NOT NULL DEFAULT '',
    to_addresses TEXT NOT NULL DEFAULT '',
    cc_addresses TEXT NOT NULL DEFAULT '',
    subject TEXT NOT NULL DEFAULT '',
    message_id TEXT NOT NULL,
    in_reply_to TEXT,
    references_header TEXT NOT NULL DEFAULT '',
    body_text TEXT NOT NULL DEFAULT '',
    body_html TEXT,
    attachments_json TEXT NOT NULL DEFAULT '{}',
    unverified INTEGER NOT NULL DEFAULT 0 CHECK (unverified IN (0, 1)),
    send_state TEXT CHECK (send_state IS NULL OR send_state IN ('queued', 'sent', 'failed')),
    send_error TEXT,
    sent_at INTEGER,
    created_at INTEGER NOT NULL,
    CHECK ((direction = 'out') = (send_state IS NOT NULL))
);

CREATE INDEX support_messages_task ON support_messages (task_id, created_at, id);
CREATE INDEX support_messages_message_id ON support_messages (workspace_id, message_id);
CREATE INDEX support_messages_mailbox_created ON support_messages (mailbox_id, direction, created_at);
