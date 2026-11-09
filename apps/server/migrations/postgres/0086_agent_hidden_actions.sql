-- A click or form submit on the agent's card no longer posts a chat message: it is queued in
-- agent_inputs alone, under an id of its own. So `message_id` stops referencing chat_messages.
ALTER TABLE agent_inputs DROP CONSTRAINT agent_inputs_message_id_fkey;
