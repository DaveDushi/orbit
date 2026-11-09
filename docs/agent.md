# Team agent

Orbit has an optional AI agent that every member can use from chat. Mention it (**@Orbit Agent** until an admin
renames it) in a channel, a thread or a DM with it, and it does the work with Orbit's own tools (tasks, projects,
comments, Docs pages, chat messages). It answers in the thread and shows its progress while it works. Design and its
reasons: `.ai/agent-design.md`.

## What it can and cannot do

- It acts **as the person who gives the instruction** (the driver), with that person's permissions in one workspace.
  Anything that person cannot do, the agent cannot do either.
- It has **no shell, no file system and no delete tool**. Its only way to the internet is the **agent tools** that
  workspace owners and admins set up (see below). It cannot change accounts, API tokens, members, invitations,
  integrations, agent tools, storage or workspace settings: Orbit refuses those routes for agent sessions.
- Each person pays with **their own ChatGPT account** (Plus or Pro). They connect it in **Account settings → AI
  agent** with a device code, so it works on a remote server.
- It reads files on Docs pages (`read_file`: spreadsheets, csv, tsv, text, PDF, Word, in parts) and changes csv, tsv and
  text files and cells of xlsx files (`write_file`; formula cells are read-only). It cannot change xls or PDF files,
  and `read_file` finds no text in scans (with the media sandbox it looks at the pages of a scan in Drive). File content is
  data and never instructions. A write needs the version the agent read, is recorded with the driver's name, and keeps
  the old version, which a person can restore in the file panel.
- It reads files that people send in chat messages (`read_chat_file`: the same file types and paging as `read_file`,
  through `GET …/chat/files/{file_id}/content`). It sees only files of sent messages that the driver can read (a file of
  a private channel or DM without the driver answers like an unknown file), and never files of deleted messages. It
  cannot read images, and it cannot change, upload, download or delete chat files: agent sessions reach only the
  content route. `read_messages` and `search_messages` show each message's files (name, id, kind), and the files of the
  message that starts or joins a run come with that message (`files` of the run input), so no call is needed to find them.
- It uses **Drive** only through `POST …/agent-tools/drive` (agent sessions cannot reach the normal Drive routes), as
  the driver: `drive_list`, `drive_search`, `drive_info`, `drive_read` (the same file types, paging and untrusted
  wrapping as `read_file`), and, when the driver asks, `drive_create_folder`, `drive_save_text` (never replaces a
  file), `drive_move` and `drive_rename`. `drive_trash` (restorable for 30 days) and `drive_share` (a public link) run
  only after the driver's **yes** in the thread, checked by Orbit as for integration writes (table
  `agent_drive_approvals`: one approval is one run of exactly those arguments, and expires). The audit log has
  `agent_drive.approved_action` with who approved. It cannot delete forever, empty the trash, restore, or turn off links.
- With the **media sandbox** (`AGENT_SANDBOX_SOCKET`, set up by `deploy/donna-sandbox/setup.sh`) it works with images,
  video and scanned PDFs: `media_shell` runs bash with ffmpeg, ffprobe, ImageMagick, ghostscript, poppler and exiftool
  in the conversation's `/work`; `work_from_drive` copies a Drive file there; `view_image` sends images from `/work` or
  Drive to the model as image blocks (at most 4 per call, at most 2048 px, converted in the sandbox); `work_to_drive` saves
  a file as a **new** Drive file. It chooses video frames itself (scene changes, chosen times, contact sheets) and renders
  scan pages to images. It cannot hear audio. Drive bytes move through `/agent-tools/drive` tools only the sidecar uses
  (`drive_fetch`: a 5-minute download URL of a file the driver can see; `drive_upload_begin` with `fail` or `rename`,
  never `replace`; `drive_upload_complete`, `drive_upload_cancel`), at most 1 GiB per file; the model never sees the URLs.
  Each run is a systemd service instance (`orbit-donna@.service`, started by `orbit-donna.socket`): a root broker
  (`deploy/donna-sandbox/broker.py`) mounts the conversation's disk image (a sparse ext4 file of 2 GiB: the hard quota
  of `/work`) in the instance's private mount namespace, then runs bash as the user `donna-sandbox` in bubblewrap with
  an empty environment, every namespace unshared (no network), a read-only system (`/usr` and a few `/etc` files; no
  `/opt`, `/root`, `/home`, `/run`, `/var` or `/usr/local`) and only `/work` and a private `/tmp` writable. Limits: 2 CPUs
  and 2 GiB per run (3 GiB for all runs, `orbit-donna.slice`), 256 tasks, the call's time limit (at most 10 minutes)
  and 15 minutes per instance. The agent never opens a `/work` file itself: files go in and out through `cat` in a run.
  A conversation's `/work` is deleted after `AGENT_SANDBOX_IDLE_HOURS` without a run. Escape tests:
  `apps/agent/test/sandbox.test.ts` against a test copy (`AGENT_SANDBOX_TEST_SOCKET`).
