# This VM's Orbit deployment

Private team trial; upstream explicitly labels Orbit alpha, not production-ready.

- Canonical URL: https://orbit.wellrox.com/
- The exe.dev hostname `wellrox-orbit.exe.xyz` still reaches nginx, but browser writes must originate from the canonical URL.
- Source: DaveDushi/orbit, local merge commit 29c694cd495000cd62a0e0ba7b53cc42bc573209
  (`main` with the `agent-integrations` branch merged).
- Binary extracted from the image built locally on this VM from that commit
  (`orbit:29c694c`); fork commits have no published upstream image.
- Image ID: sha256:d11ba31eebb7a9accb29aafcf6b448e06db30d184743bcd503f894734f2c0f9c
- Team agent sidecar code (`/opt/orbit-agent`): the same merge commit 29c694c.
- Executable: /usr/local/bin/orbit; unprivileged system user: orbit.
- Config: /etc/orbit/orbit.env (production mode); registration remains closed in instance settings.
- Environment-only startup: /etc/systemd/system/orbit.service.d/environment.conf.
- Legacy TOML is retained at /etc/orbit/orbit.toml for the old binary and rollback only.
- SQLite, attachments, encryption key: /var/lib/orbit/ (mode 700).
- Local backups: /var/backups/orbit/ (mode 700).
- systemd: orbit.service, orbit-agent.service and nginx.service, enabled at boot.
- nginx listens on 8000; Orbit listens only on loopback port 8080; the team agent
  sidecar listens only on loopback port 8790 (see "Team agent sidecar").
- exe.dev terminates TLS and authenticates private web access; nginx forwards its HTTPS transport header. Orbit trusts only loopback.
- nginx access logging is disabled for this app to avoid logging invitation/setup tokens.

## Custom-domain configuration

Orbit's `/etc/orbit/orbit.env` sets `ORBIT_PUBLIC_ORIGIN` to
`https://orbit.wellrox.com` (no trailing slash). This controls the browser
origin policy as well as generated application links. Changing DNS alone does
not update it: a mismatched origin makes sign-in fail with
`The request origin is not allowed.`

nginx's `/etc/nginx/conf.d/orbit.conf` names both the custom domain and the
exe.dev hostname `wellrox-orbit.exe.xyz`. After changing the domain, validate the application config
and nginx config, restart `orbit.service`, and reload `nginx.service`.
Keep production origin checks and the existing trusted-proxy policy enabled.
The original pre-domain application config is saved locally at
`/etc/orbit/orbit.toml.before-custom-domain`.

For CLI commands, load the same environment as the service:

```sh
sudo -u orbit bash
set -a
source /etc/orbit/orbit.env
set +a
orbit config check
```

Verified through the local nginx proxy with the HTTPS forwarding header:
readiness is healthy; a login POST with the custom origin reaches request-body
validation (400 for an empty body), while an unrelated origin still receives
403 `origin_forbidden`. Both services are active. This verifies origin handling,
not a successful authenticated login through the public domain.

### VM rename (October 29, 2026)

The exe.dev VM was renamed from `victor-vs-isolani` to `wellrox-orbit`.
The DNS CNAME for `orbit.wellrox.com` now points to `wellrox-orbit.exe.xyz`,
and nginx `server_name` names the new hostname. The public URL resolves and
redirects to the exe.dev login page.

If `orbit.wellrox.com` does not resolve, this is **not** normal. First check
that its CNAME target is `wellrox-orbit.exe.xyz` (`getent hosts orbit.wellrox.com`).
If the VM is renamed again, update the CNAME, nginx `server_name` and the
`share add` command above.

## First login and team access

The one-time setup link is stored in /home/exedev/orbit-setup-link.txt (mode 600).
Open it to create the root account, then create your workspace and project.
Do not share this setup link with teammates.

Two access layers apply:
1. Grant teammates exe.dev **Web** access (`share add wellrox-orbit team`, or individual emails). Do not grant Root access just to use Orbit.
2. Invite them to your Orbit workspace; share Orbit's invitation links directly.

