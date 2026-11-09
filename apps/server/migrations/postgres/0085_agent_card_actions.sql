-- A click or form submit on a card that the team agent posted (a `ui` block) is queued for the
-- agent as kind 'action'. `card_message_id` is the message with the card (for the per-card limit)
-- and `action` the JSON that the sidecar reads: what was clicked, from the stored card, and the
-- form values.
ALTER TABLE agent_inputs DROP CONSTRAINT agent_inputs_kind_check;
ALTER TABLE agent_inputs ADD CONSTRAINT agent_inputs_kind_check
    CHECK (kind IN ('instruction', 'answer', 'steer', 'action'));
ALTER TABLE agent_inputs ADD COLUMN card_message_id TEXT;
ALTER TABLE agent_inputs ADD COLUMN action TEXT;

CREATE INDEX agent_inputs_card ON agent_inputs (card_message_id, created_at)
    WHERE card_message_id IS NOT NULL;
