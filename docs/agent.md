# Team agent

Orbit has an optional AI agent that every member can use from chat. Mention it (**@Orbit Agent** until an admin
renames it) in a channel, a thread or a DM with it, and it does the work with Orbit's own tools (tasks, projects,
comments, Docs pages, chat messages). It answers in the thread and shows its progress while it works. Design and its
reasons: `.ai/agent-design.md`.

## What it can and cannot do

- It acts **as the person who gives the instruction** (the driver), with that person's permissions in one workspace.
  Anything that person cannot do, the agent cannot do either.
- It has **no shell, no file system, no internet and no delete tool**. It cannot change accounts, API tokens,
  members, invitations, integrations, storage or workspace settings: Orbit refuses those routes for agent sessions.
- Each person pays with **their own ChatGPT account** (Plus or Pro). They connect it in **Account settings → AI
  agent** with a device code, so it works on a remote server.
- When it needs information, it asks a teammate in the thread (`@`-mention) and stops. The next reply in the thread
  continues the run, as the original driver.
- Runs are durable: a restart of Orbit or of the agent continues them.

## Name and instructions

| Setting | Where | Who | Notes |
|---|---|---|---|
| Name | Admin → AI agent (`/admin/agent`, `PUT /api/v1/admin/agent`) | Installation admins | One name for the installation: it is the bot user's display name, so it is also the agent's `@handle`. 2–40 characters (letters, digits, single spaces, `- _ . '`). Orbit refuses a name that another account already has (trimmed, any case), because `@Name` would then reach that person. Messages and earlier mentions show the new name. |
| Team instructions | Workspace settings → AI agent (`/settings/agent`, `PUT /api/v1/workspaces/{id}/agent`) | Owners and admins write, every member reads | Plain text, at most 4,000 characters, stored in `agent_team_instructions`. |

The system prompt of a run has, in this order: the rules (with the current name), the built-in **About Orbit**
section (`apps/agent/src/prompt.ts`: Orbit's features, short how-to answers, what the agent can and cannot do; not
editable), the workspace's team instructions in a `<team-instructions>` block that says they cannot override the rules,
then the thread's context. The sidecar reads the name and the team instructions with each queued message and keeps
them for the run, so a change applies to the next run without a restart. Agent sessions cannot read or change either
setting. With `AGENT_MOCK=1`, the instruction `show prompt` answers with the current system prompt.

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
| `AGENT_MODEL` | `gpt-6.1-sol` | The ChatGPT (Codex) model id. |
| `AGENT_MAX_RUNS_PER_USER` | `3` | Runs one person can have going at the same time; more wait. |
| `AGENT_MAX_TOOL_CALLS` | `40` | Tool calls per run. |
| `AGENT_MOCK` | off | `1`: a scripted model and a fake ChatGPT sign-in, for tests without OpenAI. Never in production. |

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

### Docker Compose

Set `ORBIT_AGENT_SECRET` in `deploy/.env` and start the `agent` profile:

```bash
docker compose -f deploy/compose.yaml --profile agent up -d --build
```

## Data

- Orbit stores each person's ChatGPT credential sealed with `ORBIT_APP_KEY` (`agent_credentials`). Without an app key
  nobody can connect. Disconnecting deletes it.
- The agent stores transcripts in `AGENT_DATA_DIR`. It stores no tokens.
- One agent process owns its data directory. Several Orbit servers on one Postgres database can share one agent.

## Development

```bash
just agent        # the agent with the mock model, against `just dev` (API on 8080)
just agent-test   # its tests and typecheck
```