- It can answer with **one card**: a table, a bar or line chart, a task list, a short form or buttons, built from a
  fixed catalog (`apps/agent/src/ui-catalog.ts`, json-render 0.22.0, catalog version 1). The model writes its text
  answer, then the card in json-render's inline format: a ```spec fence of JSONL lines, each an RFC 6902 patch
  (`apps/agent/src/ui.ts`). The card **builds live**: each progress edit (every 0.75 s while a card is written) carries
  the part written so far as a `ui` block with `live: true` (missing children left out), which takes no clicks. At the
  end the sidecar checks the whole card against the catalog and Orbit checks it again (`repositories/chat/ui.rs`: at
  most 200 elements in one tree from `root`, 8 levels, fixed types and props, no `$` keys, 48 KiB) and stores it on
  the answer; a card that fails either check leaves the text answer with "(The card could not be shown.)". Webhooks
  cannot post `ui` blocks. The web and phone apps show the card under the text (and only the text when the card does
  not fit their catalog). A **reply** button or a **form** queues an input of kind `action` for the agent, with no
  chat message: the button or form shows **Sent** (the reader's sends come from `GET …/messages/{id}/actions`, so also
  after a reload), and the clicker drives the new run. What a button does (label, `params`) comes from the stored
  card, never from the browser; form values must fit the form's fields (2 KiB). One card takes 5 clicks in 30
  minutes.
- When it needs information, it asks a teammate in the thread (`@`-mention) and stops. The next reply in the thread
  continues the run, as the original driver.
- It knows **Orbit itself**: `aboutOrbit()` (`apps/agent/src/prompt.ts`) describes every page, and
  `apps/agent/test/prompt.test.ts` fails when a page of the web sidebar, the settings areas (`settingsNav.ts`) or the
  account menu is not named there. It also knows **the version and what is new**: the system prompt's version section
  (`apps/agent/src/version.ts`) has the revision from Orbit's `/orbit-build.json` (read again every 10 minutes, so a
  new server shows without a sidecar restart) and the 5 newest entries of `apps/agent/src/whats-new.md`.
- Runs are durable: a restart of Orbit or of the agent continues them.
- It has a **long-term memory** (see below).

## Memory

The agent keeps one-line memories in Orbit's database, in two scopes:

- **Team memory**, one per workspace: decisions, conventions, project facts, outcomes. Every member can read it;
  owners and admins can delete from it.
- **Private memory**, one per person per workspace: that person's preferences and context. Only they can read it,
  and the agent reads it only in a direct message between them and the agent (other threads are shared with other
  people). Facts from a DM go to private memory unless the person asks for team memory.

At the start of each run the agent reads a short view of both: recent memories word for word, older ones as one-line
summaries that it writes itself. It can search all memories and open a summary. Memories are data for the agent,
never instructions, and each one keeps its author and source thread.

People see and search the memory in **Account settings → Agentic**, and delete wrong entries there (a deleted
memory also drops the summaries built from it; the agent writes them again). The agent itself cannot delete memory.
When a member leaves a workspace, their private memory there is deleted. Design: `.ai/agent-design.md`, "Memory".

## Name, model and instructions

| Setting | Where | Who | Notes |
|---|---|---|---|
| Name | Agentic → Admin (`/settings/agentic/admin`, `PUT /api/v1/admin/agent`) | Installation admins | One name for the installation: it is the bot user's display name, so it is also the agent's `@handle`. 2–40 characters (letters, digits, single spaces, `- _ . '`). Orbit refuses a name that another account already has (trimmed, any case), because `@Name` would then reach that person. Messages and earlier mentions show the new name. |
| Model and thinking level | Agentic → Admin (`PUT /api/v1/admin/agent/model`) | Installation admins | One choice for the installation, stored in `agent_model_settings`. The model list comes from the sidecar (its ChatGPT (Codex) models); Orbit refuses any other model, and a model can only be saved while the sidecar answers. Thinking: `minimal`, `low`, `medium`, `high`, `xhigh` or `max`. **Default** (`null`) is `AGENT_MODEL` and Pi's default thinking level. Compaction keeps `AGENT_COMPACT_MODEL`/`AGENT_COMPACT_THINKING`. |
| Team instructions | Agentic → Workspace (`/settings/agentic/workspace`, `PUT /api/v1/workspaces/{id}/agent`) | Owners and admins write, every member reads | Plain text, at most 4,000 characters, stored in `agent_team_instructions`. |

