# Muse: from chat webhook to external agent with push (steps only)

Status: steps to follow after `external-agents-push` is deployed. Nothing here was done on production. Design:
`.ai/external-agents-design.md` §4.5. Product docs: `docs/agent.md#push`, `docs/mcp.md#external-agents`.

Today Muse is chat webhook `01a1debe-523a-70ab-b79b-db6148a811d8` in #marketing, with events on (to the Ops sink
`orbit-hooks.easytomanage.com/hooks-sink/15ba…`) and read with the webhook secret as Bearer. Rules that stay:

- Muse never gets a person's token, and never Donna's MCP chat access. It gets its **own** agent token, limited by its card.
- Do not change the old webhook until step 5 (the switch is tested first in a second channel).
- The signing secret and the agent token go only into Muse's secret store (or the Ops gateway's), never into chat,
  tickets or logs.

## 1. Make the agent (owner or admin, Orbit web)

1. Settings → Agentic → Agents → **New agent → External agent**.
2. Name **Muse** (the webhook's display name is not a user, so the handle is free; if Orbit says the name is taken,
   find which account has it before you pick another one). The same avatar as the webhook.
3. Channels: a **test channel** only (for example #muse-test, with the people who test). Not #marketing yet.
4. **Allow direct messages**: on, unless the user wants Muse in channels only.
5. Projects: none. Permissions: Chat read + Chat write only (the default).
6. **Create agent**.

## 2. Credentials (owner or admin)

1. On Muse's card: **Agent token** → 90 days → **Create token**. Copy the `orb_agent_…` token once into Muse's secret
   store. Muse answers with MCP `send_message` (`Authorization: Bearer <token>` to Orbit's MCP address).
2. **Push URL**: Muse's https endpoint (or a new path on the Ops gateway that forwards to Muse). It must be https on a
   public host; Orbit does not follow redirects and waits 3 s.
3. **Save**, then **Signing secret → Reveal**. Copy it into Muse's (or the gateway's) secret store.

## 3. Meta's side (Muse)

1. Check each push: `X-Orbit-Signature` = `v0=` + hex HMAC-SHA256 of `v0:{X-Orbit-Request-Timestamp}:{raw body}` with
   the signing secret, constant-time compare, refuse a timestamp older than 5 minutes. (The same check as the
   webhook's events, with the new secret.)
2. Answer `200` at once; drop an `event_id` already seen (a retry repeats it).
3. Reply with MCP `send_message` `{conversation_id: event.channel, thread_ts: event.thread_ts, text}` (instead of
   POSTing to the webhook URL with `thread_ts`). Optional: `set_status working` while the model runs.
4. Reading context: MCP `get_thread` / `list_messages` with the same agent token (instead of
   `GET /api/v1/hooks/chat/{webhook}/messages`). Only Muse's channels and DMs are readable.
5. Optional catch-up: call `wait_for_messages` with `timeout_seconds: 0` every few minutes for events whose 4 push
   tries failed, and `ack` them after answering.

If Meta's side cannot change its reply call, the Ops gateway can keep a thin adapter: it takes Muse's Slack-style
reply and calls MCP `send_message` with the agent token.

## 4. Test in the test channel

1. In #muse-test: `@Muse hello` → Muse's card shows a delivery `Delivered (200)` (Last pushes), and Muse answers in the
   thread under the message, as **Muse** (the bot user).
2. Reply in that thread **without** `@Muse` → it reaches Muse (event `type: message`, same `thread_ts`).
3. A DM to Muse (if allowed) → event `type: message`, `channel_type: im`.
4. A mention of Muse in a channel that is not on its card → no push; the person gets a DM that names Muse's channels.
5. Stop Muse's endpoint for a minute and mention it → the card shows `Push retrying`, then after about 6 minutes
   `Push failing`; start it again → the next push is `Delivered`, and the failed event is still in `wait_for_messages`.

## 5. Switch #marketing (owner or admin, with Ops)

1. On Muse's card, tick **#marketing** (and any other channel the user wants). Save.
2. In #marketing → Edit channel → Integrations → the Muse webhook: turn **events off**, remove its request URL, then
   **delete** the webhook. Its old messages stay with the webhook as author; new ones show the Muse bot user.
3. Ops removes the old sink entry (`hooks-sink/15ba…`) and the webhook secret from the gateway.
4. Remove the test channel from the card (or archive it).

No data migration: old webhook messages keep their author; nothing is copied.

## Rollback

Before step 5: delete the external agent (its token, connections, Push URL and secret stop at once); the webhook still
works. After step 5: make a new chat webhook in #marketing with events on (new URL and secret) and give them to Ops.
