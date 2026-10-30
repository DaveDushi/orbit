-- OAuth client credentials for HTTP agent tools (apps/server/src/agent/oauth.rs). The client ID and
-- secret are sealed apart from `sealed_secret`, so a binary without this feature sends no
-- credential at all, and never the sealed client as a header. The token stays in memory.
ALTER TABLE agent_integrations ADD COLUMN oauth_token_url TEXT;
ALTER TABLE agent_integrations ADD COLUMN oauth_sealed_client BYTEA;
-- What Test does a GET of, below the base URL.
ALTER TABLE agent_integrations ADD COLUMN test_path TEXT;
-- GraphQL over POST: a read at exactly this path, below the base URL, runs without the driver's
-- approval while the flag is on. Off by default; the path alone changes nothing.
ALTER TABLE agent_integrations ADD COLUMN graphql_read_path TEXT;
ALTER TABLE agent_integrations ADD COLUMN graphql_reads_enabled BIGINT NOT NULL DEFAULT 0;