The system prompt of a run has, in this order: the rules (with the current name), the built-in **About Orbit**
section (`apps/agent/src/prompt.ts`: Orbit's features, short how-to answers, what the agent can and cannot do; not
editable), the workspace's team instructions in a `<team-instructions>` block that says they cannot override the rules,
then the thread's context. The sidecar reads the name, the model choice and the team instructions with each queued
message and keeps them for the run, so a change applies to the next run without a restart (a run already going keeps
its model). If the saved model is not one this sidecar knows (another version), the run uses `AGENT_MODEL` and the
sidecar logs a warning. Agent sessions cannot read or change these settings. With `AGENT_MOCK=1`, the instruction
`show prompt` answers with the current system prompt, and `show model` with the run's model and thinking level.

## Agent tools (integrations)

Owners and admins can give the agent tools outside Orbit in **Settings → Agentic → Workspace**:

- **Remote MCP server**: the server's Streamable HTTP URL (for example Cloudflare's MCP server). Press
  **Test** to read its tools, then turn on the tools the agent may use. Tools the server marks read-only are on at
  first; the others are off until an admin turns them on.
- **HTTP API**: a base URL. The agent gets one tool, `<prefix>__api_request` (method, path, query, JSON body). Every
  request stays below the base URL: Orbit refuses `..`, encoded dots and slashes, other hosts and absolute URLs.
  Choose the allowed methods (GET only at first).

Each integration has **one shared credential**: a header name (default `Authorization`) and its value, for example
`Bearer <token>`. Orbit seals it with `ORBIT_APP_KEY`, never shows it again, and never gives it to the agent
service: Orbit makes every call itself, as the driver of the run. Choose which **roles** (owners, admins, members)
may use each integration. The agent sees the tools as `<prefix>__<tool>`.

**OAuth client credentials (HTTP APIs).** For an API whose tokens expire, choose **OAuth client credentials** in the
form: token URL, client ID, client secret, the header for the token (default `Authorization`) and an optional test
path. Orbit sends `grant_type=client_credentials` with the client ID and secret to the token URL and uses the token
from the answer. In `Authorization` the value is `Bearer <token>`; in another header, the bare token. A fixed
credential and an OAuth client exclude each other: setting one clears the other. MCP servers cannot use OAuth clients.

- Orbit reads `expires_in` from the answer and renews 60 seconds before expiry. If `expires_in` is missing or at most
  60, the token serves one call only. The token stays in the memory of each server, so a restart asks for a new one.
- On a 401 from the API, Orbit gets a new token and retries once, also for a write that the driver approved (no second
  approval). It does not retry on a 403 or on a failed token request.
- The token request has the same limits as other calls (below), and a 3xx is a failure. An error shows only the HTTP
  status and a short OAuth error code, such as `shop_not_permitted`.
- **A new base URL or token URL needs the client ID and secret again.** So an admin who cannot read the secret cannot
  send it, or a token, to another host. The token URL can be on another host than the base URL.

**Test** does a GET of the optional test path, which stays below the base URL. An OAuth integration always gets a new
token. With no test path, Test reports that the token request worked and does not call the API. With a test path, an
answer below 400 is a pass; otherwise Test shows the status and says that the token worked but the API refused. A
fixed credential with no test path does a GET of the base URL.

**Example: Shopify Admin API.** Your app and the store must be in the same Shopify organization, or the token request
fails with `shop_not_permitted`.

| Field | Value |
|---|---|
| Base URL | `https://STORE.myshopify.com/admin/api/2026-10` |
| Token URL | `https://STORE.myshopify.com/admin/oauth/access_token` |
| Token header | `X-Shopify-Access-Token` |
| Test path | `shop.json` |

