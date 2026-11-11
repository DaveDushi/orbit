# Webhook bots

A webhook bot is a bot that answers in chat. You run a small HTTPS service. When someone mentions the bot in one of
its channels, or writes to it in a direct message, Orbit sends the message to your service. Your service answers, and
the answer shows in chat as a message from the bot.

A bot only posts plain chat messages (Markdown, as people write them; HTML shows as text). It gets no tools of the
team agent and acts with nobody's permissions. Incoming webhooks (`docs/chat-webhooks.md`) are different: they only
post, they never receive messages.

## Make a bot

Owners and admins open **Settings → Agentic → Bots** and select **New bot**:

- **Name**: 2 to 40 characters. People mention the bot as `@Name`, so the name must not be the name of another
  account.
- **URL**: where Orbit sends messages. It must start with `https://` and reach a public address.
- **Channels**: where people may mention the bot. Orbit adds the bot to these channels. Every member can also send the
  bot a direct message.
- **Avatar**: optional, set after the bot is made.

After **Create**, Orbit shows the **signing secret** once. Copy it into your service. If you lose it, select **Rotate
secret**: Orbit signs with the new secret at once, and the old one stops working.

The bot's page also shows its 20 newest deliveries: the result, the HTTP status and a short reason. It never shows
message text.

## What reaches the bot

| Where | What reaches the bot |
|---|---|
| A channel in the bot's list | Messages that mention the bot. The answer goes in the message's thread. |
| A direct message of one person and the bot | Every message. The answer goes in the conversation. |
| A group direct message with the bot | Messages that mention the bot. The answer goes in the thread. |
| A channel not in the bot's list | Nothing. The bot tells the author in a direct message where it works. |

Guests cannot reach bots. Messages from bots and agents never reach a bot, so two bots cannot answer each other. A bot
gets at most 30 messages a minute. After that, people get one short note ("this bot gets too many messages"), and the
messages are not sent.

## The request

Orbit sends one `POST` with a JSON body:

```json
{
  "type": "message",
  "delivery_id": "0199…",
  "bot": { "id": "0199…", "name": "Echo" },
  "workspace": { "id": "0199…", "name": "Acme" },
  "conversation": { "id": "0199…", "name": "support", "kind": "public" },
  "thread_id": "0199…",
  "message": { "id": "0199…", "text": "<@0199…> what is the status?", "created_at": 1763280000000 },
  "author": { "id": "0199…", "name": "Ada" },
  "reply": {
    "url": "https://orbit.example.com/api/v1/hooks/bots/0199…/replies/kq3…",
    "token": "kq3…",
    "expires_at": 1763280030000
  }
}
```

- `conversation.kind` is `public`, `private` or `dm`. For a direct message, `name` is empty.
- `thread_id` is the thread the answer goes to; `null` in a direct message of one person and the bot.
- `message.text` is Markdown. Mentions are tokens: `<@user_id>` for a person, `<#conversation_id>` for a channel.
- Times are Unix milliseconds.

Headers:

| Header | Value |
|---|---|
| `Content-Type` | `application/json` |
| `User-Agent` | `Orbit-Bots/<version>` |
| `X-Orbit-Delivery` | The delivery ID (the same as `delivery_id`). |
| `X-Orbit-Timestamp` | Unix seconds when Orbit signed the request. |
| `X-Orbit-Signature` | `v1=` and the hex HMAC-SHA256 of `v1:{timestamp}:{body}`, with the signing secret as key. |

## Verify every request

Before you use a request:

1. Read the raw body as bytes. Do not parse and serialize the JSON again: the signature is over the exact bytes.
2. Refuse the request when `X-Orbit-Timestamp` is more than 5 minutes from your clock. This stops a replay of an old
   request.
3. Compute `v1=` + hex(HMAC-SHA256(secret, `v1:` + timestamp + `:` + body)) and compare it with `X-Orbit-Signature`
   in constant time.

Node.js:

```js
import { createHmac, timingSafeEqual } from 'node:crypto'

export function verifyOrbit(secret, headers, rawBody) {
  const timestamp = headers['x-orbit-timestamp'] ?? ''
  const signature = headers['x-orbit-signature'] ?? ''
  const sent = Number(timestamp)
  if (!Number.isInteger(sent) || Math.abs(Date.now() / 1000 - sent) > 300) return false
  const hmac = createHmac('sha256', secret)
  hmac.update(`v1:${sent}:`)
  hmac.update(rawBody)
  const expected = Buffer.from(`v1=${hmac.digest('hex')}`)
  const given = Buffer.from(signature)
  return given.length === expected.length && timingSafeEqual(given, expected)
}
```

Python:

```python
import hashlib, hmac, time

def verify_orbit(secret: str, headers, raw_body: bytes) -> bool:
    try:
        sent = int(headers.get("X-Orbit-Timestamp", ""))
    except ValueError:
        return False
    if abs(time.time() - sent) > 300:
        return False
    digest = hmac.new(secret.encode(), f"v1:{sent}:".encode() + raw_body, hashlib.sha256).hexdigest()
    return hmac.compare_digest("v1=" + digest, headers.get("X-Orbit-Signature", ""))
```

## Answer

The bot has **30 seconds** from the message to answer, in one of two ways:

- **In the HTTP answer (sync).** Answer `200` with `Content-Type: application/json` and `{"text": "…"}`. Orbit
  posts the text at once.
- **Later, through the reply URL (async).** Answer `200` (or `202`) with an empty body or `{}` quickly, then send one
  `POST` to `reply.url` with `{"text": "…"}` before `reply.expires_at`. No other credential is needed: the token in the
  URL is valid once, for this message only. Orbit answers `200 {"ok": true, "message_id": "…"}`; `404` when the URL
  was used, is older than 30 seconds, or the bot was deleted.

Rules for the text: Markdown, at most 4,000 characters (longer text is cut). `<@user_id>` mentions a person.
`<!channel>` and `<!here>` show as plain text: a bot never notifies a whole channel. The HTTP answer and the reply may
each be at most 64 KiB.

When the bot does not answer within 30 seconds, answers with a status other than 2xx, answers with a body that is not
JSON, or cannot be reached, the bot posts a short note instead ("No answer: …"), so nobody waits.

## Limits and safety

- Orbit follows no redirects and uses no proxy. Every address of the bot's host name must be public: Orbit checks the
  addresses after DNS resolution, at each request, so a DNS change cannot point a bot at a private address. Loopback,
  private, link-local (with `169.254.169.254`), shared and reserved addresses are refused, also in IPv6 and
  IPv4-mapped forms.
- Orbit stores the signing secret encrypted with the server's app key (`ORBIT_APP_KEY`), and the reply tokens only as
  SHA-256 hashes. It never logs the secret, the tokens or message text.
- `ORBIT_WEBHOOK_BOT_TEST_HOSTS` (a comma-separated list of host names or IPs) lets bots on these hosts use plain HTTP
  and local addresses. It is for local tests only and is off by default; Orbit logs a warning at start when it is set.
  Never set it in production.
