-- Failed sign-ins and registrations, shared by every server of the database
-- (apps/server/src/auth_routes/throttle.rs), so a lockout holds on all of them. `scope` keeps the sign-in
-- and the registration throttler apart.
CREATE TABLE auth_throttle_emails (
    scope TEXT NOT NULL,
    email TEXT NOT NULL,
    failures BIGINT NOT NULL,
    last_failure_at BIGINT NOT NULL,
    blocked_until BIGINT NOT NULL,
    PRIMARY KEY (scope, email)
);

CREATE TABLE auth_throttle_ips (
    scope TEXT NOT NULL,
    ip TEXT NOT NULL,
    failed_at BIGINT NOT NULL
);

CREATE INDEX auth_throttle_ips_by_ip ON auth_throttle_ips (scope, ip, failed_at);