The form fills in the last three when the base URL host ends in `.myshopify.com`. Shopify's tokens end after 24 hours;
Orbit renews them. Shopify's GraphQL Admin API uses POST also for reads, and its REST Admin API is legacy. To use
GraphQL with no question for each read, allow POST and turn on **Run GraphQL reads without approval** (below). The form
fills in the path `/graphql.json`, but the switch stays off until you turn it on.

**GraphQL reads without approval (HTTP APIs).** This setting is off by default, per integration. When it is on, a POST
to the one exact **GraphQL path** runs without the driver's approval only if Orbit's own GraphQL parser finds a read in
the body that it sends. It does not trust what the agent says. All these cases still need approval:

- the body is not a JSON object, the query is missing or is not a string, or the query does not parse;
- the document has more than one operation, or an operation that is not a query, or a directive on the operation;
- a fragment is on `Mutation` or `Subscription`, or the document has schema definitions;
- the body has a key other than `query`, `variables` and `operationName` (for example a persisted query id), or
  `operationName` is not the name of the operation;
- the request has a URL query, or goes to any other path. Every other POST needs approval as before.

Each such call is in the audit log with `graphql_read_without_approval: true` and `approved: false`, and with no body.
Roles do not change: only the roles you chose can use the integration.

**Writes need the driver's approval.** An MCP tool that is not marked read-only, and an HTTP method other than GET
or HEAD (except a GraphQL read with the setting above), runs only after the agent asked the driver in the thread with the exact action and arguments and the
driver answered **yes**. Orbit checks this itself: the approval is bound to the driver, the thread, the tool and the
arguments, works once, and ends after 30 minutes. A teammate's "yes" does not count.

Limits: HTTPS only; Orbit refuses hosts that resolve to loopback, private, link-local or metadata addresses, follows
no redirects, waits at most 30 seconds and reads at most 1 MiB of an answer. Every external call is in the audit log
(`agent_integration.call`: tool, method, path, status; no bodies, no secrets).

**Risks to know.** Whatever the agent reads (tasks, pages, messages, tool results) could carry instructions written
by someone else. The agent is told to treat it as data, but it is a language model. A write tool can send Orbit
content out of Orbit; the approval step shows you exactly what goes out, so read it before you say yes. Give the
agent credentials with the smallest scope the service offers (read-only tokens where you can), and turn on only the
tools you need. A shared credential acts the same for every member who may use it: choose the roles with care.
**GraphQL reads without approval can return customer data** (orders, names, addresses) with no question to anyone. The
result goes to the agent, and from there into the thread and the model provider. The data is what the credential's
scopes allow, so give it only the scopes you want the agent to read. Orbit checks the query, not the service: if a
service runs a side effect from a query, Orbit cannot see it.

## Running it

The agent is a small Node service (`apps/agent`, Node 22.19 or later) next to Orbit. Orbit calls it on loopback, and
it calls Orbit back. Both sides share one secret.

| Orbit setting | Meaning |
|---|---|
| `ORBIT_AGENT_URL` | The agent's address, for example `http://127.0.0.1:8790`. Without it the agent is off. |
| `ORBIT_AGENT_SECRET` or `ORBIT_AGENT_SECRET_FILE` | The shared secret, at least 32 characters (`openssl rand -hex 32`). |