SMTP is optional, configured in Admin → Settings → Email. Without SMTP,
share invitation links manually. Registration remains closed.

## Team agent sidecar

The team agent (`@Orbit Agent` in chat until an installation admin renames it in Admin → AI agent; Account → AI agent)
is a Node sidecar from
`apps/agent` (design: `.ai/agent-design.md`, operations: `docs/agent.md`).

- Service: `orbit-agent.service`, user `orbit-agent` (no access to `/var/lib/orbit`),
  `Restart=always`, started after `orbit.service`. Unit from `deploy/orbit-agent.service`;
  the drop-in `/etc/systemd/system/orbit-agent.service.d/node.conf` points `ExecStart` at
  `/opt/orbit-node/bin/node` (Node 24.11, a copy; the distro has no Node 22.19+) and sets
  the data directory mode to 0700.
- Code: `/opt/orbit-agent` (copy of `apps/agent` with `npm ci --omit=dev`), owned by root.
- Data: `/var/lib/orbit-agent/pi.sqlite` (Pi Durable transcripts and runs, no tokens).
  Orbit's backups do not include it; back it up together with Orbit's data.
- Orbit settings in `/etc/orbit/orbit.env`: `ORBIT_AGENT_URL=http://127.0.0.1:8790` and
  `ORBIT_AGENT_SECRET` (64 hex characters). Without `ORBIT_AGENT_URL` the feature is off.
- Sidecar settings in `/etc/orbit/agent.env` (root, mode 600): `ORBIT_URL=http://127.0.0.1:8000`
  (through local nginx), `ORBIT_PUBLIC_URL=https://orbit.wellrox.com`,
  `ORBIT_FORWARDED_PROTO=https`, and the same `ORBIT_AGENT_SECRET`. Defaults are kept for
  `AGENT_PORT` (8790), `AGENT_MODEL`, `AGENT_MAX_RUNS_PER_USER`, `AGENT_MAX_TOOL_CALLS` and
  `AGENT_MEMORY_TOKENS` (3000; the memory read at the start of each run, `0` turns it off).
  `AGENT_COMPACT_TOKENS`, `AGENT_COMPACT_MODEL` and `AGENT_COMPACT_THINKING` keep their defaults (a conversation is
  summarized at about 250k tokens, by `gpt-6-luna` with thinking `high`).
  `AGENT_MOCK` is not set.
- To change the secret: write a new value (`openssl rand -hex 32`) into both files, then
  restart `orbit` and `orbit-agent`.
- Each user connects their own ChatGPT account in Account → AI agent (device code).
  Their tokens are sealed with the app key in Orbit's database (`agent_credentials`).
- Agent memory (team memory per workspace, private memory per person) is in Orbit's
  database (`agent_memory_*` tables, migration 63), so Orbit's backups include it. People
  see and delete entries in Account settings → Agent memory.
- Agent tools (remote MCP servers and HTTP APIs) are set up by workspace owners and admins in
  Workspace settings → Agent tools (`agent_integrations`, migration 65). Orbit keeps each tool's
  credential sealed with the app key and makes every external call itself (HTTPS, public
  addresses only); writes need the driver's "yes" in the thread. None is set up yet.
  `ORBIT_AGENT_INTEGRATION_TEST_HOSTS` is for local tests only and must stay unset here.

```sh
sudo systemctl status orbit-agent
sudo journalctl -u orbit-agent -n 100 --no-pager
curl -fsS http://127.0.0.1:8790/health           # {"ok":true,"mock":false}
sudo systemctl restart orbit-agent                # safe at any time; Pi resumes runs
sudo systemctl stop orbit-agent                   # mentions stay queued in Orbit
sudo systemctl disable --now orbit-agent          # keep it off after reboot
```

To update the sidecar code: stop `orbit-agent`, keep the old copy
(`/opt/orbit-agent.before-<sha>`) and a copy of its data
(`tar -C /var/lib -czf /var/backups/orbit-agent/orbit-agent-before-<sha>.tgz orbit-agent`),
copy `apps/agent` (`package.json`, `package-lock.json`, `tsconfig.json`, `src`) to a new
directory, run `npm ci --omit=dev` there, move it to `/opt/orbit-agent`, and start
`orbit-agent` after `orbit`.

