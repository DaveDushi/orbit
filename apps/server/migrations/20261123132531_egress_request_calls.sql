-- SQLite. Postgres: postgres/20261123132531_egress_request_calls.sql.
--
-- Each sandbox call that asked for a host, linked to its egress request. A request used to keep only its newest call
-- (`agent_egress_requests.call_id` was moved to it), so the card of an older call (nonce `egress:<call>`) lost its
-- request: its buttons answered nothing. Now every card that asked shows the request and answers it. Additive: the old
-- column stays (the first call), and the existing links are copied.
CREATE TABLE agent_egress_request_calls (
    request_id TEXT NOT NULL REFERENCES agent_egress_requests(id) ON DELETE CASCADE,
    call_id TEXT NOT NULL REFERENCES agent_sandbox_calls(id) ON DELETE CASCADE,
    PRIMARY KEY (request_id, call_id)
);
CREATE INDEX agent_egress_request_calls_call ON agent_egress_request_calls (call_id);
INSERT INTO agent_egress_request_calls (request_id, call_id)
    SELECT id, call_id FROM agent_egress_requests WHERE call_id IS NOT NULL;
-- A request that a later call asked again had its `call_id` moved to that call; the egress log still has each call
-- that asked (`ask`, same agent, driver, thread, host and port, while the request waited). Link those calls too, so
-- the older cards get their request back.
INSERT INTO agent_egress_request_calls (request_id, call_id)
    SELECT DISTINCT r.id, l.call_id FROM agent_egress_log l
    JOIN agent_sandbox_calls c ON c.id = l.call_id
    JOIN agent_egress_requests r ON r.agent_user_id = l.agent_user_id AND r.driver_user_id = l.driver_user_id
        AND r.thread_key = c.thread_key AND r.host = l.host AND r.port = l.port
    WHERE l.decision = 'ask' AND l.created_at >= r.created_at
        AND l.created_at <= COALESCE(r.decided_at, r.expires_at, l.created_at)
    ON CONFLICT (request_id, call_id) DO NOTHING;

-- The hosts the model said a call needs (`sandbox_shell` `hosts`, a JSON array): when the call asks for one, its card
-- asks for all of them at once.
ALTER TABLE agent_sandbox_calls ADD COLUMN hosts TEXT;
