-- "Suggest a change": change requests that installation admins send from the app. An agent outside Orbit reads
-- them with `orbit improvements list` and writes the status back with `orbit improvements update`
-- (docs/improvements.md). The text is untrusted input. Off until an admin turns it on.
CREATE TABLE improvement_requests (
    id TEXT PRIMARY KEY,
    author_id TEXT NOT NULL REFERENCES users(id),
    body TEXT NOT NULL CHECK (length(body) BETWEEN 1 AND 4000),
    context_path TEXT NOT NULL CHECK (length(context_path) <= 2048),
    context_viewport TEXT NOT NULL CHECK (length(context_viewport) <= 32),
    context_revision TEXT NOT NULL CHECK (length(context_revision) <= 128),
    status TEXT NOT NULL DEFAULT 'new' CHECK (status IN ('new', 'building', 'review', 'live', 'declined')),
    status_note TEXT CHECK (status_note IS NULL OR length(status_note) BETWEEN 1 AND 2000),
    link TEXT CHECK (link IS NULL OR length(link) BETWEEN 1 AND 500),
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);

CREATE INDEX improvement_requests_status ON improvement_requests (status, created_at);
CREATE INDEX improvement_requests_created ON improvement_requests (created_at);

ALTER TABLE instance_settings ADD COLUMN improvement_requests_enabled BIGINT NOT NULL DEFAULT 0
    CHECK (improvement_requests_enabled IN (0, 1));