To turn the feature off completely: `systemctl disable --now orbit-agent`, remove the two
`ORBIT_AGENT_*` lines from `/etc/orbit/orbit.env`, and restart `orbit`.

## Health and logs

```sh
sudo systemctl status orbit orbit-agent nginx
sudo journalctl -u orbit -n 100 --no-pager
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
```

## Backups

An initial empty-instance snapshot was created and verified. This is not a
backup of future team work. Enable daily automatic backups in Admin → Storage.
For disaster recovery, configure an off-host S3-compatible bucket there.
Local snapshots on this VM do not protect against VM loss.

Keep /var/lib/orbit/app.key with secure off-host backups: it is needed to
decrypt saved secrets. `ORBIT_APP_KEY_FILE` points to that existing key; do not
generate a replacement during upgrades. A local mode-600 copy is
/var/backups/orbit/app.key.

Manual CLI backup (the CLI requires the live app stopped):

```sh
sudo systemctl stop orbit
sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; orbit backup create'
sudo systemctl start orbit
sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; orbit backup list'
sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; orbit backup verify <id>'
```

If creating a backup fails, still start Orbit again after investigating.
Before upgrades, create and verify a backup and retain the current binary.
Never run two Orbit processes against the same SQLite database. Restore only
with Orbit stopped; see docs/operations.md and .ai/DEVELOPMENT.md.

## Expired setup link

While setup is incomplete:

```sh
sudo systemctl stop orbit
sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; orbit setup-token rotate' > /home/exedev/orbit-setup-link.txt
chmod 600 /home/exedev/orbit-setup-link.txt
sudo systemctl start orbit
```

## Verification performed

### October 30, 2026 upgrade: agent tools (integrations)

Merged the `agent-integrations` branch (through `b9fef38`, which already contains `main` up to `dcf4b8a`) into
`main` (merge `29c694c`); the merged tree equals the tested branch. The branch session ran, on the code of this
tree: clippy (`-D warnings`), `agent_integrations` (4, with local fake MCP and HTTP servers), `agent_api` (11),
`app_smoke` (18), platform `database` (43) and `backup` (21) on SQLite and Postgres; the sidecar typecheck and its
23 tests; the web typecheck, lint and build; and a browser run on a local test instance (owner page, approval
question, a teammate's "yes" refused, the driver's "yes" runs the write once). The image `orbit:29c694c` built with
the limited builder and `deploy/smoke.sh` passed.

Stopped `orbit-agent` and `orbit`. Counts before: schema 64, 6 users, 1 workspace, 2 projects, 15 tasks,
18 pages, 15 attachment blobs, 370 chat messages, 35 agent inputs, 3 ChatGPT connections, 0 agent memories.
Created and verified snapshot `01a18b85-244e-75b7-a046-a1314f0b8b07`
(`/var/backups/orbit/snapshots/01a18b85-244e-75b7-a046-a1314f0b8b07`) with the old CLI. Saved the sidecar data as
`/var/backups/orbit-agent/orbit-agent-before-29c694c.tgz` and the old sidecar code as
`/opt/orbit-agent.before-29c694c`; the old executable is `/usr/local/bin/orbit-51c3035` and the new one is also
kept as `/usr/local/bin/orbit-29c694c`. Rehearsed migration 65 (`0065_agent_integrations.sql`, two new tables) in
`/var/lib/orbit/rehearsal-29c694c-1J8f4d`, then applied it live: schema version 65, quick-check and foreign-key
checks pass, all counts unchanged. Installed the new sidecar `src` (with `integrations.ts`); `package.json`,
`package-lock.json` and `tsconfig.json` did not change. `/etc/orbit/orbit.env`, `/etc/orbit/agent.env` and nginx
did not change.