| Agent setting | Default | Meaning |
|---|---|---|
| `ORBIT_URL` | (required) | Orbit's address as the agent reaches it, for example `http://127.0.0.1:8000`. |
| `ORBIT_PUBLIC_URL` | `ORBIT_URL` | Orbit's public address, for links in messages. |
| `ORBIT_AGENT_SECRET` / `_FILE` | (required) | The same secret as Orbit's. |
| `ORBIT_FORWARDED_PROTO` | none | Set `https` when Orbit runs in production on loopback: Orbit accepts the header only from a trusted proxy address (`ORBIT_TRUSTED_PROXIES` must contain the agent's address, for example `127.0.0.1/32`). |
| `AGENT_HOST`, `AGENT_PORT` | `127.0.0.1`, `8790` | Where the agent listens. Keep it on loopback or a private network. |
| `AGENT_DATA_DIR` | `./data` | Pi Durable's SQLite file (`pi.sqlite`): transcripts and runs. Back it up with Orbit's data. |
| `AGENT_MODEL` | `gpt-6.1-sol` | The ChatGPT (Codex) model id of runs while Agentic → Admin has **Default**. |
| `AGENT_MAX_RUNS_PER_USER` | `3` | Runs one person can have going at the same time; more wait. |
| `AGENT_MAX_TOOL_CALLS` | `40` | Tool calls per run. |
| `AGENT_MEMORY_TOKENS` | `3000` | Tokens of memory the agent reads at the start of each run (team and private together; `0` turns it off). It is paid by the driver's ChatGPT account. |
| `AGENT_COMPACT_TOKENS` | `250000` | Tokens of context at which a conversation (a thread, or the main list of a DM with the agent) is summarized in the background; a request waits for the summary 20000 tokens above. Applies to each model whose own Pi threshold is higher. `0` keeps Pi's defaults. Older entries stay in `pi.sqlite`. |
| `AGENT_COMPACT_MODEL` | `gpt-6-luna` | The ChatGPT (Codex) model that writes compaction summaries, paid by the driver like the run. Empty: the conversation's model. When the old part of the conversation does not fit this model's window (with 30000 tokens for thinking and the summary) or its request fails, the conversation's model writes the summary. |
| `AGENT_COMPACT_THINKING` | `high` | Thinking level for the compaction summary: `minimal`, `low`, `medium`, `high`, `xhigh` or `max`. |
| `AGENT_SANDBOX_SOCKET` | none | The media sandbox's socket, for example `/run/orbit-donna/sandbox.sock`. Without it the agent has no media tools. `setup.sh` sets it in a drop-in of `orbit-agent.service`. |
| `AGENT_SANDBOX_IDLE_HOURS` | `24` | A conversation's `/work` is deleted after this many hours without a run (checked every hour). |
| `AGENT_MOCK` | off | `1`: a scripted model and a fake ChatGPT sign-in, for tests without OpenAI. Never in production. |

| Orbit test setting | Meaning |
|---|---|
| `ORBIT_AGENT_INTEGRATION_TEST_HOSTS` | Comma-separated hosts that agent tools may reach over plain HTTP and on local addresses, for local tests with fake servers (for example `127.0.0.1`). **Never set it in production**: it turns off the address check for those hosts. Orbit logs a warning when it is set. |

### systemd

```bash
sudo useradd --system --home /var/lib/orbit-agent orbit-agent
sudo cp -r apps/agent /opt/orbit-agent && cd /opt/orbit-agent && sudo npm ci --omit=dev
secret=$(openssl rand -hex 32)
echo "ORBIT_AGENT_URL=http://127.0.0.1:8790" | sudo tee -a /etc/orbit/orbit.env
echo "ORBIT_AGENT_SECRET=$secret" | sudo tee -a /etc/orbit/orbit.env
printf 'ORBIT_URL=http://127.0.0.1:8000\nORBIT_PUBLIC_URL=https://orbit.example.com\nORBIT_FORWARDED_PROTO=https\nORBIT_AGENT_SECRET=%s\n' "$secret" | sudo tee /etc/orbit/agent.env
sudo chmod 600 /etc/orbit/agent.env
sudo cp deploy/orbit-agent.service /etc/systemd/system/ && sudo systemctl daemon-reload
sudo systemctl enable --now orbit-agent && sudo systemctl restart orbit
```

`ORBIT_TRUSTED_PROXIES` must contain `127.0.0.1/32` (it does when a local nginx or Caddy is the proxy).

The media sandbox (optional; root, idempotent, also installs the packages it needs):

```bash
sudo deploy/donna-sandbox/setup.sh && sudo systemctl restart orbit-agent
```

The Docker Compose agent has no media sandbox.

### Docker Compose

Set `ORBIT_AGENT_SECRET` in `deploy/.env` and start the `agent` profile:

```bash
docker compose -f deploy/compose.yaml --profile agent up -d --build
```

## Data

- Orbit stores each person's ChatGPT credential sealed with `ORBIT_APP_KEY` (`agent_credentials`). Without an app key
  nobody can connect. Disconnecting deletes it.
- Agent tools and their sealed credentials are in `agent_integrations`; the driver's approvals of writes in
  `agent_integration_approvals`. Removing an integration deletes both. An OAuth client ID and secret are sealed
  together in `oauth_sealed_client`, apart from `sealed_secret`. OAuth access tokens are never written to the
  database: they stay in the memory of each server.
- The agent stores transcripts in `AGENT_DATA_DIR`. It stores no tokens.
- One agent process owns its data directory. Several Orbit servers on one Postgres database can share one agent.

## Development

```bash
just agent        # the agent with the mock model, against `just dev` (API on 8080)
just agent-test   # its tests and typecheck
```