Verified: `orbit`, `orbit-agent` and nginx are active; readiness (all checks); revision `29c694c`; sidecar health
`mock: false`; the internal agent API answers 200 with the sidecar's secret and 401 without it;
`/api/v1/workspaces/<id>/agent-integrations`, `.../agent-tools`, `.../agent` and `/api/v1/agent` answer 401 without
a session; origin checks (400/403); the frontend serves `/settings/agent-tools`; https://orbit.wellrox.com/
answers 307/401 to the exe.dev login. Workspace settings → Agent tools was checked signed in, in a browser on the
URL `https://orbit.wellrox.com`, against a copy of the migrated production database served by the new binary (a
session was added to the copy only; no SMTP or S3 is configured; the copy was deleted): the page and its
navigation link load without errors. No agent tool is set up in production; an agent run with one needs a
person's ChatGPT account and was not tested from this VM. Stripe was not configured.

Rollback (an older binary cannot read a version 65 database; preserve the current database first; work after the
snapshot is lost):

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-51c3035 backup verify 01a18b85-244e-75b7-a046-a1314f0b8b07 && /usr/local/bin/orbit-51c3035 backup restore 01a18b85-244e-75b7-a046-a1314f0b8b07'
sudo install -m 755 /usr/local/bin/orbit-51c3035 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo cp -a /opt/orbit-agent.before-29c694c /opt/orbit-agent
sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-29c694c.tgz
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
```

### October 29, 2026 upgrade: agent name and team instructions

Merged the `agent-settings` branch (through `f5f68e1`, which already contains `main` up to `de26aa6`) into `main`
(merge `51c3035`). The merged tree is the tested branch plus `docs/donna-migration.md` (`21a3323`). The branch
session ran the sidecar, web, mobile (with the iOS `expo export`), clippy, `api-check`, and Rust tests on SQLite and
Postgres. On merged `main`: the sidecar typecheck and its 22 tests passed; `agent_api` (11), `database` (43) and
`backup` (21) passed on SQLite (same Rust code as the branch build); the image `orbit:51c3035` built with the
limited builder and `deploy/smoke.sh` passed.

Stopped `orbit-agent` and `orbit`. Created and verified snapshot `01a1893b-753a-7113-9e6f-a98aae29cf64`
(`/var/backups/orbit/snapshots/01a1893b-753a-7113-9e6f-a98aae29cf64`) with the old CLI. Saved the sidecar data as
`/var/backups/orbit-agent/orbit-agent-before-51c3035.tgz`, the old sidecar code as `/opt/orbit-agent.before-51c3035`
and the old executable as `/usr/local/bin/orbit-1a80ba8`. Rehearsed migration 64
(`0064_agent_team_instructions.sql`, a new table only) in `/var/lib/orbit/rehearsal-51c3035-e2Uksg`, then applied it
live: schema version 64, quick-check and foreign-key checks pass, counts unchanged (6 users, 1 workspace,
2 projects, 10 tasks, 340 chat messages, 21 agent inputs, 3 ChatGPT connections). Installed the new sidecar code
(`src` with `prompt.ts`); `package.json` and `package-lock.json` did not change, so `node_modules` is the installed
copy. `/etc/orbit/orbit.env`, `/etc/orbit/agent.env` and nginx did not change.

Verified: `orbit`, `orbit-agent` and nginx are active; readiness (all checks); revision `51c3035`; the sidecar polls
without errors and its health reports `mock: false`; the internal agent API returns `bot_name` "Orbit Agent" with the
sidecar's secret and 401 without it; `/api/v1/admin/agent`, `/api/v1/workspaces/<id>/agent` and `/api/v1/agent`
answer 401 without a session; origin checks (400/403); the frontend serves `/admin/agent`;
https://orbit.wellrox.com/ answers 307 to the exe.dev login. Admin → AI agent, Workspace settings → AI agent and
Account → AI agent were checked signed in on a copy of the migrated production database served by the new binary
(no production session was created; the copy was deleted): they load and show the name "Orbit Agent". The name is
unchanged and no team instructions are set. An agent run with team instructions needs a person's ChatGPT account
and was not tested from this VM. Stripe was not configured.

Rollback (an older binary cannot read a version 64 database; preserve the current database first; work after the
snapshot is lost):

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-1a80ba8 backup verify 01a1893b-753a-7113-9e6f-a98aae29cf64 && /usr/local/bin/orbit-1a80ba8 backup restore 01a1893b-753a-7113-9e6f-a98aae29cf64'
sudo install -m 755 /usr/local/bin/orbit-1a80ba8 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo cp -a /opt/orbit-agent.before-51c3035 /opt/orbit-agent
sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-51c3035.tgz
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
```

### October 29, 2026 upgrade: agent compaction (sidecar only)

Merged the `agent-compaction` branch (2 commits, through `9d2d551`, based on `ae06487`)
into `main` without conflicts (merge `01cd6b0`). It changes only `apps/agent` and docs, so
Orbit's binary (`orbit:1a80ba8`), its database and nginx did not change. On merged `main`:
the sidecar typecheck and its 20 tests passed.

Stopped `orbit-agent`. Saved its data as
`/var/backups/orbit-agent/orbit-agent-before-01cd6b0.tgz` and the old code as
`/opt/orbit-agent.before-01cd6b0`. Installed the new code (`src` with `compaction.ts`,
`package.json`, `package-lock.json`, `tsconfig.json`). `package.json` and
`package-lock.json` did not change and this VM has no `npm`, so `node_modules` is the
installed production copy. Started `orbit-agent`. `/etc/orbit/agent.env` did not change:
the defaults apply (`AGENT_COMPACT_TOKENS` 250000, `AGENT_COMPACT_MODEL` `gpt-6-luna`,
`AGENT_COMPACT_THINKING` `high`).

Verified: `orbit`, `orbit-agent` and nginx are active; Orbit readiness (all checks); the
sidecar started with the new settings and polls Orbit's queue without errors; sidecar health
`mock: false`; the internal agent API answers 200 with the sidecar's secret and 401 without
it; `/api/v1/agent` 401; https://orbit.wellrox.com/ answers 307 to the exe.dev login.
`pi.sqlite` is the same before and after (5 conversations, 39 entries, no open tasks); no
conversation is near 250k tokens yet, so no compaction ran. An agent run in chat needs a
signed-in person and their ChatGPT account and was not tested from this VM.

Rollback (sidecar only; agent work after the deploy is lost):

```sh
sudo systemctl stop orbit-agent
sudo rm -rf /opt/orbit-agent && sudo cp -a /opt/orbit-agent.before-01cd6b0 /opt/orbit-agent
sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-01cd6b0.tgz
sudo systemctl start orbit-agent
curl -fsS http://127.0.0.1:8790/health
```

### October 29, 2026 upgrade: agent memory

Merged the `agent-memory` branch (6 commits, through `75eb8a1`, based on `59e35f7`) into
`main` without conflicts (merge `1a80ba8`). The merged tree is the tested branch plus
this file only. The branch session ran the full suite (Rust on SQLite and Postgres,
sidecar, web, mobile, clippy, api-check). On merged `main`: the sidecar typecheck and its
8 tests passed, the release image `orbit:1a80ba8` built, and `deploy/smoke.sh` passed.

Stopped `orbit-agent` and `orbit`. Created and verified snapshot
`01a188ce-c5bf-74d5-96d8-8db0e32cdef2`
(`/var/backups/orbit/snapshots/01a188ce-c5bf-74d5-96d8-8db0e32cdef2`) with the old CLI.
Saved the sidecar data as `/var/backups/orbit-agent/orbit-agent-before-1a80ba8.tgz` and
the old sidecar code as `/opt/orbit-agent.before-1a80ba8`. Rehearsed migration 63
(`0063_agent_memory.sql`) in `/var/lib/orbit/rehearsal-1a80ba8-DwOWZ4`, then applied it
live: schema version 63, quick-check and foreign-key checks pass, counts unchanged
(6 users, 1 workspace, 2 projects, 10 tasks, 12 blobs, 266 chat messages, 2 ChatGPT
connections, 12 agent inputs). Installed the new sidecar code (with `memory.ts`).

Orbit, the sidecar and nginx are active. Verified: readiness (all checks), revision
`1a80ba8`, frontend and assets, origin checks (400/403), the agent memory routes
(`/api/v1/workspaces/<id>/agent-memory/{team,private}` and `.../wake`) answer 401
without a session (an unknown route answers 404), `/api/v1/agent` 401, sidecar health
`mock: false`, the internal agent API accepts the sidecar's secret, and
https://orbit.wellrox.com/ answers 307 to the exe.dev login. nginx did not change.
A signed-in check of Account settings → Agent memory and an agent run with memory were
not tested from this VM.

Rollback (an older binary cannot read a version 63 database; preserve the current
database first; memory and chat written after the snapshot are lost):

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-59e35f7 backup verify 01a188ce-c5bf-74d5-96d8-8db0e32cdef2 && /usr/local/bin/orbit-59e35f7 backup restore 01a188ce-c5bf-74d5-96d8-8db0e32cdef2'
sudo install -m 755 /usr/local/bin/orbit-59e35f7 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo cp -a /opt/orbit-agent.before-1a80ba8 /opt/orbit-agent
sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-1a80ba8.tgz
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
```

The previous executable is `/usr/local/bin/orbit-59e35f7`. `/etc/orbit/orbit.env` and
`/etc/orbit/agent.env` did not change.

### October 29, 2026 upgrade: team agent

Merged the `team-agent` branch (6 commits, through `bbb3fcb`) into `main` without
conflicts (merge `59e35f7`). Checks on the merge passed: clippy, 960 Rust tests (with
`agent_api`), `api-check`, web build/lint, the agent typecheck and tests, packages, and the
mobile typecheck. Not clean before this merge and not caused by it: `cargo fmt --check`
reports drift in upstream files, and two timing-dependent tests in
`apps/web/src/features/tasks/components/TaskList.test.tsx` also fail on the branch alone.

Built `orbit:59e35f7` locally (as `ci.yml`), and `deploy/smoke.sh` passed. Stopped Orbit,
created and verified snapshot `01a18843-5e7f-70d2-8ef7-e2148acabd35`
(`/var/backups/orbit/snapshots/01a18843-5e7f-70d2-8ef7-e2148acabd35`) with the old CLI,
and rehearsed migration 62 (`0062_agent.sql`) in
`/var/lib/orbit/rehearsal-59e35f7-WGf9zU`. Applied it live: schema version 62,
quick-check and foreign-key checks pass, counts unchanged (5 users, 1 workspace,
2 projects, 10 tasks, 12 blobs, 228 chat messages). Orbit created the `Orbit Agent` bot
user and its workspace membership at start.

Installed the sidecar as described in "Team agent sidecar" with a new secret (not the test
instance's). Verified: Orbit readiness (all checks), revision `59e35f7`, frontend and
assets, HSTS/CSP, origin checks (400/403), `/api/v1/agent` rejects unauthenticated
requests (401), the sidecar health reports `mock: false`, the sidecar accepts Orbit's
secret on `/login/<user>` (the call behind the "online" state) and rejects requests
without it, Orbit's internal agent API accepts the sidecar's secret and rejects a wrong
one, and https://orbit.wellrox.com/ answers 307 to the exe.dev login. nginx did not
change. A signed-in check of Account → AI agent, a ChatGPT connection and an agent run
were not tested from this VM.

Rollback (this changes data; preserve the current database first):

```sh
sudo systemctl disable --now orbit-agent
sudo systemctl stop orbit
sudo cp -p /etc/orbit/orbit.env.before-agent /etc/orbit/orbit.env
sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-6af5e60 backup verify 01a18843-5e7f-70d2-8ef7-e2148acabd35 && /usr/local/bin/orbit-6af5e60 backup restore 01a18843-5e7f-70d2-8ef7-e2148acabd35'
sudo install -m 755 /usr/local/bin/orbit-6af5e60 /usr/local/bin/orbit
sudo systemctl start orbit
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
```

The previous executable is `/usr/local/bin/orbit-6af5e60`; the previous Orbit
environment is `/etc/orbit/orbit.env.before-agent`. Chat, tasks and ChatGPT
connections made after the snapshot are lost by a restore.

### October 29, 2026 upgrade (locally built image)

Merged seven new commits from `origin/main`, through `bdaaecf`. That last
commit (pronouns removal) exists only on the DaveDushi/orbit fork, whose
GitHub Actions are off, so no registry publishes an image for it. The image
was therefore built locally from merge commit `6af5e60` with
`docker build --build-arg ORBIT_BUILD_REVISION=<full sha> -t orbit:6af5e60 .`
(as `ci.yml` does), and `deploy/smoke.sh orbit:6af5e60` passed before the
binary was extracted. The live `/orbit-build.json` confirms revision `6af5e60`.

Stopped Orbit, created and verified snapshot
`01a187bd-84d4-7706-83ad-a94be5865254` with the old CLI, and rehearsed
migrations 60–61 against a copy in `/var/lib/orbit/rehearsal-6af5e60-FoxSJS`.
Integrity and foreign-key checks passed, with counts unchanged. Applied the
same two migrations to the live database and restarted. Live quick-check and
foreign-key checks pass; counts remain 5 users, 1 workspace, 2 projects,
10 tasks and 12 attachment blobs.

Migration 61 drops `users.pronouns`; old values exist only in the snapshot
above. Migration 60 adds `attachment_blobs.deleted_at` for admin file
deletion. Stripe was not configured; cloud billing remains disabled.

Orbit and nginx are active and enabled. Verification through local nginx
with the HTTPS forwarding header passed: liveness, all readiness checks,
completed setup, matching embedded revision, frontend and three referenced
assets, HSTS/CSP, unauthenticated API rejection (401), canonical-origin login
body validation (400), and unrelated-origin rejection (403). No recent service
warnings. `orbit.wellrox.com` did not resolve at that time because its CNAME
still pointed to the old VM name (fixed after the rename below), so
public-domain access and authenticated browser workflows were not tested.

The previous executable is `/usr/local/bin/orbit-45df05b`. Rollback requires
the matching snapshot above, because the live schema advanced.

### October 27, 2026 evening upgrade

Fetched and merged seven new commits from `origin/main`, through `45df05b`.
Installed the executable extracted from `ghcr.io/coollabsio/orbit:45df05b`
at the image digest above after the isolated `deploy/smoke.sh` suite passed.
The live `/orbit-build.json` confirms the full upstream revision above.

Stopped Orbit, created and verified snapshot
`01a17edf-c340-77e3-9dd5-ebdb73ed90dc`, and rehearsed migrations 57–59
against a separate database copy. Full integrity and foreign-key checks
passed, with user/workspace/project/task counts unchanged. Applied the same
three migrations to the live database and restarted. Live quick-check and
foreign-key checks pass; counts remain 5 users, 1 workspace, 2 projects
and 10 tasks.

The default workspace storage quota changed from 5,000,000,000 to
1,000,000,000 bytes, as specified by migration 59. Cloud billing remains
disabled (`ORBIT_CLOUD` is unset/false); the new trial restrictions do not
apply to this self-hosted installation. One workspace is now marked
billing-exempt by migration 57. Existing files, app key, storage backend,
production environment and origin/proxy settings were preserved.

Orbit and nginx are active and enabled. Verification through local nginx
with the HTTPS forwarding header passed: liveness, all readiness checks,
completed setup, matching embedded revision, frontend and three referenced
assets, HSTS/CSP, unauthenticated API rejection, canonical-origin login body
validation (400), and unrelated-origin rejection (403). No recent service
warnings were reported. Authenticated browser workflows and external
private-domain access were not tested; no native mobile build was deployed.

The previous executable is `/usr/local/bin/orbit-4f149f1`.
**Rollback requires the matching pre-upgrade snapshot above**, not just the
old executable, because the live schema advanced. Preserve current data
before any restore; changes since that snapshot would be lost. Stop Orbit,
verify and restore that snapshot using the matching old CLI and existing
environment, restore the old executable, then start and verify readiness.
The environment-based systemd configuration does not need to change.

Update highlights: new layout default on web/mobile; resizable page-list
sidebar on web; regrouped workspace/admin settings with dedicated Email
and Limits pages; optional workspace-owned S3 file storage with background
file relocation; largest-file usage details; Cloud trial/plan UI and mobile
billing notices. These web/mobile feature changes came together from upstream;
this deployment changed no application code.

### October 27, 2026 upgrade

Fetched and merged `origin/main`, retaining the VM deployment documentation.
Installed the executable extracted from `ghcr.io/coollabsio/orbit:4f149f1`
at the image digest above after `deploy/smoke.sh` passed.
`/orbit-build.json` confirms the full upstream revision above.

Translated all existing TOML settings to `/etc/orbit/orbit.env` (root:orbit,
mode 640) and validated them with the new binary. The systemd drop-in loads
that file and replaces the obsolete `--config` command. The existing app key,
SQLite paths, canonical origin, loopback-only bind, proxy trust and limits
are unchanged.

Created and verified pre-upgrade snapshot
`01a17c80-10e3-7071-b2ea-2be1e9d38ca2`. Rehearsed migrations 47–56 against a
copy of its database: integrity passed, no foreign-key violations, and user,
workspace, project and task counts were unchanged. Applied the same ten
migrations to the live database while Orbit was stopped, then restarted.
After restart, the database still has 5 users, 1 workspace, 2 projects and
10 tasks; SQLite quick-check passes with no foreign-key violations.

Orbit and nginx are active and enabled. Checks through local nginx with the
HTTPS forwarding header passed: liveness/readiness, completed setup,
unauthenticated API rejection, embedded frontend and its three referenced
assets, HSTS/CSP, canonical-origin login validation (400 for an empty body),
and unrelated-origin rejection (403). No authenticated browser workflows or
external private-domain access were tested.

The previous executable is `/usr/local/bin/orbit-d83ce6c`, with the previous
base unit saved at `/etc/orbit/orbit.service.before-4f149f1`. **Rollback needs
the matching pre-upgrade database snapshot**, not only the old executable:
stop Orbit, use the old CLI with the retained TOML to verify/restore the
snapshot, restore the previous executable/unit, disable the environment
drop-in, reload systemd, then start and verify readiness. Preserve current
data before restoring; changes since the snapshot would be lost.

### October 14, 2026 upgrade

Merged `origin/main` while retaining the VM deployment documentation commits.
Installed the binary from `ghcr.io/coollabsio/orbit:d83ce6c` after the isolated
production image smoke suite passed. The binary embeds the full upstream
revision above. Existing SQLite storage and production configuration were kept;
Postgres support does not change this instance's database backend.

Stopped Orbit and created and verified snapshot
`01a13996-4324-7168-88dd-25ecc5cff4de` before installing the new binary.
The new CLI reported the existing database schema current. The previous
executable is retained at `/usr/local/bin/orbit-599976c`.
For rollback, stop Orbit before replacing the executable; if a future upgrade
changes the schema, restore its matching verified snapshot before using an
older executable.

After restart, Orbit and nginx are active. Readiness reports all checks healthy,
the embedded frontend returns 200 with HSTS and CSP, setup remains complete,
the canonical origin reaches login validation, and an unrelated origin is
rejected with 403. Authenticated browser workflows were not tested.

Upstream production image smoke suite passed: non-root execution, HTTPS
boundary, embedded frontend, setup required, no seeded login, production
seed/reset rejection, and restart readiness. Deployed service readiness,
frontend assets, security headers, origin rejection and local backup verification
passed. Private public-URL browser access could not be tested from this VM;
the owner should complete that check when opening the setup link. Multi-user
collaboration, SMTP and off-host recovery still require configuration/testing.
