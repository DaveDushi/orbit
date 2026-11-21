# This VM's Orbit deployment

Private team trial; upstream explicitly labels Orbit alpha, not production-ready.

- Canonical URL: https://orbit.wellrox.com/
  Since November 12, 2026 (12:15 UTC, outside a deploy) nginx, `ORBIT_PUBLIC_ORIGIN` and the sidecar's
  `ORBIT_PUBLIC_URL` use `https://orbit.easytomanage.com`; `orbit.wellrox.com` answers a 301 to it. Local checks need
  `-H 'Host: orbit.easytomanage.com'` (with `Host: orbit.wellrox.com` they get the 301).
- The exe.dev hostname `wellrox-orbit.exe.xyz` still reaches nginx, but browser writes must originate from the canonical URL.
- Source: DaveDushi/orbit, commit 78f8c7d1d9e7b3b35dc2da9bdf9ee6218aff4969 on local `main`, not pushed
  (upstream port: blob store calls out of the write lock, write lock scopes, InfoTip fix, phone dialog sheets; schema
  20261119125533, 128 rows; November 21, 2026).
- Agent sandbox: installed for the Arthur pilot (Part A, 2026-11-19): broker + egress proxy, `ORBIT_AGENT_SANDBOX_KEY_FILE`
  in `/etc/orbit/orbit.env`, `AGENT_SANDBOX_BROKER_SOCKET` for the sidecar. Host guards live since 2026-11-16
  (`deploy/host-guards/README.md`). No agent has `sandbox_shell` ticked and no host is allowed: nothing runs until the
  owner does Part B (`deploy/agent-sandbox/PILOT.md` step 7).
- Support email (`docs/support.md`): inbound email is `POST /api/v1/hooks/support/{mailbox_id}/{secret}` on
  `ORBIT_PUBLIC_HOOKS_URL`. No mailbox is connected, and the hooks gateway does not let `/api/v1/hooks/support/` through
  yet: both are a separate Ops step (with its own yes), with the DNS/MX or Cloudflare Email Worker change. Mailbox SMTP
  passwords are encrypted with the existing app key (`ORBIT_APP_KEY_FILE`); no new env key.
- Chat webhook URLs and the `response_url` of button clicks use `ORBIT_PUBLIC_HOOKS_URL=https://orbit-hooks.easytomanage.com`
  (in `/etc/orbit/orbit.env` since f14e780; Cloudflare tunnel → hooks gateway → Orbit, outside this VM). Binaries before
  f14e780 refuse this key: a rollback to them also needs `/etc/orbit/orbit.env.before-hooks`.
- MCP public origin: `ORBIT_PUBLIC_MCP_URL=https://orbit-hooks.easytomanage.com` (in `/etc/orbit/orbit.env` since d8d3eb4,
  `docs/connect-your-agent.md`): a request with `Host: orbit-hooks.easytomanage.com` gets its own OAuth issuer and the
  resource `…/mcp`; authorize and consent stay on `orbit.easytomanage.com`. It works from outside only after Ops lets the
  MCP routes through the hooks gateway (`/etc/orbit-hooks-gateway/nginx.conf`, proposal `/tmp/mcpo/gateway/proposal.diff`).
  Binaries before d8d3eb4 refuse this key: a rollback to them also needs `/etc/orbit/orbit.env.before-d8d3eb4`.
- Binary extracted from the image built locally on this VM from that commit
  (`orbit:78f8c7d`); fork commits have no published upstream image.
- Image ID: sha256:109ec94ee844779f2be90f1f33118895ac70e75e659e22f5acaeb8c80d00f9b7
- Database pool: `ORBIT_DATABASE_MAX_CONNECTIONS` is not set (since 78f8c7d): SQLite keeps its default of 5, the same pool
  as before. Raise it only for Postgres (default 20 there).
- Team agent sidecar code (`/opt/orbit-agent`): commit 78f8c7d (what's new 2026-11-21: phone dialogs as bottom sheets,
  info tip fix; 2026-11-19: external agents with Push URL,
  support email, code mode with the agent sandbox built and off plus routine edits in chat, report and block with agent
  @handles and the /terms and /privacy pages; aboutOrbit: update_routine, the legal pages, report and block, @handles).
- iPhone push (APNs, `docs/operations.md` "iPhone push"): on since November 15, 2026, 12:07 UTC (set by Ops, outside a
  deploy): `ORBIT_APNS_KEY_FILE`, `ORBIT_APNS_KEY_ID`, `ORBIT_APNS_TEAM_ID`, `ORBIT_APNS_TOPIC` and `ORBIT_APNS_SANDBOX`
  are in `/etc/orbit/orbit.env`; the journal shows `iPhone push (APNs) is on` (topic `com.easytomanage.orbit`, sandbox
  false) at each start. A successful send writes no log line; a failed one writes `iPhone push not sent`.
- LibreOffice (Calc, Writer, Impress 24.2, `--no-install-recommends`) is installed for Office previews
  (`docs/file-previews.md`); ffmpeg makes the chat video copies.
- Desktop app downloads: https://orbit.easytomanage.com/downloads/desktop/ (installer, zip, `latest.json` for the app's update
  check; behind the exe.dev login like the rest). nginx `location /downloads/desktop/` in
  `/etc/nginx/conf.d/orbit.conf` serves `/srv/orbit-downloads/desktop/` (outside the deploy, so deploys keep it; the
  conf before this block is `/tmp/orbit.conf.before-downloads` until reboot). Publish a new version: push a tag
  `desktop-v<version>` (the version in `apps/desktop/Cargo.toml`), wait for the Desktop workflow, then
  `deploy/desktop-downloads.sh [<run id>]` as root (checks the sha256 values, removes older Orbit-* files).
- Donna's media sandbox: `deploy/donna-sandbox/setup.sh` (socket `/run/orbit-donna/sandbox.sock`, user
  `donna-sandbox`, `orbit-donna.slice`, `/var/lib/orbit-donna`, drop-in `orbit-agent.service.d/orbit-donna.conf`).
- OIDC gateway: `orbit-oidc-gateway.service`, nginx on `127.0.0.1:8400`, config in `/etc/orbit-oidc-gateway/`
  (see `docs/apps.md`). `cloudflared` is NOT installed.
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

The team agent (`@Orbit Agent` in chat until an installation admin renames it in Agentic → Admin)
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
- Each user connects their own ChatGPT account in Agentic → You (device code).
  Their tokens are sealed with the app key in Orbit's database (`agent_credentials`).
- Agent memory (team memory per workspace, private memory per person) is in Orbit's
  database (`agent_memory_*` tables, migration 63), so Orbit's backups include it. People
  see and delete entries in Agentic → You.
- Agent tools (remote MCP servers and HTTP APIs) are set up by workspace owners and admins in
  Agentic → Workspace (`agent_integrations`, migration 65). Orbit keeps each tool's
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

## Deploy steps

Only one deploy can run on this VM. `orbit-deploy` (`scripts/deploy-guard.sh`, setup in `deploy/README.md`) holds the
lock from the merge to `main` through the verified switch. It is separate from the `orbit-build` lock. Run every step
below through `orbit-deploy run`. A step that runs outside it is not protected.

```sh
orbit-deploy status                               # first, see if a deploy is running
orbit-deploy begin '<sha>: <what is deployed>'    # prints <id>; exit 75 if a deploy is running
orbit-deploy disk                                 # before the merge; exit 73 below 3 GiB free

# A rehearsal build before the merge is also part of the deploy: begin before it, and build with
# `orbit-deploy image <id> <branch sha>`. A fast-forward merge then reuses that image.
orbit-deploy run <id> -- git -C /home/exedev/orbit merge <branch>
orbit-deploy image <id> main                      # skips the build if a clean image of this exact commit exists
orbit-deploy run <id> -- deploy/smoke.sh orbit:<sha>
orbit-deploy run <id> -- sudo systemctl stop orbit-agent orbit
orbit-deploy run <id> -- sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; orbit backup create'
# Also through `run`: backup verify, migration rehearsal on a copy, copy of the sidecar data and code,
# keep the old executable as /usr/local/bin/orbit-<old sha>, install the new one as orbit and orbit-<sha>.
orbit-deploy run <id> -- sudo systemctl start orbit
orbit-deploy run <id> -- sudo systemctl start orbit-agent
orbit-deploy run <id> -- curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready

orbit-deploy end <id>                             # only after all the checks pass
```

- `orbit-deploy image` builds from a clean `git archive` of the commit and labels the image. It reuses `orbit:<sha>`
  only if that label has the same full commit. A real merge commit has another sha, so it builds. An image with no
  label (all images from before this change, or a manual build) is built again. **Always run the smoke test.**
- Every shell needs the `<id>`. Do not lose it. `orbit-deploy status` shows it.
- If a deploy fails, keep the lock while you repair or roll back. Run `end` after production is verified.
- The lock stays through a pause of up to 2 hours with no running step. A step of any length keeps it. To extend a
  pause, run `orbit-deploy run <id> -- true`. After the 2 hours, or after a holder crash or reboot, the lock is free and
  `run` and `build` refuse with exit 75.
- If `begin` or `status` says "the previous deploy did not end", production can be half switched. Check the services,
  the revision and the schema before you continue.
- Do not delete the lock file. To release the lock of a dead session, read the `<id>` from `status`, check production,
  and run `orbit-deploy end <id>`.
- **What's new for Donna:** before the build (with the merge), add an entry on top of `apps/agent/src/whats-new.md`
  (`## <deploy date>`, then 1-4 short lines about what people can see or do; one entry per date, so add lines to
  today's entry if it exists; keep the 8 newest) and commit it to `main`. A deploy with nothing new for people adds no
  entry. The file is in `apps/agent`, so a deploy that changes it also copies the sidecar code to `/opt/orbit-agent`
  and restarts `orbit-agent`. Donna takes the revision from `/orbit-build.json` herself. Name the entry in the
  upgrade section below.
- Put each new upgrade section first under "Verification performed", with its rollback command. Cleanup reads the
  first `install -m 755 /usr/local/bin/orbit-<sha> /usr/local/bin/orbit` line in this file as the rollback copy.

Cleanup of old binaries and images, after a verified deploy:

```sh
sudo orbit-deploy cleanup           # shows the list, removes nothing
sudo orbit-deploy cleanup --apply   # removes the list; refused during a deploy
```

Cleanup keeps the installed binary, the running process's binary, the rollback copy and the 3 newest other copies, and
the images with these names. After cleanup, the rollback commands of older upgrades below can name a binary that is gone.
It does not touch backups, `/opt/orbit-agent.before-*` or `/var/backups/orbit-agent`.

## Verification performed

### November 21, 2026 upgrade: upstream port (slow work out of the write lock, write lock scopes, InfoTip, phone dialog sheets; no migration)

`main` from `751b5647` to `78f8c7d1`: fast-forward to `upstream-port-1121` (5 commits). Ported from coollabsio/orbit:
`3b2f5f87` by hand (`e64b6c36`): S3/blob store calls leave the write transactions (`prepare`/`prepare_bytes`, then
`finalize_*_in_transaction`; also the fork's helpdesk intake, file previews, page file saves and workspace logo), reconcile
and the retention file purge call the store after commit, S3 timeouts (60 s + 1 s/MiB; HEAD/DELETE 15 s), Postgres
`lock_timeout` 30 s, new setting `ORBIT_DATABASE_MAX_CONNECTIONS` (5 SQLite / 20 Postgres), `PoolTimedOut` retried, an
unreadable job row becomes dead, batched retention purge (with the fork's Drive, logo, preview and page version tables).
`f9ebf329` by hand (`facbe51c`): `workspace_transaction` / `jobs_transaction` lock scopes (Postgres only; SQLite keeps
`BEGIN IMMEDIATE`, so production write behavior does not change), `ORBIT_WRITE_LOCK`, the slow-lock monitor; only the
upstream first wave uses the workspace scope, every fork-only writer stays global (fork note in
`docs/specs/2026-11-19-write-lock-scopes.md`). Cherry-picks `e4670de0` (`bc79a18e`, InfoTip click in a label no longer
focuses the field) and `ddc44ab1` (`0b3f661c`, dialogs as bottom sheets above the keyboard on phones; the fork's FileViewer
opts out). Not needed: `54c064a6` (our image workflow builds only `main` and inspects the tag it created) and `b46b7787`
(already formatted). No migration, no env change: `ORBIT_DATABASE_MAX_CONNECTIONS` stays unset (SQLite default 5, as
before). aboutOrbit: no change (it does not describe dialog positions or info tips). What's new: new entry `## 2026-11-21`
(2 lines: phone dialog sheets, info tip fix; `78f8c7d1`); the oldest entry (2026-11-12) left, 8 entries.

Checks on the branch: SQLite and Postgres, platform `--lib`, `write_lock`, `config`, `jobs`, `database`, `files` all
pass; server `--lib` and 23 suites (attachments, support, page files, logo, workspaces, tasks, pages, backup items, chat,
chat media, collab, Drive, agent api/drive/profiles/sandbox/task runs, external agents, scheduled, routine scripts,
app smoke) pass, except `chat_live::chat_events_go_to_the_members_who_may_see_them` on both databases: it also fails on
`main` (the test expects `/chat/<id>`; `push/mod.rs:858` adds `?workspace=` since `d46302e3`, not touched here). Web
`tsc -b`, mobile `tsc` and `bun test src` (349) clean; agent `prompt.test.ts` 5 pass. Browser e2e on a test instance
(new web build, then the image's own bundle): InfoTip click keeps focus off `project-key` (main: focuses it), New channel
on a 390 px phone is a sheet 8 px from the edges and ends above a simulated keyboard (main: centered). Screenshots
`/root/.local/share/com.jean.desktop/pasted-images/upstream-port-1121/`.

Deploy id `20261121T191950Z-110036`. `orbit:78f8c7d` (image
`sha256:109ec94ee844779f2be90f1f33118895ac70e75e659e22f5acaeb8c80d00f9b7`, label = the full commit; 19:20 to 20:00 UTC);
`deploy/smoke.sh` passed. Hot dry run on a copy: applied 0, schema unchanged, quick_check ok. Switch
(`/root/ops/upstream-port-1121/switch.sh`): stopped 20:05:05 UTC; snapshot `01a1ffb6-63fd-701d-b31e-4648ad8a7751` with
`/usr/local/bin/orbit-6e7d294`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-78f8c7d.sqlite`; the new binary on a copy: applied 0, schema unchanged,
quick_check ok; sidecar data `/var/backups/orbit-agent/orbit-agent-before-78f8c7d.tgz`, old code
`/opt/orbit-agent.before-78f8c7d`; ready 20:05:31 (26 s down). Sandbox egress proxy: the new binary copied to
`/usr/local/lib/orbit-sandbox/orbit` and `orbit-sandbox-egress` restarted. `vm-health.conf`: `PROD_TAG=78f8c7d`,
`ROLLBACK_TAG=6e7d294`, `ROLLBACK_BACKUP=01a1ffb6-…`.

Verified: `orbit`, `orbit-agent`, nginx, `orbit-sandbox.socket`, `orbit-sandbox-egress` active, 0 restarts; readiness
local and `https://orbit-app.easytomanage.com/health/ready` 200; `/orbit-build.json` revision `78f8c7d1…`; schema
20261119125533 (128 rows); proxy binary = orbit binary; `/opt/orbit-agent/src` = `main`; sidecar `/health` ok
(`mock: false`); journal since the switch: only the 2 known "allowed origin without a port" start warnings (also at the
6e7d294 start). Signed-in check as David Dusi (temporary session, every write blocked, logged out, session revoked;
`ui-check.ts`, all pass): InfoTip in Project settings → General → Tag shows its tip and leaves the field unfocused (a
click on the label text still focuses it); Chat → New → New channel at 390 × 844 is a bottom sheet (x 8, width 374,
bottom 836) and ends at 500 px above a simulated 336 px keyboard. Screenshots `prod-5-infotip-desktop.png`,
`prod-6-sheet-phone.png`, `prod-6-sheet-phone-keyboard.png` in the folder above. `vm-health`: nginx WARN = the switch
window only (connection refused, 0 5xx). Root-owned files of the merge given back to exedev. Scripts and logs:
`/root/ops/upstream-port-1121/`.

Rollback to `6e7d294` (same schema, so only the binary, the proxy copy and sidecar code):

```sh
sudo bash /root/ops/upstream-port-1121/rollback.sh
# = stop orbit-agent orbit; then:
install -m 755 /usr/local/bin/orbit-6e7d294 /usr/local/bin/orbit
# also the proxy copy, /opt/orbit-agent.before-78f8c7d back; start orbit, orbit-agent; restart orbit-sandbox-egress;
#   vm-health.conf.before-78f8c7d back
```

### November 19, 2026 upgrade: agents change, pause, resume and delete routines from chat (no migration)

`main` from `41c926e4` to `6e7d2949`: fast-forward to `routine-edit` (2 commits, rebased on `41c926e4`). New agent tool
`update_routine` (`apps/agent/src/scheduled.ts`; `TOOL_CATALOG` 62, `route_need`): an agent edits (instruction, schedule,
time zone, channel, name), pauses, resumes or deletes a routine through `/agent-tools/scheduled`
(`scheduled/routes.rs`, `scheduled/store.rs`) only after the driver's yes in the thread, with an old → new card (for a
delete: which routine). Same rules as Settings: only the creator edits or resumes; owners and admins may also pause or
delete anyone's; guests never. No migration, no env change. aboutOrbit (rule #76) names `update_routine` and who may do
what (it was in the branch). What's new: the rebase had no conflict; the branch's `## 2026-11-20` placeholder was removed
and its text joined to the 3rd line of the 2026-11-19 entry (still 4 lines, 8 entries; `6e7d2949`).

Checks on the rebased branch: agent `tsc` clean, agent tests 129 pass, 0 fail (40 skipped; Node 24); `scheduled_api`
13/13 on SQLite and on Postgres; `agent::profiles` 4/4. No browser check in this deploy (the e2e screenshots
`pasted-images/routine-edit/1-4` are from the test instance).

Deploy id `20261119T202614Z-3811883`. `orbit:6e7d294` (image
`sha256:b6f30492173780ba624e02f380a6bc8f425339abf8fce9b2bead94cec8c94da2`, label = the full commit; 20:27 to 21:10 UTC);
`deploy/smoke.sh` passed. Switch (`/root/ops/routine-edit/switch.sh`): stopped 21:12:15 UTC; snapshot
`01a1f5a7-2ae3-7092-a17e-76771504cc13` with `/usr/local/bin/orbit-15a4db4`, verified, `backup-rehearsal.sh` passed;
plain copy `/var/backups/orbit/pre-migration/orbit-before-6e7d294.sqlite`; the new binary on a copy: applied 0, schema
unchanged, quick_check ok; sidecar data `/var/backups/orbit-agent/orbit-agent-before-6e7d294.tgz`, old code
`/opt/orbit-agent.before-6e7d294`; ready 21:12:36 (21 s down). Sandbox egress proxy: the new binary copied to
`/usr/local/lib/orbit-sandbox/orbit` and `orbit-sandbox-egress` restarted. `vm-health.conf`: `PROD_TAG=6e7d294`,
`ROLLBACK_TAG=15a4db4`, `ROLLBACK_BACKUP=01a1f5a7-…`.

Verified: `orbit`, `orbit-agent`, nginx, `orbit-sandbox.socket`, `orbit-sandbox-egress` active, 0 restarts; readiness
local and `https://orbit-app.easytomanage.com/health/ready` 200; `/orbit-build.json` revision `6e7d2949…`; schema
20261119125533 (128 rows); proxy binary = orbit binary; `/opt/orbit-agent/src` = `main`; sidecar `/health` ok
(`mock: false`); journal since the switch: no warnings. `vm-health`: nginx FAIL = the switch window only (21:12,
connection refused, 0 5xx); OOM WARN = known; docker WARN = `orbit-mailpit`. Scripts and logs: `/root/ops/routine-edit/`.

Rollback to `15a4db4` (same schema, so only the binary, the proxy copy and sidecar code):

```sh
sudo bash /root/ops/routine-edit/rollback.sh
# = stop orbit-agent orbit; then:
install -m 755 /usr/local/bin/orbit-15a4db4 /usr/local/bin/orbit
# also the proxy copy, /opt/orbit-agent.before-6e7d294 back; start orbit, orbit-agent; restart orbit-sandbox-egress;
#   vm-health.conf.before-6e7d294 back
```

### November 19, 2026 upgrade: public Terms of Service and Privacy Policy pages (no migration)

`main` from `824db0ae` to `15a4db47`: fast-forward to `legal-pages` (rebased on `824db0ae`). Code: public routes
`/terms` and `/privacy` (`features/legal/*`, effective 2026-11-19, no Draft banner), "Terms · Privacy" links under the
sign-in form and in the account menu; `15a4db47`: aboutOrbit names the pages and the links, what's new 2026-11-19 (added
to the 4th line, the entry keeps 4 lines). Web and sidecar text only: no migration, no env change, no server code change.

Checks on `main`: web `tsc -b` clean; `legalRoutes.test.ts` 1 pass; agent `tsc` clean, `prompt.test.ts` 5 pass (Node 24).

Deploy id `20261119T191233Z-3782385`. `orbit:15a4db4` (image
`sha256:b09ae1d52c3be74ce6b34ae570bb7519409a7c06dc385d11b207395b3970c505`, label = the full commit; 19:13 to 20:02 UTC);
`deploy/smoke.sh` passed. Switch (`/root/ops/legal-pages/switch.sh`): stopped 20:02:51 UTC; snapshot
`01a1f567-a0a4-7155-ba7f-22a04434c9ea` with `/usr/local/bin/orbit-8e437f7`, verified, `backup-rehearsal.sh` passed; plain
copy `/var/backups/orbit/pre-migration/orbit-before-15a4db4.sqlite`; the new binary on a copy: applied 0, schema
unchanged, quick_check ok; sidecar data `/var/backups/orbit-agent/orbit-agent-before-15a4db4.tgz`, old code
`/opt/orbit-agent.before-15a4db4`; ready 20:03:10 (19 s down). Sandbox egress proxy: the new binary copied to
`/usr/local/lib/orbit-sandbox/orbit` and `orbit-sandbox-egress` restarted (the binary step of `setup.sh`; all of
`setup.sh` needs `orbit-agent-sandbox:8e437f7`, which is removed). `vm-health.conf`: `PROD_TAG=15a4db4`,
`ROLLBACK_TAG=8e437f7`, `ROLLBACK_BACKUP=01a1f567-…`.

Verified: `orbit`, `orbit-agent`, nginx, `orbit-sandbox.socket`, `orbit-sandbox-egress` active, 0 restarts; readiness
local and `https://orbit-app.easytomanage.com/health/ready` 200; revision `15a4db47…`; `/terms`, `/privacy`, `/login`, `/`
200; proxy binary = orbit binary; `/opt/orbit-agent/src` = `main`; journal since the switch: no warnings. Browser
without a session (`ui-check.ts`, all pass): `https://orbit-app.easytomanage.com/terms` and `/privacy`, light and dark,
200, no sign-in redirect, no Draft banner, "Effective: 2026-11-19", no console errors; `/login` shows the Terms and
Privacy links. Screenshots `/root/.local/share/com.jean.desktop/pasted-images/legal-pages/{terms,privacy}-{light,dark}.png`,
`login-links.png`. `vm-health`: nginx WARN = the switch window, 0 5xx; OOM WARN = known; docker WARN = `orbit-mailpit`.
Not done: a signed-in check of the account-menu link. Scripts and logs: `/root/ops/legal-pages/`.

Rollback to `8e437f7` (same schema, so only the binary, the proxy copy and sidecar code):

```sh
sudo bash /root/ops/legal-pages/rollback.sh
# = stop orbit-agent orbit; then:
install -m 755 /usr/local/bin/orbit-8e437f7 /usr/local/bin/orbit
# also the proxy copy, /opt/orbit-agent.before-15a4db4 back; start orbit, orbit-agent; restart orbit-sandbox-egress;
#   vm-health.conf.before-15a4db4 back
```

### November 19, 2026 upgrade: Arthur pilot Part A, agent sandbox installed, no agent ticked (no migration)

`main` from `9aec0e06` to `8e437f7e`: `--no-ff` merge of `sandbox-arthur-pilot` at `1d8e6c48` (no conflict; the branch
already had `faba3a4c`, so the merged tree differs from the tested branch only by the `9aec0e06` docs). Code: the broker
puts the call token, the proxy password and the script in a root-only `--process` spec file, never on a command line
(phase 9, finding 7); `send_files` posts result files in the thread (`POST /api/internal/agent/files`, sidecar secret);
`setup.sh --key-group`; aboutOrbit follows the sidecar's broker socket ("Agent sandbox (pilot)"). No migration, no
what's new (Part B adds it). Branch checks: session 2c618d23.

Deploy id `20261119T180921Z-3751892`. `orbit:8e437f7` (image
`sha256:1146e888475789bb2ee69dc6636efb6612f9d3928a2f3a9b75a75dcfa4d172b8`, label = the full merge commit; 18:10 to 18:55
UTC); `deploy/smoke.sh` passed. Code switch (`/root/ops/sandbox-pilot/switch.sh`): stopped 19:02:15 UTC; snapshot
`01a1f530-2504-72a4-9772-746f9fc3cafd` with `/usr/local/bin/orbit-faba3a4`, verified, `backup-rehearsal.sh` passed; plain
copy `/var/backups/orbit/pre-migration/orbit-before-8e437f7.sqlite` (sha256 `426044948dbfd66d…`); the new binary on a
copy of it: applied 0, schema and checksums unchanged, quick_check ok; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-8e437f7.tgz`, old code `/opt/orbit-agent.before-8e437f7` (`package.json` and
lock unchanged); ready 19:02:35 (20 s down). `vm-health.conf`: `PROD_TAG=8e437f7`, `ROLLBACK_TAG=faba3a4`,
`ROLLBACK_BACKUP=01a1f530-…`.

Pilot Part A (`PILOT.md` steps 2-6, `/root/ops/sandbox-pilot/pilot.sh`): host guards checked, all values as in step 2
(not changed). `orbit-agent-sandbox:8e437f7` built with `orbit-limited`, `setup.sh --client-group orbit-agent --key-group
orbit --image orbit-agent-sandbox:8e437f7 --orbit-url http://127.0.0.1:8080 --egress-bin /usr/local/bin/orbit`; rootfs id
`sha256:b457d2caee2dc3c6ecf212541b1141f1ca22ad94fc736500dd329137036d36d7` (the image is removed). `ProtectProc=invisible`
drop-ins (`protect-proc.conf`) on `orbit`, `orbit-agent`, `orbit-sandbox-egress`; `ORBIT_AGENT_SANDBOX_KEY_FILE` added
(old file `/etc/orbit/orbit.env.before-sandbox-pilot`); `orbit-agent.service.d/orbit-sandbox.conf` with
`AGENT_SANDBOX_BROKER_SOCKET`. Orbit restarted 19:03:23, ready 19:03:37. The nft table `inet orbit_sandbox` is made by the
broker only at the first sandbox start, so the step 3 check found none: made at once with the broker's own
`ensure_nft()` (env `/etc/orbit-sandbox/broker.env`), 2 `dport 3128` rules. After a reboot it is gone again until the
first sandbox: the proxy then listens on `0.0.0.0:3128` with no drop rule (it still needs a per-call password).

Verified: `orbit`, `orbit-agent`, nginx, `orbit-sandbox.socket`, `orbit-sandbox-gc.timer`, `orbit-sandbox-egress` active,
0 restarts, `ProtectProc=invisible` on the 3; readiness (all 6 checks) local, nginx and
`https://orbit-app.easytomanage.com/health/ready` (200); revision `8e437f7e…`; schema 20261119125533 (128 rows); runsc
`release-20261109.0`; key `-rw-r----- root:orbit`; socket `srw-rw---- root:orbit-agent`; the proxy binary = the orbit
binary; sidecar `/health` `sandbox: true`, its env has both sandbox sockets; aboutOrbit from `/opt/orbit-agent/src` with
the sidecar's env: "Agent sandbox (pilot)"; no secret (broker key, egress token, the secrets and key files of
`orbit.env` and `agent.env`) in any `/proc/*/cmdline`; `routes.sh`: was 404 `sandbox_off`, now 401/422, never 200;
0 sandbox settings, 0 egress rules, 0 calls, no agent with `sandbox_shell`; journal: no warning other than the known
start warnings. `vm-health`: nginx FAIL = 55 "Connection refused" lines of the two restarts (19:02, 19:03), 0 5xx; OOM
WARN = known; docker WARN = `orbit-mailpit`; git-owner WARN fixed with chown. Not done: Part B (the owner ticks Sandbox on
Arthur, first call), a signed-in check. Scripts and logs: `/root/ops/sandbox-pilot/`. After each later Orbit deploy, run
`setup.sh` again with the same options (the proxy's copy of the binary).

Rollback: the pilot alone (keeps 8e437f7) `sudo bash /root/ops/sandbox-pilot/pilot-undo.sh` (key line, sidecar drop-in,
`protect-proc.conf` removed; socket and proxy stopped; restarts). The code to `faba3a4` (no data step: no schema change):

```sh
sudo bash /root/ops/sandbox-pilot/rollback.sh
# = stop orbit-agent orbit; the pilot lines above; then the binary:
install -m 755 /usr/local/bin/orbit-faba3a4 /usr/local/bin/orbit
#   /opt/orbit-agent.before-8e437f7 back; start orbit, orbit-agent; vm-health.conf.before-8e437f7 back
```

### November 19, 2026 upgrade: external agents, phase 2: push (1 migration)

`main` from `86cc6251` to `faba3a4c`: `--no-ff` merge of `external-agents-push` at `4d531973` (1 commit, rebased on
`86cc6251`). Orbit sends each input of an external agent that has a **Push URL** to that URL (`agent/external_push.rs`,
a loop every 2 s), signed like chat webhook events (HMAC `v0:{ts}:{body}`, `X-Orbit-Signature` + `X-Slack-Signature`,
https on public addresses only, no redirects, 3 s); tries at once, then after 10 s, 60 s and 5 min; after the last try
the input stays for `wait_for_messages` and the card shows "Push failing". New routes `GET …/agents/{id}/push-secret`
and `POST …/agents/{id}/push-secret/rotate` (owners and admins); `push_url` in `POST/PATCH …/external`. A reply in a
thread where an external agent posted reaches it without a new @mention. Mobile: Edit on an external agent saves on
`PATCH …/external` (was 422). Migration `20261119125533_external_agent_push` (additive): `agent_profiles.push_url`,
`push_secret` (sealed with the app key), `push_since`; `agent_inputs.push_attempts`, `push_next_at` and the partial
index `agent_inputs_push`; table `agent_push_deliveries`. `ORBIT_AGENT_PUSH_ALLOW_LOCAL` is not set in
`/etc/orbit/orbit.env` (`switch.sh` refuses if it is; the server refuses it without `ORBIT_ENV=development`). No agent
has a Push URL after the switch (0 rows): Muse and webhook `01a1debe` are unchanged; the Muse switch is a separate step
(`.ai/muse-push-migration.md`). What's new: the push sentence added to the External agents line of the 2026-11-19 entry
("A server agent can get each message at once at its **Push URL**, and replies in a thread where it answered reach it
without a new @mention."); aboutOrbit: Push URL and thread follow-ups, next to the report/block and @handle text.

Rebase on `86cc6251`: conflict only in `apps/agent/src/prompt.ts` (aboutOrbit: kept the @handle sentence of main and
the push text of the branch). `external.rs` merged by itself and keeps `handle::email` on create and `handle::sync` on
rename; `whats-new.md` merged by itself (push text made shorter); `openapi_contract` counts and the SDK did not
conflict. Checks on the rebased branch: `external_agents` 12/12 and `agent_api` 22/22 on SQLite and Postgres;
`openapi_contract` 19/19; web `tsc` clean; mobile `tsc` clean, `bun test src` 349 pass; agent tests (prompt, profiles,
helpers, agent) 51/51, agent `tsc` clean. No browser check in this deploy.

Deploy id `20261119T164518Z-3697528`. `orbit:faba3a4` (image
`sha256:dd4a51f2678eb635b995f01a5f83d91e081da94133e25e70670ed7bff510b8d8`, label = the full merge commit; 16:46 to 17:35
UTC); `deploy/smoke.sh` passed. Migration dry run (hot copy, and in the switch on a copy of the stopped database;
`/root/ops/external-agents-push/dryrun.sh`): applied 1 then 0, schema 20261119125533 (128 rows), counts (13 users, 173
tasks, 1595 chat messages, 3 agents, 427 agent inputs, 2 unacked) and old checksums unchanged, quick_check ok, 0 FK rows;
`serve` on the copy under `unshare -n`: no error other than object storage (no network); the old binary refuses the new
schema; `rollback-schema.sql` (drop the table, the index and the 5 columns) gives the exact old schema and counts, the
old binary then starts (0 applied), and the new binary migrates again to the same schema.

Switch (`switch.sh`): stopped both services 17:43:30 UTC; snapshot `01a1f4e8-0e28-7452-b376-64cfd158061f` with
`/usr/local/bin/orbit-5b0d980`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-faba3a4.sqlite` (sha256 `28db47ca8750edc0…`); sidecar data
`/var/backups/orbit-agent/orbit-agent-before-faba3a4.tgz`; sidecar code copied (old `/opt/orbit-agent.before-faba3a4`;
`package.json` and the lock file unchanged). `orbit` and `orbit-agent` ready 17:44:20 (50 s down). `vm-health.conf`
(old copy `.before-faba3a4`): `PROD_TAG=faba3a4`, `ROLLBACK_TAG=5b0d980`,
`ROLLBACK_BACKUP=01a1f4e8-0e28-7452-b376-64cfd158061f`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness (all 6 checks) local and on
`https://orbit-app.easytomanage.com/health/ready` (200); `/orbit-build.json` revision `faba3a4c…`; schema 20261119125533
(128 rows), quick_check ok; journal: only the known start warnings; sidecar `/health` ok (`mock: false`),
`/opt/orbit-agent/src` = `main`. New routes with no session (`routes.sh`; before the switch 404 `route_not_found`):
`GET …/push-secret` and `POST …/push-secret/rotate` 401 `authentication_required`; `PATCH …/external` 401 before and
after; `/`, `/settings/agentic` 200. `vm-health`: nginx FAIL = 60 "Connection refused" lines of the switch window
(17:43-17:44), 0 5xx; OOM WARN = 5 known `gvisor_sentry` kills; docker WARN = `orbit-mailpit`; git-owner WARN (72 files
in `.git` from root git runs) fixed with chown to exedev. Not done: a signed-in check on production and a real push to
an outside URL (the Muse switch). Scripts and logs: `/root/ops/external-agents-push/`.

Rollback to `5b0d980` (undo the migration first: lost are Push URLs, push secrets and push tries; inputs stay for
`wait_for_messages`; the old binary refuses the new schema):

```sh
sudo bash /root/ops/external-agents-push/rollback.sh
# = stop orbit-agent orbit; .backup to /var/backups/orbit/pre-migration/orbit-before-rollback-to-5b0d980.sqlite;
#   sqlite3 /var/lib/orbit/orbit.sqlite < /root/ops/external-agents-push/rollback-schema.sql;
#   install -m 755 /usr/local/bin/orbit-5b0d980 /usr/local/bin/orbit; /opt/orbit-agent.before-faba3a4 back;
#   start orbit, orbit-agent; vm-health.conf.before-faba3a4 back
# Way 2, only for a data problem (work after 17:43 UTC is lost), Orbit stopped: /usr/local/bin/orbit-5b0d980 backup verify +
#   backup restore 01a1f4e8-0e28-7452-b376-64cfd158061f (as user orbit), then the binary and sidecar lines above
```

### November 19, 2026 upgrade: report messages and block people, agent @handles from their names (1 migration)

`main` from `ebc3aaa8` to `5b0d9803`: `--no-ff` merge of `upstream-mobile-1119` at `60497db3` (6 commits, rebased on
`ebc3aaa8`: `712c2598` report messages and block people (upstream port; web and iPhone), `9eb92f76` iPhone terms of
service and privacy links (the iPhone app only; they ship with the next iPhone build, no EAS build now), `0093742e`
tests, `6e44fc5d` lessons, `ba3f1e9b` agent @handles, `60497db3` what's new and aboutOrbit). Migration
`20261119120000_chat_reports_and_blocks`: new tables `chat_message_reports`, `user_blocks`; `notifications` rebuilt for
the kind `chat_reported` and the column `chat_report_id` (every old row kept). New routes: `POST …/chat/messages/{id}/report`,
`GET …/chat/reports`, `DELETE …/chat/reports/{id}/message`, `GET/PUT/DELETE /api/v1/chat/blocks[/{user_id}]`; pages
Settings → Reports (`chat.manage`) and Account → Blocked people.

Agent @handles: an agent bot user's email was `agent-<user id>@orbit.invalid`, so web, iPhone and desktop showed
`agent-<uuid>` as its handle (the part before `@`) and the `.invalid` address in member lists. It is now
`<handle>@agents.orbit.invalid`, the handle from the agent's name (unique in the workspace and across agents, `-2` on a
clash), kept in sync on rename (internal and external agents; `apps/server/src/agent/handle.rs`). A backfill at each
start changes the old emails (idempotent); on production it changed 2: Arthur → `@arthur`, Muse → `@muse`. User ids do
not change, so `<@user_id>` mentions, DMs and MCP are unchanged. Web and iPhone also hide any `*.invalid` address
(`@handle` instead) and turn an old `agent-<uuid>` handle into the name's handle (`packages/api/src/members.ts`); the
iPhone part ships with its next build. The default agent keeps `agent@orbit.invalid` (handle `@agent`).
What's new: report/block and @handles line in the 2026-11-19 entry (the sandbox line merged into the code mode line,
so the entry keeps 4 lines); aboutOrbit: report and block, agent @handles.

Rebase on `ebc3aaa8`: conflicts only in `openapi_contract.rs` (counts 315 paths, 410 operations) and the generated SDK
(made again from the merged `openapi.json`). Checks on the rebased branch: `agent_profiles` 10/10 (with the new handle
test: clash suffix, rename sync, mention by user id after a rename, backfill twice), `external_agents` 8/8 and
`chat_safety` 4/4 on SQLite and Postgres; `openapi_contract` 19/19; `handle::tests` slug unit test; `packages/api`
members helpers 4/4; mobile `tsc` clean, `bun test src` 347 pass, iOS bundle exported (`expo export --platform ios`;
no Metro ran); web `tsc`, build, lint (0 errors); agent prompt tests 12/12, agent `tsc` clean. `cargo clippy -p
orbit-server --lib --tests -D warnings`: 6 errors, all in files on `main` this merge does not touch (jobs.rs,
page_markdown_repair.rs, instance_settings.rs, tasks.rs, workspaces.rs, scheduled/runner.rs; a newer clippy). Browser
check of the built `dist` (test instance in `/tmp`, port 18741, strong password, stopped and deleted after): BEFORE with
the production binary `orbit-cf290c9` (2 agents + 1 external agent show `agent-<uuid>` in the @ list and
`agent-…@orbit.invalid` in Settings → Members; on a phone the handle hides the agent's name), AFTER with the branch
binary on the same data (backfill changed 3: `@donna`, `@research-bot`, `@claude-code`; Members shows `@handle`).
Screenshots `/root/.local/share/com.jean.desktop/pasted-images/agent-names/{before,after}-{mentions,members}-{light,dark}-{desktop,phone}.png`.

Deploy id `20261119T141917Z-3595582`. `orbit:5b0d980` (image
`sha256:17dc020fe26a6200e335835110b2847eaba9c4b77093cee5d8df1933831e653e`, label = the full merge commit; 14:20 to 15:46
UTC); `deploy/smoke.sh` passed. Migration dry run (hot copy, and in the switch on a copy of the stopped database;
`/root/ops/agent-handles/dryrun.sh`): applied 1 then 0, schema 20261119120000 (127 rows), counts, names and old checksums
unchanged, quick_check ok, 0 FK rows; `serve` on the copy under `unshare -n` ran the backfill (0 old emails left, 0
duplicate emails); the old binary refuses the new schema; `rollback-schema.sql` gives the exact old schema and counts,
the old binary then starts (0 applied), and the new binary migrates again to the same schema.

Switch (`switch.sh`): stopped both services 15:47:59 UTC; snapshot `01a1f47e-4bc6-75cb-a467-64ab87c5fd1c` with
`/usr/local/bin/orbit-cf290c9`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-5b0d980.sqlite` (sha256 `a8ea4dbc1cfd5f23…`); sidecar data
`/var/backups/orbit-agent/orbit-agent-before-5b0d980.tgz`; sidecar code copied (old `/opt/orbit-agent.before-5b0d980`;
`package.json` and the lock file unchanged). `orbit` and `orbit-agent` ready 15:48:44 (45 s down). `vm-health.conf` (old
copy `.before-5b0d980`): `PROD_TAG=5b0d980`, `ROLLBACK_TAG=cf290c9`, `ROLLBACK_BACKUP=01a1f47e-4bc6-75cb-a467-64ab87c5fd1c`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness (all 6 checks) local and on
`https://orbit-app.easytomanage.com/health/ready` (200); `/orbit-build.json` revision `5b0d9803…`; schema 20261119120000
(127 rows), quick_check ok; journal: `agent bot users got an @handle from their name changed=2`, then only the known
start lines; sidecar `/health` ok (`mock: false`), `/opt/orbit-agent/src` = `main`. New routes with no session
(`routes.sh`; before the switch 404 `route_not_found`): 401 `authentication_required`, the report POST 400
`invalid_request` (body checked first), never 200; `/settings/reports`, `/profile/blocked` 200. `verify-spa.ts` (not
signed in, socat TLS hop) passed. `vm-health`: nginx FAIL = 90 "Connection refused" lines of the switch window, 0 5xx;
OOM WARN = the 9 known `gvisor_sentry` kills; docker WARN = `orbit-mailpit`. Not done: a signed-in check on production
(needs a temporary session for a person, its own yes). Scripts and logs: `/root/ops/agent-handles/`.

Rollback to `cf290c9` (undo the migration first: lost are reports, blocks and their inbox notices; agent @handle emails
stay, the old binary ignores them; the old binary refuses the new schema):

```sh
sudo bash /root/ops/agent-handles/rollback.sh
# = stop orbit-agent orbit; .backup to /var/backups/orbit/pre-migration/orbit-before-rollback-to-cf290c9.sqlite;
#   sqlite3 /var/lib/orbit/orbit.sqlite < /root/ops/agent-handles/rollback-schema.sql;
#   install -m 755 /usr/local/bin/orbit-cf290c9 /usr/local/bin/orbit; /opt/orbit-agent.before-5b0d980 back;
#   start orbit, orbit-agent; vm-health.conf.before-5b0d980 back
# Way 2, only for a data problem (work after 15:47 UTC is lost), Orbit stopped: /usr/local/bin/orbit-cf290c9 backup verify +
#   backup restore 01a1f47e-4bc6-75cb-a467-64ab87c5fd1c (as user orbit), then the binary and sidecar lines above
```

### November 19, 2026 upgrade: code mode for every agent (sidecar only, no migration)

`main` from `1d896df1` to `d443958c`: `--no-ff` merge of `agent-codemode-2tools` at `776b5076` (7 commits, rebased on
`1d896df1`). Every agent (Donna and custom agents) is offered only `MODEL_TOOLS`: `find_tools` (finds a tool among the
agent's Orbit and integration tools by words, or by a read-only script over the catalog), `codemode`, `plan_job` and
`run_job` (direct: a job runs up to 60 min, a script 5 min), and `view_image`, `ask_teammate`, `load_skill`,
`read_skill_file` when the run has them. Every other tool, `sandbox_shell` included, is called from a `codemode` script.
A script call that asks for approval (or a waiting egress card) ends the script and the run; later calls are refused
while the run waits. Each script call shows a progress label in the agent's message. It changes only `apps/agent`,
`evals/donna-codemode` and docs: Orbit's binary (`orbit:cf290c9`), its database, nginx and the env files did not change;
no image was built. What's new: "code mode" line added to the 2026-11-19 entry (8 entries kept); aboutOrbit: the "How
agents work (code mode)" line names the direct tools.

Rebase on `1d896df1`: conflicts with the agent sandbox merge in `agent.ts` (import; `tools:` keeps main's sandbox line,
`modelTools` is `MODEL_REGISTRATIONS` for non-script runs), `tools.ts` (header kept as on main; `toolRegistrations` and
the extension list keep `SANDBOX_TOOLS` and add `FIND_TOOLS`), `profiles.test.ts` (imports; `find_tools` in the
`always` list) and `whats-new.md` (both lines kept). `find-tools.ts`, `codemode.ts` and `find-tools.test.ts` are
byte-equal to the tested branch.

Checks on the rebased branch (tree equal to the merge): agent `tsc` clean; agent tests 128 pass, 0 fail, 38 skipped
(27 agent-sandbox broker and 11 media-sandbox tests that need a live socket); `evals/donna-codemode/security.test.ts`
10/10. Browser check (built `apps/web/dist` embedded in the production binary `orbit-cf290c9`, test instance
`/root/orbit-test-codemode`, mock model, sidecar from the rebased worktree; `scripts/e2e-deploy.mjs`): a script with 4
reads shows its 4 progress labels; `plan_job` shows one approval card for 2 Drive trash calls; after **yes** the job
ran 2/2 and both files are in the trash; a script whose write asks for approval stops there (`Q3 budget.csv` not
trashed, 0 "Must not exist" tasks). Screenshots
`/root/.local/share/com.jean.desktop/pasted-images/codemode/0{1..4}-*-2-result.png`.

Deploy id `20261119T113701Z-3530459`. Before: production `cf290c9`, schema 20261119094300 (126 rows), sidecar code equal
to `main` `1d896df1`. DB backup (online `.backup`, Orbit kept running since its binary does not change):
`/var/backups/orbit/pre-migration/orbit-before-d443958.sqlite` (quick_check ok, schema 126 rows, sha256 `96ac2d08…`).
Migration dry run (installed binary on a copy, under `unshare -n`): applied 0, schema, counts and checksums unchanged,
quick_check ok, 0 FK rows. Switch: `package.json`, `package-lock.json`, `tsconfig.json` unchanged, so the installed
`node_modules` stays; stopped `orbit-agent` 11:37:39 UTC, data `/var/backups/orbit-agent/orbit-agent-before-d443958.tgz`,
old code `/opt/orbit-agent.before-d443958`, started 11:37:41, healthy 11:37:42 (3 s down; Orbit stayed up).

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness (all 6 checks) local and on
`https://orbit-app.easytomanage.com/health/ready` (200); sidecar `/health` `mock: false`, `/opt/orbit-agent/src` =
`d443958c`; schema 20261119094300 (126 rows); journal since the switch: only the known start lines; 0 nginx error lines
since the switch. `vm-health`: WARN oom (9 `gvisor_sentry` kills of the 2026-11-18 spike), WARN nginx (45 lines from the
11:02 switch), WARN docker (`orbit-codemode-s3` of the test instance, `orbit-mailpit`), WARN git-owner (root-owned
`.git` files of the rebase, given back to exedev). `vm-health.conf` unchanged (`PROD_TAG=cf290c9`,
`ROLLBACK_TAG=d7f0473`: the binary did not change). Not tested: a real-model run on production (needs a signed-in
person and their ChatGPT account). Scripts and logs: `/root/ops/codemode/`.

Rollback to cf290c9's sidecar code (Orbit, binary and DB stay):

```sh
sudo systemctl stop orbit-agent
sudo mv /opt/orbit-agent /opt/orbit-agent.d443958 && sudo mv /opt/orbit-agent.before-d443958 /opt/orbit-agent
sudo systemctl start orbit-agent && curl -fsS http://127.0.0.1:8790/health
# (the same as /root/ops/codemode/rollback.sh; only for a data problem also restore
#  /var/backups/orbit-agent/orbit-agent-before-d443958.tgz into /var/lib)
```

### November 19, 2026 upgrade: agent sandbox phases 1-9, installed OFF (2 migrations)

`main` from `164cb65e` to `cf290c99`: `--no-ff` merge of `agent-sandbox-design` at `31145f79` (18 commits: design,
gVisor spike, host-guard scripts, broker v2 with one sandbox per chat thread, Orbit data and routes for sandbox calls and
egress, egress proxy with per-call passwords, the `sandbox_shell` tool and egress card, the Internet access page, budgets,
notices, kill path and lost-call job, phase 9 security fixes). New routes `GET/PUT …/agents/{id}/sandbox`,
`POST/DELETE …/agents/{id}/egress-rules`, `GET …/agents/egress-requests`, `POST …/agents/egress-requests/{id}` and the
internal `…/agent/sandbox/calls`, `…/calls/{id}/end`, `…/egress/check`, `…/egress/done`, `…/runs/{id}/usage`,
`…/sandbox/stops/{id}/done`. Migrations (additive): `20261119094200_agent_sandbox`, `20261119094300_agent_sandbox_budgets`
(9 new tables, 6 indexes, column `agent_sessions.sandbox_call_id`). What's new: "Agent sandbox (built, off)" line in the
2026-11-19 entry.

**OFF for every agent.** No env change, no host change: `/etc/orbit/orbit.env` has no `ORBIT_AGENT_SANDBOX*` key and the
sidecar has no `AGENT_SANDBOX_BROKER_SOCKET` (`switch.sh` refuses to run otherwise). The phase 2 host guards
(`deploy/host-guards/`) were already live since 2026-11-16; this deploy did not change them. Every sandbox route answers 404 `sandbox_off`. 0 sandbox rows; no agent has `sandbox_shell` ticked.
The Arthur pilot needs its own yes.

Checks before the switch: server suites of the branch pass (`tests.log`, `tests2.log` in `/root/ops/agent-sandbox/`);
`offcheck.sh` on a migrated copy of the production DB in production mode under `unshare -n`: signed-in owner GET/PUT
sandbox, POST egress rule, GET egress-requests and the sidecar's `sandbox/calls` all 404 `sandbox_off`, inputs poll
`agent_sandbox=false`, 0 rows written. `orbit:cf290c9` (image
`sha256:b553dab223fa0efe0401b11377c9498515f86a649f00022f7a149a1d266c2252`); `deploy/smoke.sh` passed. Migration dry run
(hot copy, and in the switch on a copy of the stopped database): applied 2 then 0, schema 20261119094300 (126 rows),
counts, names and old checksums unchanged, quick_check ok, 0 FK rows; the old binary refuses the new schema;
`rollback-schema.sql` gives the exact old schema and counts, the old binary then starts (0 applied), and the new binary
migrates again.

Deploy id `20261119T110233Z-3521569`. Switch: stopped both services at 11:02:37 UTC; snapshot
`01a1f379-075e-75ba-9027-4df4436a75e5` (`/var/backups/orbit/downloads/01a1f379-…`) with `/usr/local/bin/orbit-d7f0473`,
verified, `backup-rehearsal.sh` passed; plain copy `/var/backups/orbit/pre-migration/orbit-before-cf290c9.sqlite`; sidecar
data `/var/backups/orbit-agent/orbit-agent-before-cf290c9.tgz`; sidecar code copied (old `/opt/orbit-agent.before-cf290c9`).
`orbit` and `orbit-agent` ready 11:02:57 (20 s down). `vm-health.conf` (old copy `.before-cf290c9`): `PROD_TAG=cf290c9`,
`ROLLBACK_TAG=d7f0473`, `ROLLBACK_BACKUP=01a1f379-075e-75ba-9027-4df4436a75e5`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness (all 6 checks) local and on
`https://orbit-app.easytomanage.com/health/ready` (200); `/orbit-build.json` revision `cf290c99…`; schema 20261119094300
(126 rows), quick_check ok; sidecar `/health` ok, `/opt/orbit-agent/src` = `main`; `/`, `/login`, `/tasks`, `/chat`,
`/settings/agentic/agents`, `…/agents/<id>/internet` 200. New routes with no session (`routes.sh`; before the switch 404
`route_not_found`): 404 `sandbox_off`, 422 (body checked first) or 401, never 200; `sandbox/calls` with the sidecar
secret 422 (refused before any write). Journal since the switch: only the known start lines (orbit "allowed origin
without a port" ×2, agent node SQLite ExperimentalWarning). `vm-health`: nginx WARN only from the switch (45 "Connection
refused" lines, 0 other, 0 5xx); OOM WARN = 9 `gvisor_sentry` kills from the 2026-11-18 sandbox spike, none since;
docker WARN = other sessions' containers; git-owner WARN = root-owned `.git` files, given back to exedev. Signed-in check
as David Dusi (1 temporary session, logged out and revoked; socat TLS hop; every API write aborted, 0 writes;
`ui-check.ts`), light and dark, 1440 px and 390 px phone: Settings → Agentic → Agents shows Arthur with no sandbox badge
and no Internet access button; Arthur's Edit dialog lists `sandbox_shell` unticked, and ticking it (local only, not
saved) shows no sandbox fields; `/settings/agentic/agents/<Arthur>/internet` says "Agent sandboxes are off on this
server."; GET sandbox 404 `sandbox_off`; no console errors. After: 0 sandbox settings, rules and calls; 0 agents with
`sandbox_shell`. Screenshots `/root/.local/share/com.jean.desktop/pasted-images/agent-sandbox/{agents,agent-edit-sandbox-tool,internet-access-off}-{desktop,phone}-{light,dark}.png`
(the `01-…`–`06-…` files there are from the test copies, not production). Changelog not posted (text
`/root/ops/agent-sandbox/changelog.md`). Scripts and logs: `/root/ops/agent-sandbox/`.

Open (each with its own yes): the Arthur pilot (sandbox key, broker socket, ticking `sandbox_shell`; Part A done in the
8e437f7 upgrade above).

Rollback to `d7f0473` (undo both migrations first: lost are only custom agents' model-token counts, since sandboxes were
OFF; the old binary refuses the new schema):

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit sqlite3 "file:/var/lib/orbit/orbit.sqlite?mode=ro" ".backup '/var/backups/orbit/pre-migration/orbit-before-rollback-to-d7f0473.sqlite'"
sudo -u orbit sqlite3 /var/lib/orbit/orbit.sqlite < /root/ops/agent-sandbox/rollback-schema.sql   # DROP 9 tables + the column, delete the 2 version rows
sudo install -m 755 /usr/local/bin/orbit-d7f0473 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-cf290c9 /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# (the same as /root/ops/agent-sandbox/rollback.sh)
# Way 2, only for a data problem (work after 11:02 UTC is lost), Orbit stopped: /usr/local/bin/orbit-d7f0473 backup verify +
#   backup restore 01a1f379-075e-75ba-9027-4df4436a75e5 (as user orbit), then the binary and sidecar lines above
```

After this rollback, copy `/root/ops/vm-health.conf.before-cf290c9` back to `/root/ops/vm-health.conf`.

### November 19, 2026 upgrade: helpdesk phase 1, support email as tickets in Tasks (1 migration)

`main` from `384aa68f` to `d7f0473d`: `--no-ff` merge of `helpdesk-mvp` at `64c33ad5` (`4a0bb5d6` remove the mock Mail
page and its seed; `46c943c3` feat: support email as tickets in Tasks: owners and admins connect a mailbox to a project
(Project settings → Support email), each inbound email becomes or joins a ticket task, Reply to customer sends by the
mailbox's SMTP (job `support.send`) and moves the ticket to Pending, Internal note stays a comment; `64c33ad5` Microsoft 365
mailboxes, Test connection, original requester on forwards). New routes `GET/POST/PUT/DELETE …/projects/{id}/support-mailbox`,
`POST …/support-mailbox/rotate`, `POST …/support-mailbox/test`, `GET …/tasks/{id}/support`, `POST …/tasks/{id}/support/replies`,
`POST …/support/messages/{id}/retry`, and the inbound hook `POST /api/v1/hooks/support/{mailbox_id}/{secret}` (raw MIME,
up to the upload body limit, rate-limited per mailbox, no Origin check). OpenAPI contract changed (310 paths, 404
operations): `openapi.json` and the SDK regenerated from the merged server (`orbit openapi`), equal to the branch's files.
Migration: the branch's `20261118180413_support_desk` (SQLite and Postgres) was older than production's 20261118180902, so
the merge renamed it to `20261119075838_support_desk` (nothing named the old file except `.ai/FEATURES.md`, updated). It
is additive: new tables `support_mailboxes`, `support_contacts`, `support_tickets`, `support_messages`, 6 indexes. Merge
with no conflicts; what's new: the branch's line moved from 2026-11-18 to the 2026-11-19 entry, with "This team's support
mailbox is not connected yet." (8 entries kept); aboutOrbit came from the branch ("Support email (helpdesk) is built into
Tasks"). No env change: `orbit config check` passed with the current `/etc/orbit/orbit.env` (SMTP passwords use the
existing `ORBIT_APP_KEY_FILE`; inbound URLs use `ORBIT_PUBLIC_HOOKS_URL`). No mailbox added, no DNS/MX/gateway change.

Checks on the merge: server `--test support_api` 12 pass and `--test openapi_contract` 19 pass (SQLite; `target` removed
after); web `tsc -b`, `packages/api` and agent `tsc` clean; web `productNavigation.test.ts`, `tasks/api/models.test.ts`,
`supportEmail.test.ts` 24 pass. Not run: the `orbit-platform` unit tests of the hook body limit and rate limit.

Deploy id `20261119T075830Z-3411958`. `orbit:d7f0473` (image
`sha256:41b53b92825c50cf5274c71507ccad0fd4ce7661d57a569b5a15459aae4e1960`; release compile 35m54s, after a wait for the
`orbit-build` lock of two other worktrees); `deploy/smoke.sh` and `orbit config check` passed. Migration dry run (hot copy,
and in the switch on a copy of the stopped database): applied 1 then 0, schema 20261119075838 (124 rows), counts, names and
old checksums unchanged, 4 new tables, 6 indexes, 0 support rows, quick_check ok, 0 FK rows; the old binary refuses the new
schema; `rollback-schema.sql` gives the exact old schema and counts, the old binary then starts (0 applied), and the new
binary migrates again.

Switch: stopped both services at 09:33:47 UTC; snapshot `01a1f327-b1e3-70d5-ab72-a543be00661b`
(`/var/backups/orbit/downloads/01a1f327-…`) with `/usr/local/bin/orbit-b2b7e80`, verified, `backup-rehearsal.sh` passed;
plain copy `/var/backups/orbit/pre-migration/orbit-before-d7f0473.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-d7f0473.tgz`; sidecar code copied (old `/opt/orbit-agent.before-d7f0473`).
`orbit` and `orbit-agent` ready 09:34:04 (17 s down). `vm-health.conf` (old copy `.before-d7f0473`): `PROD_TAG=d7f0473`,
`ROLLBACK_TAG=b2b7e80`, `ROLLBACK_BACKUP=01a1f327-b1e3-70d5-ab72-a543be00661b`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness (all 6 checks) local and on
`https://orbit-app.easytomanage.com/health/ready` (200); `/orbit-build.json` revision `d7f0473d…`; schema 20261119075838
(124 rows); sidecar `/health` ok, `/opt/orbit-agent/src` = `main`; `/`, `/login`, `/tasks`, `/chat`,
`/settings/agentic/agents` 200. New routes with no session (`routes.sh`): 401 (before the switch 404); `POST`/`PUT
support-mailbox` and `POST …/support/replies` with an empty body 400 (body checked before the session), with a valid body
401. Inbound hook (before the switch 403 `origin_forbidden`): unknown mailbox with a fake secret 404
`support_mailbox_not_found` (also with `Host: orbit-hooks…`), malformed id 404, no secret 404 `route_not_found`, `GET` 405;
nothing accepted. Journal since the switch: only the 2 known start lines (orbit "allowed origin without a port", agent
node SQLite ExperimentalWarning, the same at the 07:51 start). `vm-health`: nginx WARN only from the switch (40 "Connection
refused" lines, 0 other, 0 5xx); OOM WARN = `gvisor_sentry` kills before the deploy; docker WARN = other sessions'
containers; git-owner WARN = 33 `.git/worktrees/*` files of other worktrees (2026-11-18); the deploy's root-owned files in
the checkout were given back to exedev. Signed-in check as David Dusi (1 temporary session, logged out and revoked; socat TLS
hop; every API write blocked, 0 writes; `ui-check.ts`): Wellrox → General → Project settings → Support email shows the
empty "Connect mailbox" form (GET support-mailbox 200), light and dark, 1440 px and 390 px phone; no console errors; no
mailbox created (0 rows). There is no "Support" project on production; none was created. Screenshots
`/root/.local/share/com.jean.desktop/pasted-images/helpdesk/support-settings-{desktop,phone}-{light,dark}.png`.
Changelog not posted (text `/root/ops/helpdesk/changelog.md`). Scripts and logs: `/root/ops/helpdesk/`.

Open (Ops, with its own yes): connect the real support mailbox (Project settings → Support email), let
`POST /api/v1/hooks/support/` through the hooks gateway (`/etc/orbit-hooks-gateway/nginx.conf`), and the DNS/MX or
Cloudflare Email Worker change (`docs/support.md`, `docs/support/cloudflare-email-worker.js`).

Rollback to `b2b7e80` (undo the migration first: lost are support mailboxes, customer contacts, the ticket links and the
emails of tickets, and pending reply sends; ticket tasks, comments and attachments stay as plain tasks; the old binary
refuses the new schema):

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit sqlite3 "file:/var/lib/orbit/orbit.sqlite?mode=ro" ".backup '/var/backups/orbit/pre-migration/orbit-before-rollback-to-b2b7e80.sqlite'"
sudo -u orbit sqlite3 /var/lib/orbit/orbit.sqlite < /root/ops/helpdesk/rollback-schema.sql   # DELETE support.send jobs, DROP 4 tables, delete the version row
sudo install -m 755 /usr/local/bin/orbit-b2b7e80 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-d7f0473 /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# (the same as /root/ops/helpdesk/rollback.sh)
# Way 2, only for a data problem (work after 09:33 UTC is lost), Orbit stopped: /usr/local/bin/orbit-b2b7e80 backup verify +
#   backup restore 01a1f327-b1e3-70d5-ab72-a543be00661b (as user orbit), then the binary and sidecar lines above
```

After this rollback, copy `/root/ops/vm-health.conf.before-d7f0473` back to `/root/ops/vm-health.conf`.

### November 19, 2026 upgrade: external agents, phase 1 (1 migration)

`main` from `acfe6819` to `b2b7e800`: `--no-ff` merge of `external-agents` at `88f70a31` (`1fdfab1b` feat: external
agents, phase 1 (pull + identity): owners and admins create an agent that an outside client (Claude Code, Codex, a
server) runs through Orbit's MCP as the agent's bot user, by OAuth "Connect as" on the consent page or an `orb_agent_…`
token; its card limits channels, projects and scopes; new routes `POST …/agents/external`, `PATCH …/agents/{id}/external`,
`POST …/agents/{id}/tokens`, `DELETE …/agents/{id}/tokens/{id}`, `DELETE …/agents/{id}/connections/{id}`; `88f70a31` fix:
doc reads keep to the card, refusal text follows Allow DMs). Migration `20261118180902_external_agents` (additive:
`agent_profiles.kind/scopes/allow_dms/last_seen_at`, `oauth_grants.agent_profile_id/approved_by` + index,
`agent_inputs.delivered_at/delivery_attempts`, new tables `agent_profile_projects`, `agent_tokens`). Merge conflict only
in what's new: the branch's line moved to a new 2026-11-19 entry (the deploy date), 2026-11-11 dropped (8 kept);
aboutOrbit (the branch's "External agents" line) merged clean. No env change; no OpenAPI contract change.

Checks on the merge: web `tsc -b`, `packages/api` and agent `tsc` clean; `OAuthConsentPage.test.tsx` 8 pass; agent
`prompt.test.ts` + `profiles.test.ts` 10 pass; server `--test external_agents` (SQLite) 8 pass (`target` removed after).

Deploy id `20261119T065833Z-3377327`. `orbit:b2b7e80` (image
`sha256:f9689e1ce8cb5c0153d712c1d6bcece4e53a2b65957dff77dc5a5091451842e6`, ~41 min); `deploy/smoke.sh` and `orbit config
check` passed. Migration dry run (hot copy, and in the switch on a copy of the stopped database): applied 1 then 0, schema
20261118180902 (123 rows), counts, names and old checksums unchanged, 2 new tables, 8/8 new columns, the 1 existing
profile `internal` with `allow_dms` 1, quick_check ok, 0 FK rows; the old binary refuses the new schema;
`rollback-schema.sql` gives the exact old schema and counts, the old binary then starts (0 applied), and the new binary
migrates again.

Switch: stopped both services at 07:51:09 UTC; snapshot `01a1f2c9-bc3f-76f6-8bda-055039ca7df7`
(`/var/backups/orbit/downloads/01a1f2c9-…`) with `/usr/local/bin/orbit-1b62c91`, verified, `backup-rehearsal.sh` passed;
plain copy `/var/backups/orbit/pre-migration/orbit-before-b2b7e80.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-b2b7e80.tgz`; sidecar code copied (old `/opt/orbit-agent.before-b2b7e80`).
`orbit` and `orbit-agent` ready 07:51:31 (22 s down). `vm-health.conf` (old copy `.before-b2b7e80`): `PROD_TAG=b2b7e80`,
`ROLLBACK_TAG=1b62c91`, `ROLLBACK_BACKUP=01a1f2c9-bc3f-76f6-8bda-055039ca7df7`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness (all 6 checks) local and on
`https://orbit-app.easytomanage.com/health/ready`; `/orbit-build.json` revision `b2b7e800…`; schema 20261118180902 (123
rows); sidecar `/health` ok, `/opt/orbit-agent/src` = `main`; `/`, `/login`, `/settings/agentic/agents`, `/tasks`, `/chat`
200. New routes with no session (`routes.sh`): 401 (before the switch 404/405); `POST …/tokens` with an empty body 400
`invalid_request` (body checked before the session; a valid body gives 401); MCP with a fake `orb_agent_` token 401.
Journal since the switch: no warnings, no errors. `vm-health`: nginx WARN only from the switch (49 "Connection refused"
at 07:51, 0 other lines, 0 5xx); OOM WARN = `gvisor_sentry` kills before the deploy; docker WARN = other sessions'
containers. Signed-in check as David Dusi (2 temporary sessions, both logged out and revoked; socat TLS hop; every API
write blocked, 0 writes; `ui-check.ts`): Wellrox Settings → Agentic → Agents loads (GET agents 200), New agent offers
"Agent run by Orbit" and "External agent (Claude Code, Codex, a server)", the "New external agent" dialog opens (Name,
warning, Channels, Allow direct messages, Projects, Permissions); nothing created; no console errors. Screenshots
`/root/.local/share/com.jean.desktop/pasted-images/external-agents/{agents-page,agents-new-menu,external-agent-dialog}-{light,dark}.png`.
Changelog not posted (text `/root/ops/external-agents/changelog.md`). Scripts and logs: `/root/ops/external-agents/`.

Rollback to `1b62c91` (undo the migration first: lost are external agents' kind, scopes, projects and DM setting,
agent tokens, the "as agent" link of OAuth grants and the pull-delivery counters; an external agent then stays as a
plain agent profile; the old binary refuses the new schema):

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit sqlite3 "file:/var/lib/orbit/orbit.sqlite?mode=ro" ".backup '/var/backups/orbit/pre-migration/orbit-before-rollback-to-1b62c91.sqlite'"
sudo -u orbit sqlite3 /var/lib/orbit/orbit.sqlite < /root/ops/external-agents/rollback-schema.sql   # DROP 8 columns, 2 tables, 2 indexes, delete the version row
sudo install -m 755 /usr/local/bin/orbit-1b62c91 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-b2b7e80 /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# (the same as /root/ops/external-agents/rollback.sh)
# Way 2, only for a data problem (work after 07:51 UTC is lost), Orbit stopped: /usr/local/bin/orbit-1b62c91 backup verify +
#   backup restore 01a1f2c9-bc3f-76f6-8bda-055039ca7df7 (as user orbit), then the binary and sidecar lines above
```

After this rollback, copy `/root/ops/vm-health.conf.before-b2b7e80` back to `/root/ops/vm-health.conf`.

### November 18, 2026 upgrade: Enter sends on touch laptops, rules rows stack on phones (no migration)

`main` from `c0b8f0e5` to `1b62c91d`: `--no-ff` merges `9d85734f` of `enter-sends-touch-laptop` at `edd7a857` (chat:
a Windows touch laptop reports a coarse primary pointer, so the composer took it for a phone and Enter made a new line;
a new touch-only check, `(hover: none) and (pointer: coarse)` with no `(any-pointer: fine)`, now decides Enter and
autofocus in the composer, the message editor and the focus return; regression test `environment.test.ts`) and
`85fe9cd3` of `owner-names-mobile` at `457a2040` (Project settings → Rules rows stack below `sm`; `ProjectRulesCard.tsx`
only). No merge conflict. `1b62c91d`: sidecar what's new 2026-11-18 (both fixes added to the navigation line; the entry
keeps 4 lines) and aboutOrbit (Enter / Shift+Enter in the message box, also on a touch laptop; rules rows on a phone).
No migration, no env change, no server code change.

Checks on the merged `main`: web `tsc -b` clean; web lint (oxlint) exit 0, 0 errors (50 old warnings, none in the
touched files); `environment.test.ts` 3 pass; agent `tsc` clean, `prompt.test.ts` 5 pass.

Deploy id `20261118T193153Z-3148058`. `orbit:1b62c91` (image
`sha256:f569e5e5112b5a159ce4fb3d8c507fb9dbe9009bf909d993a54b1bedc30d60a9`; waited ~27 min for the `orbit-build` lock
behind other worktrees, compile ~45 min); `deploy/smoke.sh` and `orbit config check` passed. Migration dry run on a hot
copy: applied 0, counts, names and checksums unchanged, quick_check ok, 0 FK rows.

Switch: stopped both services at 20:50:52 UTC; snapshot `01a1f06d-3be8-70c3-b867-f1261e023ad0` with
`/usr/local/bin/orbit-4fa8e8f`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-1b62c91.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-1b62c91.tgz`; sidecar code copied (old `/opt/orbit-agent.before-1b62c91`).
`orbit` and `orbit-agent` ready 20:51:12 (20 s down). `vm-health.conf` (old copy `.before-1b62c91`):
`PROD_TAG=1b62c91`, `ROLLBACK_TAG=4fa8e8f`, `ROLLBACK_BACKUP=01a1f06d-3be8-70c3-b867-f1261e023ad0`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness (all 6 checks) local and on
`https://orbit-app.easytomanage.com/health/ready`; `/orbit-build.json` revision `1b62c91d…`; `/`, `/login`, `/tasks`,
`/docs`, `/chat`, `/drive` 200, API without a session 401; `/opt/orbit-agent/src` = `main`; journal since the switch: no
warnings. `vm-health`: nginx FAIL only from the switch window (67 "Connection refused" lines at 20:50-20:51, 0 other
lines, 0 5xx); OOM WARN = `gvisor_sentry` kills of other work before the deploy; docker WARN = other sessions' containers.

Signed-in check of the served bundle as David Dusi (temporary sessions, logged out, revoked; socat TLS hop; every API
write blocked, so a "sent" message is a caught and aborted `POST …/messages`; `ui-check.ts`), all pass: the loaded
`shared~DrivePage~ChatPage~AskDonnaPanel-qp7pfXx5.js` has the touch-only check; in David's DM with Donna, desktop and
touch laptop (matchMedia emulation: coarse primary + fine pointer): Shift+Enter a new line, Enter sends (1 caught POST,
box empty); touch only: Enter a new line, no POST. Slate → Project settings → Rules (5 rows): at 390 px light and dark the
label part takes the row (318 of 350 px), at 1280 px it shares the row (507 of 720 px); no console errors. Screenshots
`/root/.local/share/com.jean.desktop/pasted-images/web-deploy-enter-rules/prod-rules-{light-390,dark-390,light-1280}.png`
(with the branch screenshots). Changelog posted to #orbit-updates as David (temporary session, logged out; message
`01a1f071-f6db-73f6-b8b9-1ebfd072648f`; text `/root/ops/enter-rules/changelog.md`). Scripts and logs:
`/root/ops/enter-rules/`.

Rollback to `4fa8e8f` (same schema, so only the binary and sidecar code; `/root/ops/enter-rules/rollback.sh`):

```sh
sudo systemctl stop orbit-agent orbit
sudo install -m 755 /usr/local/bin/orbit-4fa8e8f /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-1b62c91 /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
```

After this rollback, copy `/root/ops/vm-health.conf.before-1b62c91` back to `/root/ops/vm-health.conf`.

### November 18, 2026 upgrade: one coherent navigation on the web (no migration)

`main` from `3de09ea3` to `4fa8e8f2`: `--no-ff` merge `b57cd48d` of `nav-sidebar-ux` at `f384b5d1` (web only: the
Account settings → Appearance → Layout choice (Classic / New UI) and the classic code path with `ProjectRail` are gone,
the old `orbit:layout` key is deleted; one saved, resizable width (220-480 px, `orbit:page_list_width`) for the open
sidebar on every page; the Tasks sidebar has All tasks (no second "All projects" row), favourite views under Views,
Back on top and Trash at the bottom; nav colours from sidebar tokens, so custom themes colour the whole app; visible
keyboard focus on sidebar links). No merge conflict. `4fa8e8f2`: sidecar what's new 2026-11-18 line (the entry keeps 4
lines: the imports and large-files lines are one now) and an aboutOrbit line on the one navigation; the prompt test's
sidebar sentinel "All projects" became "All tasks" (the merge alone made `prompt.test.ts` fail). No migration, no env
change, no server code change.

Checks on the merged `main`: web `tsc -b` clean; web `bun test` of `src/app/shell`, `lib/theme`, `features/profile`,
`features/settings`, `features/docs/components` and the touched task tests: 204 pass; `src/features/tasks` +
chat sidebar: 404 pass, 2 fail: "bulk toolbar rejects more than 100 selected tasks" (known 80 s timeout, also fails on
`main`) and "bulk due date picker schedules the selected tasks for this week" in the same `TaskList.test.tsx` (passes
alone; the file is unchanged by the merge, so it follows the timeout). Agent `prompt.test.ts` 5 pass, agent `tsc` clean.

Deploy id `20261118T142720Z-2978289`. `orbit:4fa8e8f` (image
`sha256:c34227a3e4ad35963a9877f9be854a0617f5a743518845547fcf83b2e38a742b`, compile 38 min); `deploy/smoke.sh` and
`orbit config check` passed.

Switch: stopped both services at 15:10:05 UTC; snapshot `01a1ef35-3baf-75e3-a16b-a362f3ebf0c8` with
`/usr/local/bin/orbit-42b5fc3`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-4fa8e8f.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-4fa8e8f.tgz`; sidecar code copied (old `/opt/orbit-agent.before-4fa8e8f`).
`orbit` and `orbit-agent` ready 15:10:19 (14 s down). `vm-health.conf` (old copy `.before-4fa8e8f`):
`PROD_TAG=4fa8e8f`, `ROLLBACK_TAG=42b5fc3`, `ROLLBACK_BACKUP=01a1ef35-3baf-75e3-a16b-a362f3ebf0c8`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness (all 6 checks) local and on
`https://orbit-app.easytomanage.com/health/ready`; `/orbit-build.json` revision `4fa8e8f2…`; `/`, `/login`, `/tasks`,
`/docs`, `/chat`, `/drive`, `/profile/appearance`, `/settings/general` 200, API without a session 401;
`/opt/orbit-agent/src` = `main`; journal since the switch: no warnings. `vm-health`: nginx FAIL only from the two switch
windows of this hour (80 "Connection refused" lines at 14:23-14:24 and 15:10, 0 5xx); docker WARN = other sessions'
containers.

Signed-in check of the served bundle as David Dusi (temporary session, logged out, revoked; socat TLS hop; every API
write blocked: only chat read markers and page visits were tried), `ui-check.ts`, light and dark at 1440 and 1200 px,
all pass: no `data-new-ui`, an old `orbit:layout=classic` is deleted; sidebar 260 px on `/`, `/tasks`, `/docs`, `/chat`,
`/drive`, `/settings/general`; ArrowRight ×2 → 292 px, kept after a reload on `/docs`; drag +60 → 352, drag stops at
480, double click → 260; Tab gives "All tasks" a solid 1 px outline; Tasks sidebar has All tasks, Views, My tasks,
Projects, Trash and no "All projects"; Appearance has Color theme and no Layout / Classic / New UI; no console errors.
Screenshots `/root/.local/share/com.jean.desktop/pasted-images/nav-sidebar-ux/prod-{light,dark}-{1440,1200}{,-appearance}.png`.
Changelog posted to #orbit-updates as David (temporary session, logged out; message
`01a1ef39-9310-73a3-8dca-a32d89fca329`; text `/root/ops/nav-sidebar/changelog.md`). Scripts and logs:
`/root/ops/nav-sidebar/`.

Rollback to `42b5fc3` (same schema, so only the binary and sidecar code; `/root/ops/nav-sidebar/rollback.sh`):

```sh
sudo systemctl stop orbit-agent orbit
sudo install -m 755 /usr/local/bin/orbit-42b5fc3 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-4fa8e8f /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
```

After this rollback, copy `/root/ops/vm-health.conf.before-4fa8e8f` back to `/root/ops/vm-health.conf`.

### November 18, 2026 iPhone build 21: a tapped push opens its conversation at the new message (TestFlight internal; no server change)

`main` from `084374b5` to `52b95085`: `--no-ff` merge of `iphone-push-open-chat` at `f80fc831` (`8ca99c9e`: a tapped
push opens a conversation you can write in; `f80fc831`: the conversation or thread opens at its newest message on a new
screen and loads again, so the new message shows also after a cold start). No merge conflict, no server or URL change.
`aboutOrbit()` already says how to get iPhone notifications; this is a fix of what a tap does, so no prompt change.
Merged only after the 8b614cc server deploy was verified.

Checks on the merged `main`: mobile `tsc --noEmit` clean, `bun test src/data/keyboard.test.ts` 4 pass (`push.ts` has no
test file). No native change since build 20 (`app.json`, `package.json`, `eas.json`, `bun.lock` unchanged), so
`eas build --non-interactive --no-wait` from the main checkout.

EAS build `111dec9c-5ea3-4be6-9588-6297757195a5`, build number 21 (version 1.0.0, commit `52b95085`, ~6 min).
Submission `79681107-dcf6-4dca-bd69-5c255061427d` with the App Manager ASC key (~1 min in the submission queue;
eas.json changed only for the run and restored; the temporary `AuthKey.p8` deleted). ASC build
`5f9020bc-2a63-47a9-870f-81c5d9bcf249`: `VALID` ~3 min after the submission, internal state `IN_BETA_TESTING`, external
`READY_FOR_BETA_SUBMISSION` (not submitted, not in the external group). Scripts and logs: `/root/ops/iphone-build21/`.

### November 18, 2026 upgrade: removing a member lifts the suspension their workspace set (no migration)

`main` from `08922f84` to `42b5fc3f` (fast-forward, deployed; branch `remove-clears-suspend`): removing a member
(`remove_member_unchecked`) also clears `suspended_at` and `suspended_workspace_id` when that workspace set the
suspension (a suspension by an instance admin or another workspace stays); the `membership.removed` audit row gets
`suspension_lifted`. Tests in `apps/server/tests/member_suspension.rs`. Sidecar: what's new 2026-11-18 line and one
aboutOrbit sentence. No migration, no env change, no UI change.

Deploy id `20261118T134447Z-2963930`. `orbit:42b5fc3` (image
`sha256:4572b7faedb218e73d21d61c632b88e94cdda9f182d679114418739d1f77d254`, ~38 min); `deploy/smoke.sh` and
`orbit config check` passed.

Switch: stopped both services at 14:23:54 UTC; snapshot `01a1ef0a-f33c-7065-a18f-8b3daffcf5f9` with
`/usr/local/bin/orbit-6122f46`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-42b5fc3.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-42b5fc3.tgz`; sidecar code copied (old `/opt/orbit-agent.before-42b5fc3`).
`orbit` and `orbit-agent` ready 14:24:07 (13 s down). `vm-health.conf` (old copy `.before-42b5fc3`):
`PROD_TAG=42b5fc3`, `ROLLBACK_TAG=6122f46`, `ROLLBACK_BACKUP=01a1ef0a-f33c-7065-a18f-8b3daffcf5f9`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness (all 6 checks) local and on
`https://orbit-app.easytomanage.com/health/ready`; `/orbit-build.json` revision `42b5fc3f…`; `/opt/orbit-agent/src` =
`main`; journal since the switch: no errors (only the 2 rmcp origin WARN lines that every start prints). `vm-health`:
nginx WARN only from the switch window (connection refused 14:23-14:24, 0 5xx); OOM WARN = 7 kills on Nov 17; docker
WARN = other sessions' containers. No signed-in browser check (server-only change). Changelog posted to
#orbit-updates as David (temporary session, logged out; message `01a1ef0b-c27f-763d-b898-1e66d926fdf3`; text
`/root/ops/remove-suspend/changelog.md`). Scripts and logs: `/root/ops/remove-suspend/`.

Rollback to `6122f46` (same schema, so only the binary and sidecar code; `/root/ops/remove-suspend/rollback.sh`):

```sh
sudo systemctl stop orbit-agent orbit
sudo install -m 755 /usr/local/bin/orbit-6122f46 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-42b5fc3 /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
```

After this rollback, copy `/root/ops/vm-health.conf.before-42b5fc3` back to `/root/ops/vm-health.conf`.

### November 18, 2026 upgrade: suspend a member, label rule unassign on GitHub, GitHub issue link (2 migrations)

`main` from `077e8de8` to `6122f461` (deployed): fast-forward to `adba522d` (`member-suspend`: owners and admins suspend
and resume a member, `POST`/`DELETE /api/v1/workspaces/{id}/members/{membership_id}/suspension`, migration
`20261118104258_workspace_member_suspension` (`users.suspended_workspace_id`, a nullable column only)), then merge
`6122f461` of `label-unassign-fix` (`e136b320` a label removed on GitHub unassigns the person its rule added, migration
`20261118090108_task_rule_assignees` (new table, backfill from the `task.assigned_by_rule` audit rows); `e55ba4e9` a
synced task shows "GitHub issue #N"). Merge conflict only in what's new: the 2026-11-18 entry keeps 4 lines (suspend
line; the label unassign fix, the issue link and the dark mic fix share one GitHub line); `aboutOrbit()` keeps both
branches' texts. `d1500146` (sidecar only, after the image): what's new + one aboutOrbit sentence for the issue link.
Mobile parts ship with iPhone build 22, not now.
Checks on the merge: agent `tsc` + `npm test` 111 pass / 0 fail / 11 skipped; web build ok, web tests 18 pass; mobile
`tsc` ok, `bun test src` 346 pass; server on SQLite AND Postgres: `member_suspension` 2, `label_rules_api` 13,
`tasks_api` 100, `workspaces_api` 30, `openapi_contract` 19 pass (`target` removed after).

Deploy id `20261118T120822Z-2915420`. `orbit:6122f46` (image
`sha256:15b7a64ddb4be4eb5914ff9489e358f042bb45d262928a55ee5e79d7f2369ae2`, ~38 min, deps cached); `deploy/smoke.sh` and
`orbit config check` passed. Migration dry run of BOTH migrations (fresh hot copy, and in the switch on a copy of the
stopped database): applied 2 then 0, schema 20261118104258 (122 rows), counts, names and old checksums unchanged,
`task_rule_assignees` 37 rows backfilled (37 rule audit rows), `suspended_workspace_id` present (0 non-null),
quick-check ok, 0 foreign-key rows; the old binary refuses the new schema; `rollback-schema.sql` gives the exact old
schema and counts, the old binary then starts (0 applied), and the new binary migrates again. Built-bundle check before
the switch (`pre/run.sh`: seeded instance in a network namespace, release binary's embedded app): `/`, `/tasks`, `/docs`,
`/chat`, `/settings/members` render with no errors; Manage → Suspend member on member@example.com: badge, its login 401
(right password), Resume member: badge gone, login 200 (`pre/shots/`).

Switch: stopped both services at 13:07:55 UTC; snapshot `01a1eec5-63a3-7059-a96c-c912d3263f00` with
`/usr/local/bin/orbit-8b614cc`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-6122f46.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-6122f46.tgz`; sidecar code copied (old `/opt/orbit-agent.before-6122f46`);
no env change. `orbit` ready 13:08:15, `orbit-agent` 13:08:16 (20 s down). `vm-health.conf` (old copy
`.before-6122f46`): `PROD_TAG=6122f46`, `ROLLBACK_TAG=8b614cc`, `ROLLBACK_BACKUP=01a1eec5-63a3-7059-a96c-c912d3263f00`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness (all 6 checks) local and on
`https://orbit-app.easytomanage.com/health/ready`; `/orbit-build.json` revision `6122f461…`; schema 20261118104258 (122
rows); sidecar `/health` ok, `/opt/orbit-agent/src` = `main`. `ORBIT_VERIFY_SPA=1 verify-production.sh` passed (signed
in and out, session logged out). Signed-in browser check (`ui-check.ts`, David Dusi, writes blocked, logged out 204):
Wellrox `/`, `/tasks`, `/docs`, `/chat`, `/settings/members` render; a member's Manage menu shows Suspend member (not
clicked); Slate task SLATE-9 shows "GitHub issue #33" → `https://github.com/easy2manage/crm-etm/issues/33`. Its only 2
console errors are the 2 writes the script blocked itself (page visit, chat read). `vm-health`: nginx WARN only from
the switch window (connection refused 13:08, 0 5xx); OOM WARN = 7 `gvisor_sentry` kills Nov 17, before this deploy;
docker WARN = other sessions' containers; git-owner fixed after.

After: Apple state checked first (ASC API): newest build 21, beta review `APPROVED` (build 20 too, 14-19 never
submitted), App Store version 1.0 `PREPARE_FOR_SUBMISSION`: nothing waits for Apple review. So
`apple-review@easytomanage.com` (member of Orbit Demo only) was suspended by David (Orbit Demo owner) in Settings →
Members → Manage → Suspend member (`suspend-apple.ts`; only that POST allowed): 204, Suspended badge, audit
`membership.suspended` 13:10:38 UTC, `suspended_workspace_id` = Orbit Demo, its 3 live sessions revoked (0 left). Login
answers `invalid_credentials` for a suspended account even with the right password (pre-switch check); the reviewer's
password is not stored here, so no login was tried on production. Changelog posted to #orbit-updates as David
(temporary session, logged out; message `01a1eec7-fdfb-71ad-b3b9-75f1ed8d62e5`; text
`/root/ops/suspend-labels/changelog.md`). Test instances :18475 (`/root/orbit-test/member-suspend`) and :18471
(`/tmp/lu-e2e`) stopped and removed. Screenshots: `/root/.local/share/com.jean.desktop/pasted-images/suspend-labels-deploy/`.
Scripts and logs: `/root/ops/suspend-labels/`.

Rollback to `8b614cc` (undo both migrations first: lost are which workspace suspended an account (the suspension
stays) and which assignees a rule added; the old binary refuses the new schema):

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit sqlite3 "file:/var/lib/orbit/orbit.sqlite?mode=ro" ".backup '/var/backups/orbit/pre-migration/orbit-before-rollback-to-8b614cc.sqlite'"
sudo -u orbit sqlite3 /var/lib/orbit/orbit.sqlite < /root/ops/suspend-labels/rollback-schema.sql   # DROP column + table, delete the 2 version rows
sudo install -m 755 /usr/local/bin/orbit-8b614cc /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-6122f46 /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# (the same as /root/ops/suspend-labels/rollback.sh)
# Way 2, only for a data problem (work after 13:07 UTC is lost), Orbit stopped: /usr/local/bin/orbit-8b614cc backup verify +
#   backup restore 01a1eec5-63a3-7059-a96c-c912d3263f00 (as user orbit), then the binary and sidecar lines above
```

After this rollback, set `PROD_TAG=8b614cc`, `ROLLBACK_TAG=b7c3122`,
`ROLLBACK_BACKUP=01a1ee03-14bc-776e-b995-9b93566df092` in `/root/ops/vm-health.conf` (copy
`/root/ops/vm-health.conf.before-6122f46`).

### November 18, 2026 upgrade: one approval per agent job, whole large file read, dark-theme mic hover fix (1 migration)

`main` from `ec747414` to `8b614cc4` (deployed): `921f237f` (fix: in the dark theme the listening mic of the Ask bar and
the composer turned gray under the pointer, found in the b7c3122 check; `data-[status=listening]:dark:hover:bg-destructive/90`
now wins over the ghost button's `dark:hover:bg-muted/50`; CSS only, no test), merge `8b614cc4` of `agent-batch-approval`
(`c0e68a73` one approval per job: `plan_job`/`run_job`, new routes `POST /api/v1/workspaces/{id}/agent-tools/jobs`,
`…/jobs/{job_id}/start`, `…/jobs/{job_id}/finish`, migration `20261117133309_agent_jobs` (new table + index only),
`Details` card component; `5f9550ea` job prompt; `277aee32` whole large file read in one script call). `aboutOrbit()`
already had both features (Jobs and Large files lines, the "What you can do" list). `b28ae38d` (sidecar only, after the
image): what's new moved the two lines to a `## 2026-11-18` entry with the mic fix, oldest entry (2026-11-10) dropped
(8 kept). No env key, no new sidecar package. As the user asked, NO real-model `plan_job`/token check was run.
Checks: agent `tsc` clean, `npm test` 111 pass / 0 fail / 11 skipped, `prompt.test.ts` 5 pass (after `b28ae38d`); web
`tsc -b` + build ok; mobile `tsc` ok; server `--lib agent::` 60 pass, `agent_drive` 8, `agent_integrations` 25,
`openapi_contract` 19 pass. Chromium on the built CSS: before the fix, dark hover `oklab(0.269 0 0 / 0.5)` (gray); after,
dark and light hover red.

Deploy id `20261118T085421Z-2833304`. `orbit:8b614cc` (image
`sha256:2cfbd3d1a05409b46c11c3013354b4f4db6316674200988bf3aa111c1ff87d22`, ~40 min, deps cached); `deploy/smoke.sh` and
`orbit config check` passed. Migration dry run (hot copy, and in the switch on a copy of the stopped database): applied
1 then 0, schema 20261117133309 (120 rows), counts, names and old checksums unchanged, `agent_jobs` + index present (0
rows), quick-check ok, 0 foreign-key rows; the old binary refuses the new schema; `rollback-schema.sql` gives the exact old
schema and counts, the old binary then starts (0 applied), and the new binary migrates again. Built-bundle check before
the switch (`pre/run.sh`: seeded instance in a network namespace, `GET /api/v1/agent` **stubbed**, fake microphone and
**fake recognizer**): Ask bar mic idle → listening → idle, hovered stop button red in the dark AND light theme; chat mic;
Home, Tasks, Docs, Chat, Settings → Agentic with no errors. New routes before the switch: 404.

Switch: stopped both services at 09:35:41 UTC; snapshot `01a1ee03-14bc-776e-b995-9b93566df092` with
`/usr/local/bin/orbit-b7c3122`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-8b614cc.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-8b614cc.tgz`; sidecar code copied (old `/opt/orbit-agent.before-8b614cc`);
no env change. `orbit` and `orbit-agent` ready 09:36:06 (25 s down). `vm-health.conf` (old copy `.before-8b614cc`):
`PROD_TAG=8b614cc`, `ROLLBACK_TAG=b7c3122`, `ROLLBACK_BACKUP=01a1ee03-14bc-776e-b995-9b93566df092`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness (all 6 checks) local and on
`https://orbit-app.easytomanage.com/health/ready`; `/orbit-build.json` revision `8b614cc4…`; schema 20261117133309 (120
rows); sidecar `/health` ok (sandbox true), `/opt/orbit-agent/src` = `main`; APNs on; `/`, `/chat`, `/tasks`, `/docs`,
`/settings/agentic` 200, `/api/v1/agent` 401 without a session; the 3 job routes 401 with a wrong secret and a valid body
(422 with an empty body, as the existing `agent-tools/call`), an unknown `agent-tools` path 404. Signed-in browser check
on production (`ui-check.ts`, temporary sessions for David Dusi, writes blocked, each logged out: 204, rows revoked;
real agent status: on, online, ChatGPT connected): Ask bar mic listening and hovered: red in the dark
(`oklab(0.704 0.177 0.072 / 0.9)`) and the light theme; Home, Chat, Tasks, Docs and Settings → Agentic render. Job
approval card: David's DM with Donna with ONE **stubbed** message added to the real messages GET (the question and
Details card as Orbit's `question()`/`details()` and the sidecar's `askForApproval` make them; nothing written): the
question, field mapping, left out, samples and a closed Details that opens on the scope and script. Its only console
error is the 404 of `GET …/messages/<stubbed id>/actions` (the message does not exist on the server): expected. The first
two runs failed on the stub itself (`route.fetch()` skips Chromium's resolver rule; a ui block needs `block_id`).
Screenshots in `/root/.local/share/com.jean.desktop/pasted-images/batch-approval-deploy/`
(`prod-ask-bar-mic-listening-hover-dark.png`, `prod-job-approval-card*.png`). `vm-health`: nginx FAIL only from the switch
window (50 connection-refused lines 09:35-09:36, 0 5xx); OOM WARN = 7 `gvisor_sentry` kills Nov 17 14:28-14:36, before
this deploy; docker WARN = other sessions' containers; git-owner fixed after. Changelog posted to #orbit-updates as David
Dusi (temporary session, logged out: 204; message `01a1ee08-358d-70b2-8fe9-cc12360b92c5`; text
`/root/ops/batch-approval/changelog.md`). Scripts and logs: `/root/ops/batch-approval/`.

Rollback to `b7c3122` (undo the migration first: only the agent jobs are lost; the old binary refuses the new schema):

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit sqlite3 "file:/var/lib/orbit/orbit.sqlite?mode=ro" ".backup '/var/backups/orbit/pre-migration/orbit-before-rollback-to-b7c3122.sqlite'"
sudo -u orbit sqlite3 /var/lib/orbit/orbit.sqlite < /root/ops/batch-approval/rollback-schema.sql   # DROP agent_jobs + index, delete the 20261117133309 row
sudo install -m 755 /usr/local/bin/orbit-b7c3122 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-8b614cc /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# (the same as /root/ops/batch-approval/rollback.sh)
# Way 2, only for a data problem (work after 09:35 UTC is lost), Orbit stopped: /usr/local/bin/orbit-b7c3122 backup verify +
#   backup restore 01a1ee03-14bc-776e-b995-9b93566df092 (as user orbit), then the binary and sidecar lines above
```

After this rollback, set `PROD_TAG=b7c3122`, `ROLLBACK_TAG=c21ec38`,
`ROLLBACK_BACKUP=01a1ea44-329a-724e-a09e-a20d48520a49` in `/root/ops/vm-health.conf` (copy
`/root/ops/vm-health.conf.before-8b614cc`).

### November 17, 2026 upgrade: the mic in the Ask bar, recording states, desktop 0.1.7-0.1.10 web parts (no migration)

`main` from `c21ec383` to `b7c3122c` (deployed; 38 commits, all on `origin/main` `bf30e05c` except `b7c3122c`). Server-side
the change is the embedded web app and the sidecar's `prompt.ts`/`whats-new.md`; no Rust server code, no migration, no
env key, no new route. Web: `03d57172` (0.1.7: the recognizer's `message` in the toast), `74777d3a` (0.1.8: account
menu → Desktop app with the tray items, Notifications page text in the app), `25e535d7` (0.1.9 text), `835fc734`
(0.1.10: the mic in the small Ask bar; while it listens the mic is red with a stop square, a red timer, a level meter in
the Orbit app and the placeholder "Listening… click the mic to stop"; "Transcribing…" after stop). The iPhone commits
(`776dbdcf` … `77f1310e`) and `scripts/seed-review-demo.ts` do not change the server. `fc4a2428` and `2fdcc97d` were already
live (in `c21ec383`). `b7c3122c` (this deploy): `aboutOrbit()` says the recording states show in every browser (they were
only in the Windows-app text), names the mic of the small Ask box, and drops "this menu item shows after the server's
next web update"; what's new 2026-11-17: the voice input line names the Ask box mic, the red timer, desktop 0.1.10 and the
account menu. Checks: agent `tsc` clean, `prompt.test.ts` 5 pass (Node 24).

Deploy id `20261117T152846Z-2658848`. `orbit:b7c3122` (image
`sha256:b6101d5053c834fc9cd5bba78e2d8309ee14d3a81e58f165c6465848062aa254`, ~35 min); `deploy/smoke.sh` and
`orbit config check` passed. No-migration check (hot copy, and in the switch on a copy of the stopped database): 0
applied, schema 20261115105242 (119 rows), counts, names and checksums unchanged, quick-check ok, 0 foreign-key rows.
Built-bundle check before the switch (`pre/run.sh`: seeded dev instance in a network namespace, port 18932; `GET
/api/v1/agent` **stubbed** to "agent on, ChatGPT connected", because the seed has no agent; Chromium with a fake
microphone and a **fake speech recognizer** that types "Hello Donna"): the Ask bar shows one mic (idle), a click gives the
stop button, "Listening 0:01" with the level meter and the words in the box, a second click goes back to idle; the chat
mic renders; Home, Tasks, Docs, Chat and Settings → Agentic with no page or console errors. Headless Chromium without the
microphone permission shows the mic crossed out ("Microphone access is off"): expected.

Switch: stopped both services at 16:08:20 UTC; snapshot `01a1ea44-329a-724e-a09e-a20d48520a49` with
`/usr/local/bin/orbit-c21ec38`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-b7c3122.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-b7c3122.tgz`; sidecar code copied (old `/opt/orbit-agent.before-b7c3122`);
no env change. `orbit` and `orbit-agent` ready 16:08:44 (24 s down). `vm-health.conf` (old copy `.before-b7c3122`):
`PROD_TAG=b7c3122`, `ROLLBACK_TAG=c21ec38`, `ROLLBACK_BACKUP=01a1ea44-329a-724e-a09e-a20d48520a49`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness (all 6 checks) local and on
`https://orbit-app.easytomanage.com/health/ready`; `/orbit-build.json` revision `b7c3122c…`; sidecar `/health` ok
(sandbox true), `/opt/orbit-agent/src` = `main`; APNs on; `/`, `/chat`, `/tasks`, `/docs`, `/settings/agentic` 200,
`/api/v1/agent` 401 without a session. Signed-in browser check on production (`ui-check.ts`, temporary session for
David Dusi, writes blocked, logged out after: 204, row revoked; real agent status: on, online, ChatGPT connected; the
same fake microphone and recognizer, nothing sent): the Ask bar mic idle, listening ("Listening 0:01", level meter,
"Hello Donna" in the box) and back to idle; chat mic; the MCP card; Home, Chat, Tasks, Docs and Settings → Agentic with
no page or console errors. Screenshots in
`/root/.local/share/com.jean.desktop/pasted-images/server-deploy-20261117/` (`prod-ask-bar-mic-*.png`). Found: in the dark
theme the listening mic's red fill turns gray while the pointer is on it (`dark:hover:bg-muted/50` of the ghost button
wins over `data-[status=listening]:hover:bg-destructive/90`); red again when the pointer moves away; not fixed here.
`vm-health`: nginx FAIL only from the switch window (76 connection-refused lines at 16:08, 0 5xx); OOM WARN = 7
`gvisor_sentry` kills 14:28-14:36 UTC, before this deploy; docker WARN = other sessions' containers; git-owner fixed
after (only files newer than this deploy). Desktop downloads not touched. Changelog posted to #orbit-updates as David
Dusi (temporary session, logged out: 204; message `01a1ea47-5efb-76bb-9e95-5533a3433a1e`; text `/root/ops/ask-mic/changelog.md`).
Scripts and logs: `/root/ops/ask-mic/`.

Rollback to `c21ec38` (no schema change, no env change, so no data is lost):

```sh
sudo systemctl stop orbit-agent orbit
sudo rm -rf /opt/orbit-agent/src && sudo cp -a /opt/orbit-agent.before-b7c3122/src /opt/orbit-agent/src
sudo install -m 755 /usr/local/bin/orbit-c21ec38 /usr/local/bin/orbit
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# (the same as /root/ops/ask-mic/rollback.sh)
# Way 2, only for a data problem (work after 16:08 UTC is lost), Orbit stopped: /usr/local/bin/orbit-c21ec38 backup verify +
#   backup restore 01a1ea44-329a-724e-a09e-a20d48520a49 (as user orbit), then the lines above
```

After this rollback, set `PROD_TAG=c21ec38`, `ROLLBACK_TAG=1bfcece`,
`ROLLBACK_BACKUP=01a1e95a-531b-70bf-9d43-5b9e46902c26` in `/root/ops/vm-health.conf` (copy
`/root/ops/vm-health.conf.before-b7c3122`).

### November 17, 2026 iPhone build 20: iPhone full audit and the Agentic tabs (TestFlight internal; no server change)

`main` from `fbc88d7a` to `cc4080c5`: `--no-ff` merges of `iphone-agentic-tabs` at `6d13977a` (`fbf0d9ff`: the
build 19 fix, Agentic tabs hide only on a known "off") and `iphone-full-audit` at `4ccf55e6` (`cc4080c5`). No merge
conflict. The audit (`.ai/iphone-audit.md`) adds: task title and description editing, Project and Repeat on a task,
comment Reply/Edit/Delete; Docs comment Resolve/Reopen/Delete thread and Edit/Delete comment, Drive file blocks in
pages; Switch workspace in Settings, New project in the project menu, project label rules; Account → Security lists and
removes passkeys; chat profile cards, channel Integrations (incoming webhooks) and link preview cards; saved views,
Home layout, apps, agent pages and stickers update live. `aboutOrbit()` already has all of it (`51751b5a`); the
Agentic fix matches its "same pages and roles as on the web".

Checks on the merged `main`: mobile `tsc --noEmit` clean, `bun test src` 343 pass (55 files), agent `tsc --noEmit`
clean. No new native module; `eas build --non-interactive --no-wait` from the main checkout.

EAS build `f92fb858-74a2-4a73-8e08-f7b0c4366535`, build number 20 (version 1.0.0, commit `cc4080c5`, ~7 min).
Submission `96941aab-c7bb-4889-a43c-7973bd05501e` with the App Manager ASC key (~35 min in the submission queue;
eas.json changed only for the run and restored; the temporary `AuthKey.p8` deleted). ASC build
`4d4bc895-2d2c-4d99-8492-38ba51744740`: `VALID`, internal state `IN_BETA_TESTING`, external
`READY_FOR_BETA_SUBMISSION` (not submitted). Scripts and logs: `/root/ops/iphone-build20/`.

### November 17, 2026 iPhone build 19: Settings → Agentic shows the web's tabs (TestFlight internal; no server change)

Branch `iphone-agentic-tabs` at `abef777e` (on `main` `fbc88d7a`; not merged, not pushed by the build session). Only
`apps/mobile` changes: the Agentic tabs (Workspace, Agents, Routines, Skills) hide only when `/api/v1/agent` says
`enabled: false`; before, a slow or failed answer left only "You" with no message, and build 17's Account → Agentic
always forced "You". Skills shows for every role (members get the note, as on the web); Retry on the agent status and
on memories; Connected apps say "All projects". Production was checked read-only: the agent is set up, the app gateway
proxies `/api/v1/agent` (401 without a session), the sidecar answers.

Checks: mobile `tsc --noEmit` clean, `bun test src` 336 pass (54 files), the iOS bundle builds on Metro; Expo web at
390x844 on an isolated test instance with the agent on: all five tabs, and with `/api/v1/agent` failing the tabs stay
and Retry loads the card. No new native module, but a full native build as for 18: `eas build --non-interactive
--no-wait` from the worktree.

EAS build `87141e30-3857-4a90-96a2-ad659c51388b`, build number 19 (version 1.0.0, commit `abef777e`, ~5 min).
Submission `3b394d37-7ce5-490f-9500-125cb014910c` with the App Manager ASC key (~30 min in the submission queue;
eas.json changed only for the run and restored; the temporary `AuthKey.p8` deleted). ASC build
`6fe8a715-74d4-441e-8baa-915981b12615`: `VALID`, internal state `IN_BETA_TESTING`, external
`READY_FOR_BETA_SUBMISSION` (not submitted). Scripts and logs: `/root/ops/iphone-build19/`.

### November 17, 2026 iPhone build 18: one Settings page, one Search screen (TestFlight internal; no server change)

`main` from `c56fe1a3` to `a27c6a88`: `--no-ff` merges of `iphone-settings-b18` at `776dbdcf` (`1faa6adf`: one
Settings page with Workspace, Account and Admin; one Agentic page with a copyable MCP address via `expo-clipboard`) and
`iphone-ux` at `409b873c` (`a27c6a88`: one Search screen, a Tasks view menu, one project settings page; Docs and Drive
tidy-up), no conflicts; not pushed by the build session. Only `apps/mobile`, `bun.lock` and `.ai/UI_CONVENTIONS.md`
change. No server code, no migration, no env key, no server deploy.

Checks on the merge (main checkout): `bun install` (adds `expo-clipboard@58.0.4`), mobile `tsc --noEmit` clean,
`bun test src` 336 pass (54 files). `expo-clipboard` is a new native module (no capability), so a full native build:
`eas build --non-interactive --no-wait` (no pty driver), no OTA update.

EAS build `bdb72cf7-5afc-4a1d-986c-8dc57831aba2`, build number 18 (version 1.0.0, commit `a27c6a88`, ~6 min). The
remote build counter gave 18: the branches "build 18" and "build 19" ship together in this one build. Submission
`6979a699-3e0c-413e-a415-23ad8ad338e1` with the App Manager ASC key (~16 min in the submission queue; eas.json changed
only for the run and restored; the temporary `AuthKey.p8` deleted). ASC build `1c22dda6-be01-4684-912a-2b323f722640`:
`VALID` ~2 min after the upload, internal state `IN_BETA_TESTING` (group "Team (Expo)", internal, all builds),
external `READY_FOR_BETA_SUBMISSION` (not submitted). Scripts and logs: `/root/ops/iphone-b19/`.

### November 17, 2026 upgrade: Settings → Agentic MCP address and the web mic fix (no migration)

`main` from `22ff461e` to `c21ec383` (deployed): `12a64e09` (merge of `origin/main` `fc4a2428`: desktop 0.1.6 and the web
mic fix: after `not-allowed` the dictation checks the microphone with `getUserMedia`, retries once, and the toast names
the cause, the error text and the app version), `5e5241dc` (`--no-ff` merge of `agentic-mcp-address` at `2fdcc97d`; no
conflicts: `GET /api/v1/oauth/grants` returns `mcp_url` (`<ORBIT_PUBLIC_MCP_URL>/mcp` or null), and Settings → Agentic
(web and iPhone) shows that address, the Codex and Claude Code commands and a link to `docs/connect-your-agent.md`),
`c21ec383` (what's new 2026-11-17: the MCP server card line, moved from the 2026-11-16 entry, and a voice input line).
`aboutOrbit()` already had both changes. After the switch, `origin/main` got `03d57172` (desktop 0.1.7; web: the
recognizer's `message` in the toast; `aboutOrbit()` text): merged into `main` as `3e595aff` before the push, **not
deployed** (production and the sidecar run `c21ec38`).

Checks on `c21ec383`: `packages/api` tsc and `mcp.test.ts` (4 pass), web `tsc` and `voiceInput.test.ts` (16 pass), mobile
`tsc --noEmit` clean, agent `tsc` and `prompt.test.ts` on Node 24 (5 pass), `cargo test -p orbit-server --test oauth_api`
(17 pass; lean target dir removed after). After the `03d57172` merge: web `tsc -b`, `voiceInput.test.ts` (17 pass),
`prompt.test.ts` (5 pass). Logs: `/root/ops/agentic-mcp/`.

Deploy id `20261117T105105Z-2460901`. `orbit:c21ec38` (image
`sha256:ce8197d59b52b44e51d93e2fcbe1621f04de8426143c4bf65d79871340c37e35`, ~40 min); `deploy/smoke.sh` and
`orbit config check` passed. No-migration check (hot copy, and in the switch on a copy of the stopped database): 0
applied, schema 20261115105242 (119 rows), counts, names and checksums unchanged, quick-check ok, 0 foreign-key rows.
Built-bundle check before the switch (`pre/run.sh`: seeded dev instance in a network namespace with
`ORBIT_PUBLIC_MCP_URL=https://orbit-hooks.easytomanage.com`): `mcp_url` and the card show
`https://orbit-hooks.easytomanage.com/mcp`, the Codex command and the guide link; the chat mic button renders (crossed
out: headless Chromium has no microphone); no page or console errors.

Switch: stopped both services at 11:52:53 UTC; snapshot `01a1e95a-531b-70bf-9d43-5b9e46902c26` with
`/usr/local/bin/orbit-1bfcece`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-c21ec38.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-c21ec38.tgz`; sidecar code copied (old `/opt/orbit-agent.before-c21ec38`);
no env change. `orbit` and `orbit-agent` ready 11:53:06 (13 s down). `vm-health.conf` (old copy `.before-c21ec38`):
`PROD_TAG=c21ec38`, `ROLLBACK_TAG=1bfcece`, `ROLLBACK_BACKUP=01a1e95a-531b-70bf-9d43-5b9e46902c26`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness (all 6 checks) local and on
`https://orbit-app.easytomanage.com/health/ready`; `/orbit-build.json` revision `c21ec383…`; sidecar `/health` ok; APNs on.
Signed-in browser check on production (`ui-check.ts`, temporary session for David Dusi, writes blocked, logged out after:
204, row revoked): Home, Chat, Tasks, Docs and Settings → Agentic render with no page or console errors; the MCP server
card shows `https://orbit-hooks.easytomanage.com/mcp` with the Codex and Claude Code commands and the guide link; the chat
mic button renders. Screenshot
`/root/.local/share/com.jean.desktop/pasted-images/connect-your-agent/agentic-mcp-card-prod.png`. `vm-health` OK except
nginx (connection-refused lines of the switch at 11:53; 0 5xx) and git-owner (fixed after). Desktop 0.1.6 downloads not
touched.

Rollback to `1bfcece` (no schema change, no env change, so no data is lost):

```sh
sudo systemctl stop orbit-agent orbit
sudo rm -rf /opt/orbit-agent/src && sudo cp -a /opt/orbit-agent.before-c21ec38/src /opt/orbit-agent/src
sudo install -m 755 /usr/local/bin/orbit-1bfcece /usr/local/bin/orbit
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# (the same as /root/ops/agentic-mcp/rollback.sh)
# Way 2, only for a data problem (work after 11:52 UTC is lost), Orbit stopped: /usr/local/bin/orbit-1bfcece backup verify +
#   backup restore 01a1e95a-531b-70bf-9d43-5b9e46902c26 (as user orbit), then the lines above
```

After this rollback, set `PROD_TAG=1bfcece`, `ROLLBACK_TAG=4ba4e2e`,
`ROLLBACK_BACKUP=01a1e8cf-d020-74cb-91b0-bf8e8443df3c` in `/root/ops/vm-health.conf` (copy
`/root/ops/vm-health.conf.before-c21ec38`).

### November 17, 2026 iPhone build 17: iPhone parity (TestFlight internal; no server change)

`main` from `40d14b9c` to `e8c0e2e4` (`--no-ff` merge of `iphone-parity-all` at `99ffac06`, no conflicts; not pushed).
The branch joins 4 iPhone branches: live data (chat, threads, unread counts, app icon number, Tasks, Activity, Docs,
Drive update by the `/live` and `/events` sockets; offline banner, optimistic send and task status with rollback) and a
disk cache of queries (`@tanstack/query-async-storage-persister`, `react-query-persist-client`, `expo-network`); chat
parity (@ and # pickers, mentions notify, Forward, share link, task from message, Send later and scheduled messages,
stickers, search with all web filters); tasks/docs/Drive parity (saved views, full filters, List or Board, swipe for
done or trash, task trash; Drive trash, Move to, share links, search and sort; Docs Version history, Lock, Move to,
Duplicate, Export, Notion import); settings parity (every web settings page, create or join a workspace, workspace apps
in the ☰ menu, Suggest a change and Admin → Feedback & fixes). Also `apps/agent/src/prompt.ts` and `whats-new.md`
(the `2026-11-17` entry gets the iPhone app line; it shows to Donna from the next sidecar deploy). No server code, no
migration, no env key; live updates need the socket fix that is live since `1bfcece`.

Checks on the merge (main checkout): `bun install --frozen-lockfile`, mobile `tsc --noEmit` clean, `bun test src` 326
pass (53 files). Native change since build 16 (`a900002e`): only `expo-network` (no capability), so
`eas build --non-interactive --no-wait` (no pty driver).

EAS build `dccbaf17-4b25-4d3c-9da0-fa63d6c5b782`, build number 17 (version 1.0.0, commit `e8c0e2e4`, ~5 min). Submission
`783f4e6c-225d-45e8-84c7-27fa10825ad4` with the App Manager ASC key (eas.json changed only for the run and restored;
the temporary `AuthKey.p8` deleted). ASC build `9a0a45c5-95a2-4e70-9a0b-8519c7ddf1e3`: `VALID` ~2 min after the
upload, internal state `IN_BETA_TESTING` (group "Team (Expo)", internal, all builds), external
`READY_FOR_BETA_SUBMISSION` (not submitted; no beta review, no external testers). Scripts and logs:
`/root/ops/iphone-b17/`.

### November 17, 2026 upgrade: iPhone live sockets from their own host (no migration)

`main` from `f67b0f4c` to `1bfcecec` (fast-forward, pushed): `f00ff330` (`is_native_own_origin` in
`crates/platform/src/http/security.rs`: a request with a bearer secret, no cookie and an Origin equal to its own `Host`
passes the Origin check, so the iPhone app's `/live` and `/events` sockets on `orbit-app.easytomanage.com` are no longer
403), `e959fdc2` (lesson), `1bfcecec` (what's new 2026-11-17: "iPhone: open chats, tasks and lists update live again; no
app update needed."; the 2026-11-09 entry dropped to keep 8). Also first deployed here: `f67b0f4c` (desktop 0.1.5: Windows
voice input; server side only `aboutOrbit()` in `prompt.ts` and docs) and `3f2cc5c3` (docs). `aboutOrbit()` unchanged by
this fix (bug fix, no new feature).

Checks: platform `http::security` 13 pass, `cargo clippy -p orbit-platform --all-targets -D warnings` clean, agent
`prompt.test.ts` on Node 24 (5 pass). Logs: `/root/ops/iphone-live/`.

Deploy id `20261117T083525Z-2399933`. `orbit:1bfcece` (image
`sha256:854480cc702b1a25ab5c9d4fc0c03f9f35a30b3c2b66b95d4436600819841a23`, release compile 36 min); `deploy/smoke.sh` and
`orbit config check` passed. No-migration check (hot copy, and in the switch on a copy of the stopped database): 0
applied, schema 20261115105242 (119 rows), counts, names and checksums unchanged, quick-check ok, 0 foreign-key rows.

Switch: stopped both services at 09:21:35 UTC; snapshot `01a1e8cf-d020-74cb-91b0-bf8e8443df3c` with
`/usr/local/bin/orbit-4ba4e2e`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-1bfcece.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-1bfcece.tgz`; sidecar code copied (old `/opt/orbit-agent.before-1bfcece`);
no env change. `orbit` and `orbit-agent` ready 09:21:52 (17 s down). `vm-health.conf` (old copy `.before-1bfcece`):
`PROD_TAG=1bfcece`, `ROLLBACK_TAG=4ba4e2e`, `ROLLBACK_BACKUP=01a1e8cf-d020-74cb-91b0-bf8e8443df3c`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness (all 6 checks) local and on
`https://orbit-app.easytomanage.com/health/ready`; `/orbit-build.json` revision `1bfcecec…`; sidecar `/health` ok.
WebSocket upgrades (`/root/ops/iphone-live/ws-check.sh`, temporary bearer session for David Dusi, logged out after: 204,
then 401), before → after: `/live` and `/events` with bearer + `Origin: https://orbit-app.easytomanage.com` via the public
iPhone host 403 → 101; bearer + no Origin 101 → 101; bearer + foreign Origin 403 → 403; bearer + cookie + own Origin
403 → 403; cookie + foreign Origin on the web host 403 → 403; cookie + web Origin 101 → 101. `vm-health` OK except nginx
(41 connection-refused lines of the switch at 09:21; 0 5xx), the OOM WARN (2 `gvisor_sentry` kills on November 16 at
10:47 and 11:01, before this deploy) and git-owner (fixed after).
Desktop downloads: Desktop run [44047671708](https://github.com/DaveDushi/orbit/actions/runs/44047671708) dispatched
on `f67b0f4c` (test, windows and manifest passed, macos skipped); `deploy/desktop-downloads.sh 44047671708` (09:03 UTC,
under this deploy's lock) put `Orbit-0.1.5-windows-x64-setup.exe` (sha256 `cc421040…`, 3,476,873 bytes),
`Orbit-0.1.5-windows-x64.zip` (`eb885163…`) and `latest.json` (version 0.1.5, Windows only) in
`/srv/orbit-downloads/desktop/` and removed the 0.1.4 files; the three files are byte-equal to the run's artifacts.
Through the local nginx: `latest.json` 200 `application/json`, `no-cache`; installer 200 (PE32 GUI, sha256 of
`latest.json`), zip 200, 0.1.4 installer 404. **Not** verified: the installer and voice input on a real Windows PC.

Rollback to `4ba4e2e` (no schema change, no env change, so no data is lost):

```sh
sudo systemctl stop orbit-agent orbit
sudo rm -rf /opt/orbit-agent/src && sudo cp -a /opt/orbit-agent.before-1bfcece/src /opt/orbit-agent/src
sudo install -m 755 /usr/local/bin/orbit-4ba4e2e /usr/local/bin/orbit
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# (the same as /root/ops/iphone-live/rollback.sh)
# Way 2, only for a data problem (work after 09:21 UTC is lost), Orbit stopped: /usr/local/bin/orbit-4ba4e2e backup verify +
#   backup restore 01a1e8cf-d020-74cb-91b0-bf8e8443df3c (as user orbit), then the lines above
```

After this rollback, set `PROD_TAG=4ba4e2e`, `ROLLBACK_TAG=d8d3eb4`,
`ROLLBACK_BACKUP=01a1e5b7-933d-76bd-958f-622b84d25183` in `/root/ops/vm-health.conf` (copy
`/root/ops/vm-health.conf.before-1bfcece`).

### November 16, 2026 upgrade: iPhone push with the agent's answer (no migration)

`main` from `298840ef` to `4ba4e2ef` (local, not pushed): `--no-ff` merge `c674f683` of `iphone-push-latency`
(`201e9669` on `05348027`): the agent's "⏳ Working on it…" post is `silent` (no push), and the answer edit (and a task
run's done line) sends `notify`, so the push comes with the answer. APNs logs `APNs took the iPhone push` / `APNs refused
the iPhone push` with the `apns_id` and `alert`. No conflicts (`agent.ts`, `prompt.ts` auto-merged; `aboutOrbit()` keeps
main's iPhone line plus the new sentence). Then `4ba4e2ef`: what's new 2026-11-16 (the iPhone line of today's entry, as
the entry has 4 lines). A new sidecar on an older server gets 422 on `silent`/`notify`: server and sidecar switch together.

Checks: `agent_api` 22 and `push_devices_api` 7 on SQLite and on Postgres (0 warnings); agent `tsc --noEmit` and
`npm test` on Node 24 (103 pass, 11 skipped), `prompt.test.ts` again after the what's-new line (5 pass). Logs:
`/root/ops/iphone-push/`.

Deploy id `20261116T175933Z-2323610`. `orbit:4ba4e2e` (image
`sha256:35117e47e985986bbadbfb49f7e199074fefde461d12d7ab57c450a5e4f541d9`, release compile 34.5 min);
`deploy/smoke.sh` and `orbit config check` passed. No-migration check (hot copy, and in the switch on a copy of the
stopped database): 0 applied, schema 20261115105242 (119 rows), counts, names and checksums unchanged, quick-check ok, 0
foreign-key rows.

Switch: stopped both services at 18:56:15 UTC; snapshot `01a1e5b7-933d-76bd-958f-622b84d25183` with
`/usr/local/bin/orbit-d8d3eb4`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-4ba4e2e.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-4ba4e2e.tgz`; sidecar code copied (old `/opt/orbit-agent.before-4ba4e2e`);
no env change. `orbit` ready 18:56:31, then `orbit-agent` (16 s down). `vm-health.conf` (old copy `.before-4ba4e2e`):
`PROD_TAG=4ba4e2e`, `ROLLBACK_TAG=d8d3eb4`, `ROLLBACK_BACKUP=01a1e5b7-933d-76bd-958f-622b84d25183`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness (all 6 checks) local and on
`https://orbit-app.easytomanage.com/health/ready`, `/health/live` 200; `/orbit-build.json` revision `4ba4e2ef…`. Donna
test (temporary session for David Dusi, logged out after, 204, session revoked): one question in his DM with Donna
(Wellrox) at 18:57:17; the answer "2 + 2 = 4." came as the edit at 18:57:21. Journal: at 18:57:17 only a badge push
(`alert: false`, David's own send), at 18:57:21 `APNs took the iPhone push` (`alert: true`, apns-id
`A6A7F40B-C73C-171D-EDED-6A208A746B8A`) and `push sent` (1 browser, 1 iPhone). Sidecar journal empty (no 422).
`vm-health` OK except nginx (54 connection-refused lines of the switch at 18:56; 0 5xx) and the OOM WARN (2 kills of the
`sandboxspike.slice` test at 10:47 and 11:01, before this deploy).

Rollback to `d8d3eb4` (no schema change, no env change, so no data is lost):

```sh
sudo systemctl stop orbit-agent orbit
sudo rm -rf /opt/orbit-agent/src && sudo cp -a /opt/orbit-agent.before-4ba4e2e/src /opt/orbit-agent/src
sudo install -m 755 /usr/local/bin/orbit-d8d3eb4 /usr/local/bin/orbit
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# (the same as /root/ops/iphone-push/rollback.sh)
# Way 2, only for a data problem (work after 18:56 UTC is lost), Orbit stopped: /usr/local/bin/orbit-d8d3eb4 backup verify +
#   backup restore 01a1e5b7-933d-76bd-958f-622b84d25183 (as user orbit), then the lines above
```

After this rollback, set `PROD_TAG=d8d3eb4`, `ROLLBACK_TAG=3acc586`,
`ROLLBACK_BACKUP=01a1e57a-7e8d-74d2-98b2-9e3de6c53164` in `/root/ops/vm-health.conf` (copy
`/root/ops/vm-health.conf.before-4ba4e2e`).

### November 16, 2026 upgrade: Donna prompt dedup, rmcp 3.6.0 and the MCP public origin (no migration)

`main` from `05348027` to `d8d3eb41` (local, not pushed): `--no-ff` merges `1604032a` of `donna-prompt-dedup`
(`76ef1767`: the `codemode` prompt section no longer repeats the tools Donna is offered directly, ~2.7k fewer tokens
per request), `e8854ff0` of `rmcp-3-6-evals` (`6e67ded2`: rmcp 3.5.0 → 3.6.0 for RUSTSEC-2026-0461, eval fixes) and
`9b0e93e4` of `mcp-public-origin` (`c66bb238`: optional `ORBIT_PUBLIC_MCP_URL`). Only conflict: `whats-new.md` (the
branch's `## TBD` entry joined today's entry; the webhook and iPhone lines are now one line, so it keeps 4 lines).
rmcp stays 3.6.0 (the MCP branch did not touch `Cargo.toml`/`Cargo.lock`). Then `d8d3eb41`: the donna-prompt eval case
c32 sends `drive_trash` `node_ids` (the list form on `main` since `0caa95a2`; with the old `node_id` it failed 34/35).
`aboutOrbit()` (API tokens line) and what's new (2026-11-16, "Connect your own agent to Orbit") describe the MCP
public origin.

Checks: `cargo check --workspace --all-targets` (0 warnings); `oauth_api` 17, `mcp_api` 19, `agent_api` 21 on SQLite
and on Postgres; `orbit-platform` config tests 6; agent `tsc` and `npm test` (102 pass, 11 skipped); Donna eval
`evals/donna-prompt` 35/35 (28,093 fixed tokens per request, `codemode` section 237). Not run: the Codex CLI OAuth
e2e (done on the branch, 2026-11-16). Logs: `/root/ops/combined-1116/`.

Deploy id `20261116T163334Z-2275347`. `orbit:d8d3eb4` (image
`sha256:1e986ef24d2d59adfd0972c0df4b47f91110934e3c9fe0d99826a2749c335d9a`, ~40 min); `deploy/smoke.sh` and
`orbit config check` passed. No-migration check (hot copy, and in the switch on a copy of the stopped database): 0
applied, schema 20261115105242 (119 rows), counts, names and checksums unchanged, quick-check ok, 0 foreign-key rows.

Switch: stopped both services at 17:49:32 UTC; snapshot `01a1e57a-7e8d-74d2-98b2-9e3de6c53164` with
`/usr/local/bin/orbit-3acc586`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-d8d3eb4.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-d8d3eb4.tgz`; sidecar code copied (old `/opt/orbit-agent.before-d8d3eb4`);
env: `ORBIT_PUBLIC_MCP_URL=https://orbit-hooks.easytomanage.com` added (old file `/etc/orbit/orbit.env.before-d8d3eb4`).
`orbit` ready 17:49:47, then `orbit-agent` (15 s down). `vm-health.conf` (old copy `.before-d8d3eb4`):
`PROD_TAG=d8d3eb4`, `ROLLBACK_TAG=3acc586`, `ROLLBACK_BACKUP=01a1e57a-7e8d-74d2-98b2-9e3de6c53164`.

Verified: `orbit`, `orbit-agent`, nginx, hooks gateway active, 0 restarts; readiness (all 6 checks) local and on
`https://orbit-app.easytomanage.com/health/ready`, `/health/live` 200; `/orbit-build.json` revision `d8d3eb41…`;
`orbit config show` lists `ORBIT_PUBLIC_MCP_URL`. Main origin: OAuth metadata issuer `https://orbit.easytomanage.com`,
`POST /mcp` without a token 401 with its `resource_metadata`. MCP origin (to `127.0.0.1:8080` with
`Host: orbit-hooks.easytomanage.com`, as the gateway proposal sends it): issuer and register/token/revoke on
`https://orbit-hooks.easytomanage.com`, authorize on `orbit.easytomanage.com`, protected resource
`https://orbit-hooks.easytomanage.com/mcp`, `POST /mcp` 401 without a token and 401 with a non-OAuth bearer. Through
the hooks gateway today these paths are 404 (Ops has not changed it yet). Journal: only rmcp's "allowed origin
without a port matches any port" WARN at start (once per origin; the old binary wrote it too). `vm-health` OK except
nginx (61 connection-refused lines of the switch at 17:49; 0 5xx).

Rollback to `3acc586` (no schema change, so no data is lost):

```sh
sudo systemctl stop orbit-agent orbit
sudo cp -p /etc/orbit/orbit.env.before-d8d3eb4 /etc/orbit/orbit.env
sudo rm -rf /opt/orbit-agent/src && sudo cp -a /opt/orbit-agent.before-d8d3eb4/src /opt/orbit-agent/src
sudo install -m 755 /usr/local/bin/orbit-3acc586 /usr/local/bin/orbit
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# (the same as /root/ops/combined-1116/rollback.sh)
# Way 2, only for a data problem (work after 17:49 UTC is lost), Orbit stopped: /usr/local/bin/orbit-3acc586 backup verify +
#   backup restore 01a1e57a-7e8d-74d2-98b2-9e3de6c53164 (as user orbit, with the old /etc/orbit/orbit.env), then the lines above
```

After this rollback, set `PROD_TAG=3acc586`, `ROLLBACK_TAG=dc59487`,
`ROLLBACK_BACKUP=01a1e531-d917-75f7-92d7-cf225067a4de` in `/root/ops/vm-health.conf` (copy
`/root/ops/vm-health.conf.before-d8d3eb4`).

### November 16, 2026 upgrade: desktop 0.1.4 merge, web/server part (no migration)

`main` from `a900002e` to `3acc5868` (pushed): `--no-ff` merge `25146515` of `voice-input-desktop` (`88cd0fc3`, no
conflicts): the "Microphone access is off" toast names the Windows switch in the Windows app, macOS settings in the
Mac app, site settings in a browser; `aboutOrbit()` names the Windows switch and desktop 0.1.4; desktop app 0.1.4
(WebView2 microphone grant for the server's origin). Then `3acc5868`: the app loads a saved `orbit.wellrox.com`
server as `orbit.easytomanage.com` (the 301 since 2026-11-12 made the app's pages "not the server's": no update check,
notifications, badge or mic grant there); the Desktop workflow's default server is `orbit.easytomanage.com` (tag runs
wrote wellrox links into `latest.json`). What's new: the 2026-11-16 voice-input line names the Windows app 0.1.4.

Checks: desktop `cargo check` and `clippy -D warnings` for `x86_64-pc-windows-gnu`, `cargo test --workspace` (all
pass); web `bun test` composer (31), `tsc -b`, oxlint (0 errors); agent `prompt.test.ts` (5). Logs: `/root/ops/desktop-014/`.

Deploy id `20261116T155136Z-2262356`. `orbit:3acc586` (image
`sha256:7a6ae57d47b354ece3c99d6bf82c030999acab8340b7e929d72937c38a19243c`, ~35 min); `deploy/smoke.sh` and
`orbit config check` passed. No-migration check (hot copy, and in the switch on a copy of the stopped database): 0
applied, schema 20261115105242 (119 rows), counts, names and checksums unchanged, quick-check ok, 0 foreign-key rows.

Switch: stopped both services at 16:30:11 UTC; snapshot `01a1e531-d917-75f7-92d7-cf225067a4de` with
`/usr/local/bin/orbit-dc59487`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-3acc586.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-3acc586.tgz`; sidecar code copied (old `/opt/orbit-agent.before-3acc586`);
env unchanged. `orbit` ready 16:30:28, then `orbit-agent` (17 s down). `vm-health.conf` (old copy
`.before-3acc586`): `PROD_TAG=3acc586`, `ROLLBACK_TAG=dc59487`, `ROLLBACK_BACKUP=01a1e531-d917-75f7-92d7-cf225067a4de`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness (all 6 checks) local and on
`https://orbit-app.easytomanage.com/health/ready`, `/health/live` 200; `/orbit-build.json` revision `3acc5868…`; the new
Windows mic text is in the binary; no orbit warnings in the journal. `vm-health` OK except nginx (connection-refused
lines of the switch at 16:30; 0 5xx), oom (2 older kills) and git-owner (given back to exedev).

Rollback to `dc59487` (no schema change, so no data is lost):

```sh
sudo systemctl stop orbit-agent orbit
sudo rm -rf /opt/orbit-agent/src && sudo cp -a /opt/orbit-agent.before-3acc586/src /opt/orbit-agent/src
sudo install -m 755 /usr/local/bin/orbit-dc59487 /usr/local/bin/orbit
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# (the same as /root/ops/desktop-014/rollback.sh)
# Way 2, only for a data problem (work after 16:30 UTC is lost), Orbit stopped: /usr/local/bin/orbit-dc59487 backup verify +
#   backup restore 01a1e531-d917-75f7-92d7-cf225067a4de (as user orbit, with /etc/orbit/orbit.env), then the lines above
```

After this rollback, set `PROD_TAG=dc59487`, `ROLLBACK_TAG=f974107`,
`ROLLBACK_BACKUP=01a1e48b-ff0e-7656-9d3a-514114f7cf9e` in `/root/ops/vm-health.conf` (copy
`/root/ops/vm-health.conf.before-3acc586`).

### November 16, 2026 hotfix: push inbox NULL actor panic (no migration)

`main` from `93ea7006` to `dc59487c` (local, not pushed): `--no-ff` merge of `fix-null-actor-panic` (`7f08b421`,
worktree `/root/jean/orbit/fix-null-actor-panic`, based on `c62e383c`), no conflicts with build 16. `push_inbox`
reads `actor_user_id` as `Option` (a GitHub-synced comment has no member actor); new `app::unless_panic` logs a
panicking round and keeps the push inbox, scheduler and routines loops alive (before, a panic stopped the whole
server). No migration, no new env key, `apps/agent` unchanged, no what's-new line (a bug fix).

Checks on the merge (main checkout): `push_api` 6, `push_devices_api` 7 (with the new GitHub-author test), lib `push`
10, lib `app::tests` 5, on SQLite and Postgres: all pass. Scripts and logs: `/root/ops/null-actor-hotfix/`.

Deploy id `20261116T123445Z-2223546`. `orbit:dc59487` (image
`sha256:9f4eeffa525dd4584d447ca8c1e9cf97c11e660e2e64b7f1e798501c5a4e9aa4`, label `dc59487c…`, about 45 min with the
lock wait); `deploy/smoke.sh` and `orbit config check` (production env) passed. No-migration check (release binary on
a hot copy, and in the switch on a copy of the stopped database): 0 applied, schema 20261115105242 (119 rows), counts,
names and checksums unchanged, quick-check ok, 0 foreign-key rows.

Switch: stopped both services at 13:29:02 UTC; snapshot `01a1e48b-ff0e-7656-9d3a-514114f7cf9e` with
`/usr/local/bin/orbit-f974107`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-dc59487.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-dc59487.tgz`; sidecar code unchanged; env unchanged. Binary installed
(`/usr/local/bin/orbit-dc59487` too), `orbit` ready 13:29:13, then `orbit-agent` (11 s down). `vm-health.conf` (old
copy `.before-dc59487`): `PROD_TAG=dc59487`, `ROLLBACK_TAG=f974107`,
`ROLLBACK_BACKUP=01a1e48b-ff0e-7656-9d3a-514114f7cf9e`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness (all 6 checks) and `/health/live` 200;
`/orbit-build.json` revision `dc59487c…`; `iPhone push (APNs) is on` in the journal; only the known "allowed origin
without a port" WARN; 0 unpushed notifications (the push inbox pass runs). f974107 crashed a 5th time at 12:54:03
(GitHub comment notification `01a1e46b…`, before the switch). `vm-health` OK except nginx (connection-refused lines of
the 12:31 and 12:54 crashes and the switch; 0 5xx), oom (2 older kills of other work) and git-owner (given back to
exedev). **Not** verified on production: a real GitHub-author notification passing the push without a panic (none came
in the 20 min after the switch, up to 13:49 UTC, 0 restarts); the tests cover it on SQLite and Postgres.

Rollback to `f974107` (no schema change, so no data is lost; the sidecar code did not change). It brings the crash
back.

```sh
sudo systemctl stop orbit-agent orbit
sudo install -m 755 /usr/local/bin/orbit-f974107 /usr/local/bin/orbit
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# (the same as /root/ops/null-actor-hotfix/rollback.sh)
# Way 2, only for a data problem (work after 13:29 UTC is lost), Orbit stopped: /usr/local/bin/orbit-f974107 backup verify +
#   backup restore 01a1e48b-ff0e-7656-9d3a-514114f7cf9e (as user orbit, with /etc/orbit/orbit.env), then the lines above
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-dc59487.tgz
# Plain copy of the database before the switch: /var/backups/orbit/pre-migration/orbit-before-dc59487.sqlite
```

After this rollback, set `PROD_TAG=f974107`, `ROLLBACK_TAG=8cd111f`,
`ROLLBACK_BACKUP=01a1e454-395d-72d4-8158-085a4cb865d6` in `/root/ops/vm-health.conf` (copy
`/root/ops/vm-health.conf.before-dc59487`).

### November 16, 2026 upgrade: build 16 server part (schema 20261115105242, 119 rows; 1 migration; 8 pages repaired)

`main` from `c62e383c` to `f9741078` (local, not pushed): seven `--no-ff` merges, in this order (worktrees under
`/root/jean/orbit/<name>`): `iphone-audit` (`d46302e3`, 9 commits: iPhones get the push also while the user is active
in Orbit and a tap opens it in its workspace, unregister with the token in the body (`DELETE /api/v1/push/devices`),
push log lines, integration messages and emoji avatars, percent-sign share fix, token only to the Orbit server);
`iphone-b15-fixes` (`f0334c97`; its `490290fa` badge fix is already on main as `8cd111fd`: main's version kept in the 4
conflicted files (`push/apns.rs`, `push/mod.rs`, `tests/push_devices_api.rs`, `docs/operations.md`, which only
`490290fa` changed), so only the `KeyboardView` commits came in); `iphone-voice-input` (`e6552932`, ships with the next
iPhone build); `voice-input-desktop` (`442c568d`, the web mic ships now; the Mac app later); `docs-tables` (`ddd3bf79`,
GFM tables and links on Docs pages, `orbit pages repair-markdown`); `donna-delete` (`a6afeea9`, `0caa95a2`:
`move_to_trash` for pages and tasks, `drive_trash` takes a list; `POST …/agent-tools/trash`; no migration);
`webhook-read` (`70276a1f`: "Can read this channel", `PUT …/webhooks/{id}/read`). Conflicts: `prompt.ts` `aboutOrbit()`
twice (donna-delete's trash text + docs-tables' "GFM tables and links"; webhook-read's two-way bots line + the iPhone voice
line), `sdk.gen.ts` imports (union; `just api` regenerated equal). `f9741078`: `openapi_contract` 393 operations (each
side counted 392). What's new `2026-11-16` (4 lines: voice input, Donna trash + tables, webhook read, iPhone push); the
webhook line moved there from `2026-11-15`; the oldest entry (`2026-11-07`) dropped to keep 8.

Checks on the merge (main checkout, after `bun install` and agent `npm ci`): web `bun --bun run build` (`tsc -b` + vite),
mobile `tsc`, `packages/api` `tsc`, agent `tsc` and 101 tests (11 skipped). SQLite and Postgres: `agent_drive` 7,
`agent_profiles` 9, `agent_trash` 3, `chat_api` 40, `chat_apps` 6, `doc_markdown_writes` 9, `openapi_contract` 19,
`push_api` 6, `push_devices_api` 6; lib `push` 10, `agent::` 56: all pass. Scripts and logs: `/root/ops/build16/`.

Deploy id `20261116T112128Z-2200587`. `orbit:f974107` (image
`sha256:6f01932eb0a31fb1e9e950e719af31f143c609a3b00f6c7b8fe207731cb4a635`, label `f9741078…`, about 31 min);
`deploy/smoke.sh` and `orbit config check` (production env) passed. Before the switch, the release binary's embedded app
on a seeded instance in `unshare -n` (agent on, no sidecar, signed in as owner): `/`, `/tasks`, `/docs`, `/drive`,
`/chat`, `/activity`, `/admin`, `/settings/agentic/agents`, `/settings/agentic/skills` render; the composer has the mic
button; a webhook made there reads 404 → "Can read" on → 200 (listed `can_read` true, `messages_url` set) → off → 404,
then deleted; 0 page or console errors (`pre/bundle.log`, `pre-chat-mic.png`). Migration dry run (release binary on a
hot copy, and in the switch on a copy of the stopped database): 1 applied, a second run 0, 119 rows, the 118 old
checksums, counts and user names unchanged (9 users, 124 tasks, 6 projects, 1192 chat messages, 62 pages),
`chat_webhooks.can_read` added (3 webhooks, all off), quick-check ok, 0 foreign-key rows; on the same copy
`pages repair-markdown`: dry run 8, run 8 (8 new page versions), a second run 0. Rollback test: the old binary refuses
("schema version 20261115105242 is newer"), `rollback-schema.sql` (drop the column, delete the version row) brings
`sqlite_master` back equal to the source, counts and checksums equal, the old binary applies 0, the new one 1 again.

Switch: stopped both services at 12:28:07 UTC; snapshot `01a1e454-395d-72d4-8158-085a4cb865d6` with
`/usr/local/bin/orbit-8cd111f`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-f974107.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-f974107.tgz`; old sidecar code `/opt/orbit-agent.before-f974107`; new
sidecar `src` (`package*.json` unchanged); env unchanged. Binary installed (`/usr/local/bin/orbit-f974107` too), then,
Orbit still stopped, `orbit pages repair-markdown` (it migrates first): dry run 8, run **8 pages repaired** (Product
Pricing Template, Studio Product & Follow-up Tracker, Notion Import Cleanup — November 10, 2026, Wellrox Articles, Draft —
How to Make Room for Movement on a Busy Day, Resistance Bands, Logistics & Inventory, Sales, Pricing & Listings), 0
skipped, a second run 0. `orbit` ready 12:28:30, then `orbit-agent` (23 s down). `vm-health.conf` (old copy
`.before-f974107`): `PROD_TAG=f974107`, `ROLLBACK_TAG=8cd111f`, `ROLLBACK_BACKUP=01a1e454-395d-72d4-8158-085a4cb865d6`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness (all 6 checks) and `/health/live` 200;
`/orbit-build.json` revision `f9741078…`; schema 20261115105242 (119 rows), live quick-check ok, 0 foreign-key rows;
`iPhone push (APNs) is on` in the journal; 0 error lines after the start (only the known "allowed origin without a port"
WARN and Node's ExperimentalWarning). New routes without a session (before → after): `PUT …/webhooks/{id}/read` 404 →
401, `DELETE /api/v1/push/devices` 405 → 401, `POST …/agent-tools/trash` 404 → 422 (an unknown agent-tools path stays
404). Sidecar `/health` ok (`mock: false`, `sandbox: true`), `src` equal to `main`, the deployed `prompt.ts` names
`move_to_trash`, GFM tables and "Can read this channel". Built bundle on production as David Dusi (temporary v7 session,
`ui-check.ts`, all API writes blocked; only the page-visit POST was tried and blocked): `/`, `/chat`, `/tasks`, `/docs`,
`/drive` render signed in, the chat composer has the mic button, the repaired "Product Pricing Template" shows its 2
tables, `GET …/webhooks` of #orbit-updates 200 with `can_read` false, `auth/me` 200, 0 console or page errors.
Screenshots `/root/.local/share/com.jean.desktop/pasted-images/build16/prod-{home,chat,repaired-page}.png` (pre-switch:
`pre-chat-mic.png`). `ORBIT_VERIFY_SPA=1 verify-production.sh` passed (session logged out, the row deleted after; token
file, TLS hop and its pem removed). `vm-health` OK except nginx (connection-refused lines of the switch window and of
the two crashes below; 0 5xx), docker (another worktree's `donna-delete-s3` container) and oom (2 gvisor kills of other
work this morning). **Not** verified on production: real voice input (needs a person's microphone), Donna moving items to
the trash in a real model run (a write), a webhook read with a real secret, an iPhone push while active (needs a look at
the phone).

**Open (found during this deploy, not caused by it):** Orbit stopped and restarted at 10:25, 12:10 and 12:21 UTC
(8cd111f) with `background service failed: … decoding column "actor_user_id": expected TEXT, got Null` in
`PushService::push_inbox` (`apps/server/src/push/mod.rs`, `row.get::<String, _>("actor_user_id")`): a `task_commented`
notification with no actor, made by the GitHub comment sync (comment author service account, `github_author_login`
DaveDushi). The rows are marked pushed first, so there is no loop, but each such comment costs a restart (about 6 s
down). f974107 has the same code: it restarted once for it at 12:31:26 (after the checks above; ready again
by 12:31:58, 4th such row, all marked pushed). Fix: read it as `Option<String>` (as the `icon` line below it does), then
a hotfix deploy.

Rollback to `8cd111f`. Way 1 keeps the work done after the switch, but loses the "Can read this channel" switches;
the 8 repaired pages stay repaired (the old binary reads them; their old versions are in the page history).

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit sqlite3 /var/lib/orbit/orbit.sqlite < /root/ops/build16/rollback-schema.sql
sudo install -m 755 /usr/local/bin/orbit-8cd111f /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-f974107 /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# (the same as /root/ops/build16/rollback.sh, which also keeps a copy of the database first)
# rollback-schema.sql: ALTER TABLE chat_webhooks DROP COLUMN can_read, delete the version row
# Way 2, only for a data problem (work after 12:28 UTC is lost), Orbit stopped: /usr/local/bin/orbit-8cd111f backup verify +
#   backup restore 01a1e454-395d-72d4-8158-085a4cb865d6 (as user orbit, with /etc/orbit/orbit.env), then the binary and sidecar lines above
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-f974107.tgz
# Plain copy of the database before the switch: /var/backups/orbit/pre-migration/orbit-before-f974107.sqlite
```

After this rollback, set `PROD_TAG=8cd111f`, `ROLLBACK_TAG=28c5f2d`,
`ROLLBACK_BACKUP=01a1e010-342a-70a0-b5ba-f7f912a244cd` in `/root/ops/vm-health.conf` (copy
`/root/ops/vm-health.conf.before-f974107`).

### November 15, 2026 hotfix: iPhone icon badge with badge-only APNs pushes (no migration)

`main` from `07735104` to `8cd111fd` (local, not pushed at the deploy): fast-forward to branch `badge-hotfix`
(worktree `/root/jean/orbit/badge-hotfix`), which is a cherry-pick of `490290fa` from `iphone-b15-fixes`. The
`SheetKeyboardView.tsx` → `KeyboardView.tsx` rename of `490290fa` is left out (`ui/index.ts` on main still imports the
old name; the rename belongs to the keyboard commits `c385d513`, `035a30c7`, which are not on main). Server: a notice
that is not pushed (open web tab, "do not disturb", kind off) and every chat read or inbox read now send the iPhones a
badge-only push (`{"aps":{"badge":N}}`, collapse id `badge`) when the count is not the one they got last (in-memory per
server). No migration, no new env key, `apps/agent` unchanged (no sidecar copy, no what's-new entry: a bug fix).

Checks on the branch: mobile typecheck and 226 tests; Rust `chat_api` 40, `push_api` 6, `push_devices_api` 5 on SQLite
and Postgres; `--lib push` 10. Scripts and logs: `/root/ops/badge-hotfix/`.

Deploy id `20261115T155545Z-2025947`. `orbit:8cd111f` (image
`sha256:5ab2969fcc8ce004e038edbe45f407f3ea8fcf3a9c3cb568207724a617618ab6`, label `8cd111fd…`, about 38 min);
`deploy/smoke.sh` and `orbit config check` (production env) passed. No-migration check (release binary on a hot copy,
and in the switch on a copy of the stopped database): 0 applied, schema 20261114000500 (118 rows), counts, names and
checksums unchanged, quick-check ok, 0 foreign-key rows.

Switch: stopped both services at 16:35:20 UTC; snapshot `01a1e010-342a-70a0-b5ba-f7f912a244cd` with
`/usr/local/bin/orbit-28c5f2d`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-8cd111f.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-8cd111f.tgz`; sidecar code unchanged; env unchanged. Binary installed
(`/usr/local/bin/orbit-8cd111f` too), `orbit` ready 16:35:32, then `orbit-agent` (12 s down). `vm-health.conf` (old copy
`.before-8cd111f`): `PROD_TAG=8cd111f`, `ROLLBACK_TAG=28c5f2d`, `ROLLBACK_BACKUP=01a1e010-342a-70a0-b5ba-f7f912a244cd`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness (all 6 checks) and `/health/live` 200;
`/orbit-build.json` revision `8cd111fd…`; `iPhone push (APNs) is on` (topic `com.easytomanage.orbit`, sandbox false) in the
journal; the one WARN after the start (allowed origin without a port) was there before too. Badge test (temporary 30-min
sessions for Arthur, an agent account, and David Dusi, who has the one iPhone device row): Arthur sent a test message in
their DM (200), David marked the DM read (200; the badge-only push). 0 WARN/ERROR lines in the journal (no `iPhone push
not sent`), the device row kept with `failures = 0`, the orbit process had TLS connections to Apple (17.188.x.x:443). The
test message was deleted, both sessions logged out and their rows deleted. `vm-health` OK except nginx (10 lines, the
switch window; 0 5xx) and git-owner (8 older root files; this deploy's given back to exedev). **Not** verified: the number
on the real iPhone icon (no log line on a successful send; needs a look at the phone).

Rollback to `28c5f2d` (no schema change, so no data is lost; the sidecar code did not change):

```sh
sudo systemctl stop orbit-agent orbit
sudo install -m 755 /usr/local/bin/orbit-28c5f2d /usr/local/bin/orbit
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# (the same as /root/ops/badge-hotfix/rollback.sh)
# Way 2, only for a data problem (work after 16:35 UTC is lost), Orbit stopped: /usr/local/bin/orbit-28c5f2d backup verify +
#   backup restore 01a1e010-342a-70a0-b5ba-f7f912a244cd (as user orbit, with /etc/orbit/orbit.env), then the lines above
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-8cd111f.tgz
# Plain copy of the database before the switch: /var/backups/orbit/pre-migration/orbit-before-8cd111f.sqlite
```

After this rollback, set `PROD_TAG=28c5f2d`, `ROLLBACK_TAG=d6c06e9`,
`ROLLBACK_BACKUP=01a1dec9-e080-70c7-903c-f61c73b4f260` in `/root/ops/vm-health.conf` (copy
`/root/ops/vm-health.conf.before-8cd111f`).

### November 15, 2026 upgrade: Donna sees chat images, label rules on tasks that have the label (no migration)

`main` from `7414676b` to `28c5f2d4` (local, not pushed at the deploy): `--no-ff` merge of `iphone-b14-fixes`
(`7c779032..c704cb68`, worktree `/root/jean/orbit/iphone-b14-fixes`, based on `7414676b`, so the merge tree equals the
branch tip). `7c779032`: Donna looks at images sent in chat and in the Ask Donna panel (`view_image` takes
`chat_file_ids`; the sidecar downloads them as the driver with `GET …/chat/files/{id}`, which `scope::allows` now lets
agent sessions reach, `route_need` gives to `view_image`, and `download_chat_file` refuses an agent session every file
that is not `image/*`; the driver's own file access check still runs); label rules also run for open tasks that already
have the label; `aboutOrbit()` names chat images (media part); what's new `2026-11-15` (2 lines). `6484869d`: mobile
`SheetKeyboardView` (Ask Donna, Docs comments, chat sheet, share screen keep the composer above the keyboard; ships with
iPhone build 14). `c704cb68`: test for bearer-token chat images. No migration, no new env key, `package*.json` of the
sidecar unchanged.

Checks on the branch (by the Feature Builder): mobile typecheck and 226 tests, iOS bundle export, agent tests 98 pass
(11 skipped), Rust `agent_api` 21 and `label_rules_api` 10 on SQLite and Postgres. Scripts and logs:
`/root/ops/iphone-b14/`.

Deploy id `20261115T100250Z-1955718`. `orbit:28c5f2d` (image
`sha256:9fe5fb041c404aa291abf57eacbd6dc21bc01ea51ee6cec0036888dd7e10ba48`, label `28c5f2d4…`, about 34 min);
`deploy/smoke.sh` and `orbit config check` (production env) passed. Before the switch, the release binary's embedded app
on a seeded instance in `unshare -n` with the agent on and no sidecar (signed in, owner): `/`, `/tasks`, `/docs`,
`/drive`, `/chat`, `/activity`, `/admin`, `/settings/agentic/agents`, `/settings/agentic/skills` render, 0 page or console
errors (`pre/bundle.log`). No-migration check (release binary on a hot copy, and in the switch on a copy of the stopped
database): 0 applied, schema 20261114000500 (118 rows), counts, names and checksums unchanged (9 users, 110 tasks, 6
projects, 1153 chat messages, 57 pages), quick-check ok, 0 foreign-key rows.

Switch: stopped both services at 10:38:54 UTC; snapshot `01a1dec9-e080-70c7-903c-f61c73b4f260` with
`/usr/local/bin/orbit-d6c06e9`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-28c5f2d.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-28c5f2d.tgz`; old sidecar code `/opt/orbit-agent.before-28c5f2d`; new
sidecar `src`; env unchanged. Binary installed (`/usr/local/bin/orbit-28c5f2d` too), `orbit` ready 10:39:11, then
`orbit-agent` (17 s down). `vm-health.conf` (old copy `.before-28c5f2d`): `PROD_TAG=28c5f2d`, `ROLLBACK_TAG=d6c06e9`,
`ROLLBACK_BACKUP=01a1dec9-e080-70c7-903c-f61c73b4f260`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness (all 6 checks) and `/health/live` 200;
`/orbit-build.json` revision `28c5f2d4…`; schema 20261114000500 (118 rows), live quick-check ok; 0 error lines in the
journal after the start; sidecar `/health` ok (`mock: false`; with the secret `sandbox: true`, so `aboutOrbit()` gets its
media part) and `src` equal to `main`; the deployed `prompt.ts` names chat images ("looks at images sent in chat", "view_image
with chat_file_ids"). Built bundle on production as David Dusi (temporary v7 session, `ui-check.ts`, all API writes
blocked, none tried): `/`, `/chat`, `/tasks`, `/docs`, `/drive` render signed in, `auth/me` 200, 0 console or page errors.
Screenshots `/root/.local/share/com.jean.desktop/pasted-images/iphone-b14/prod-{home,chat}.png` (pre-switch:
`pre-{chat,tasks}.png`). `ORBIT_VERIFY_SPA=1 verify-production.sh` passed (session logged out, the row deleted after;
token file, TLS hop and its pem removed). `vm-health` OK except nginx (25 lines, the switch window; 0 5xx) and
git-owner (other worktrees' metadata in `.git`; this merge's root-owned files given back to exedev). **Not** verified on
production: Donna describing a real chat image (it needs a real model run on a person's ChatGPT connection and a post
in their DM; covered by the sidecar and `agent_api` tests only), a label rule run on real tasks (a write).

Rollback to `d6c06e9` (no schema change, so no data is lost):

```sh
sudo systemctl stop orbit-agent orbit
sudo install -m 755 /usr/local/bin/orbit-d6c06e9 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-28c5f2d /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# (the same as /root/ops/iphone-b14/rollback.sh)
# Way 2, only for a data problem (work after 10:38 UTC is lost), Orbit stopped: /usr/local/bin/orbit-d6c06e9 backup verify +
#   backup restore 01a1dec9-e080-70c7-903c-f61c73b4f260 (as user orbit, with /etc/orbit/orbit.env), then the lines above
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-28c5f2d.tgz
# Plain copy of the database before the switch: /var/backups/orbit/pre-migration/orbit-before-28c5f2d.sqlite
```

After this rollback, set `PROD_TAG=d6c06e9`, `ROLLBACK_TAG=d4ee5e0`,
`ROLLBACK_BACKUP=01a1d887-0f47-76d7-a8b2-272e6aa01116` in `/root/ops/vm-health.conf` (copy
`/root/ops/vm-health.conf.before-28c5f2d`).

### November 14, 2026 upgrade: agent skills (schema 20261114000500, 118 rows; 1 migration)

`main` from `ec28d2c3` to `d6c06e95` (local, not pushed): merge of `agent-skills` (`7abb5432..494b9440`, worktree
`/root/jean/orbit/agent-skills`, based on `4877dd5c` (port 5), so before logos, doc links and agent task runs). Agent
Skills (agentskills.io SKILL.md format): owners and admins add them in Settings → Agentic → Skills (install from a
public GitHub folder or a skills.sh page through the agent tools' guarded client: HTTPS to public addresses only, no
redirects, size and count limits; upload a `.zip` (traversal, symlink, encryption, size and count checks); or write one)
and tick the agents that use each. Progressive load: the prompt gets only names and descriptions (`orbit-skills`
section, read as the driver at the start of each run, task runs too); `load_skill` and `read_skill_file` read the rest
(`GET …/agent-tools/skills…`, only the agent's ticked skills). Scripts run only in the media sandbox, for an agent with
`media_shell`; Orbit never runs them. Agent sessions are denied `agent-skills` (scope `DENIED`). Not in the OpenAPI
document (no contract count change). User docs `docs/agent.md` "Skills"; design `.ai/agent-design.md`. Conflicts: what's
new `2026-11-14` (the skills text joined to the agents line, still 4 lines), `agent/mod.rs` (both), `profiles.rs`
`route_need` and its test (shopify-image and skills, both), `docs/agent.md` (both sections), lessons (both). Slack text
(not posted): `/root/ops/agent-skills/slack.json`.

Migration (SQLite + Postgres, not destructive): `20261114000500_agent_skills` (new tables `agent_skills`
(`workspace_id` → ON DELETE CASCADE, `created_by` → `users` ON DELETE SET NULL, CHECK on `source`, UNIQUE
`(workspace_id, name)`), `agent_skill_files` and `agent_skill_agents` (both → `agent_skills` ON DELETE CASCADE), index
`agent_skill_agents_agent`). No RESTRICT reference, no table rebuild.

Checks on the merge commit (main checkout, mock model only, no real network): server debug build (no warnings); web
`bun --bun run build` (`tsc -b` + vite); web `SkillsPage` + `settingsNav` tests 5; agent `tsc` and 95 tests (11
skipped). SQLite and Postgres: `agent_skills` 4, `agent_profiles` 9, `agent_api` 18, `agent_task_runs` 6,
`guest_access` 1, `openapi_contract` 19; lib `agent::` 56: all pass. Mobile not touched. Scripts and logs:
`/root/ops/agent-skills/`.

Deploy id `20261114T043714Z-1348389`. `orbit:d6c06e9` (image
`sha256:23c5f731fa28145a07c2b507af1f83fd3c716c3c5fcf6febd5b4c401567de32d`, label `d6c06e95…`, about 33 min);
`deploy/smoke.sh` and `orbit config check` (production env) passed. Before the switch, the release binary's embedded app
on a seeded instance in `unshare -n` with the agent on and no sidecar (signed in, owner): `/`, `/tasks`, `/docs`,
`/drive`, `/chat`, `/activity`, `/admin`, `/settings/agentic/agents`; Settings → Agentic → Skills empty → Add skill (3
ways: GitHub or skills.sh, Upload .zip, Write; the .zip input is there) → write `weekly-report` (1 `agent_skills` row) →
tick "Orbit Agent" (1 `agent_skill_agents` row) → delete (0 rows); 0 page or console errors. Migration dry run (debug
binary on a hot copy, and in the switch the release binary on a copy of the stopped database): 1 applied, a second run
0, 118 rows, the 117 old checksums, counts and user names unchanged (9 users, 110 tasks, 6 projects, 1137 chat
messages, 3 teamspaces, 57 pages), 3 tables and the index, 0 rows, quick-check ok, 0 foreign-key rows. Rollback test on
the same copies: the old binary refuses ("database schema version 20261114000500 is newer"), `rollback-schema.sql`
brings `sqlite_master` back equal to the source, counts and checksums equal, the old binary applies 0, the new binary
applies 1 again.

Switch: stopped both services at 05:28:12 UTC; snapshot `01a1d887-0f47-76d7-a8b2-272e6aa01116` with
`/usr/local/bin/orbit-d4ee5e0`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-d6c06e9.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-d6c06e9.tgz`; old sidecar code `/opt/orbit-agent.before-d6c06e9`; new
sidecar `src` (`package*.json` unchanged); env unchanged. Binary installed (`/usr/local/bin/orbit-d6c06e9` too), `orbit`
ready 05:28:24, then `orbit-agent` (12 s down). `vm-health.conf` (old copy `.before-d6c06e9`): `PROD_TAG=d6c06e9`,
`ROLLBACK_TAG=d4ee5e0`, `ROLLBACK_BACKUP=01a1d887-0f47-76d7-a8b2-272e6aa01116`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness 200 and `/health/live` 200; `/orbit-build.json`
revision `d6c06e95…`; schema 20261114000500 (118 rows, 3 tables and the index, 0 skills); live quick-check ok, 0
foreign-key rows; 0 error lines in the journal after the start (only the known "allowed origin without a port" WARN and
Node's SQLite ExperimentalWarning); sidecar `/health` ok (`mock: false`; with the secret `sandbox: true`, so skill
scripts can run for agents with `media_shell`) and `src` equal to `main`. Built bundle on production as David Dusi
(temporary v7 session, `ui-check.ts`, all API writes blocked, none tried): `GET …/agent-skills` 200 (0 skills, max 100,
agents Donna and Arthur); Settings → Agentic → Skills shows the Skills tab, "No skills yet", the sandbox note and Add
skill; the Add skill modal has the 3 ways; Cancel, nothing filled or sent; 0 console or page errors. No skill installed
on production. Screenshots `/root/.local/share/com.jean.desktop/pasted-images/agent-skills/prod-{skills-empty,
skills-add-modal}.png` (pre-switch: `pre-skills-{empty,write,one}.png`). `ORBIT_VERIFY_SPA=1 verify-production.sh`
passed (session logged out, the row deleted after; token file, TLS hop and its pem removed). `vm-health` OK except nginx
(19 lines, upstream refused in the switch windows 04:33 and 05:28; 0 5xx), docker (other worktrees' containers) and
git-owner (this merge's root-owned files given back to exedev). **Not** verified on production: a real install from
GitHub or skills.sh, a `.zip` upload, `load_skill` in a real model run, a skill script in the sandbox.

Rollback to `d4ee5e0`. Way 1 keeps the work done after the switch, but loses the skills.

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit sqlite3 /var/lib/orbit/orbit.sqlite < /root/ops/agent-skills/rollback-schema.sql
sudo install -m 755 /usr/local/bin/orbit-d4ee5e0 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-d6c06e9 /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# (the same as /root/ops/agent-skills/rollback.sh, which also keeps a copy of the database first)
# rollback-schema.sql: drop the index and the 3 skill tables, delete the version row
# Way 2, only for a data problem (work after 05:28 UTC is lost), Orbit stopped: /usr/local/bin/orbit-d4ee5e0 backup verify +
#   backup restore 01a1d887-0f47-76d7-a8b2-272e6aa01116 (as user orbit, with /etc/orbit/orbit.env), then the binary and sidecar lines above
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-d6c06e9.tgz
# Plain copy of the database before the switch: /var/backups/orbit/pre-migration/orbit-before-d6c06e9.sqlite
```

After this rollback, set `PROD_TAG=d4ee5e0`, `ROLLBACK_TAG=9b483b6`,
`ROLLBACK_BACKUP=01a1d854-e094-7024-8a2d-13078715882c` in `/root/ops/vm-health.conf` (copy
`/root/ops/vm-health.conf.before-d6c06e9`).

### November 14, 2026 upgrade: agents work on tasks assigned to them (schema 20261114000300, 117 rows; 1 migration)

`main` from `99dd6d6f` to `d4ee5e07` (local, not pushed): merge of `agent-assigned-tasks` (`9f9a8a90..62ae4399`, worktree
`/root/jean/orbit/agent-assigned-tasks`, based on `4877dd5c` (port 5), so before the port 4 merge fixes, logos and doc
links). An agent assigned to a task (new task, task details, bulk edit, API, MCP) starts a run at once; the driver is
the assigner (their permissions, their ChatGPT, approvals and questions in their DM with the agent under "You assigned
me…"). An `@agent` comment, or a person's new comment on a task assigned to an agent, starts a run for the comment's
author. Progress and result go into one task comment written as the agent. No trigger from the agent's own comments,
agent sessions (no agent-to-agent), service-account tokens, label rules or guests. One running run per task (partial
unique index + claim), later ones queue; unassign or delete (trash) cancels, and the sidecar aborts a started run
(`stops` in the inputs poll). Design `.ai/agent-design.md` "Task runs" (T1-T6); user docs `docs/agent.md` "Tasks for
agents". Internal routes only (`/api/internal/agent/task-runs/…`), no OpenAPI change. Conflicts: what's new
`2026-11-14` (agents line on top; the Chat and workspace logos lines joined, still 4 lines), aboutOrbit (`main`'s text
plus the branch's task-run parts), `get_task` (linked docs and the newest 20 comments), `agent/mod.rs` and
`task_routes.rs` (both sides), lessons (both). Fix in the merge: the doc-links sidecar test's fake Orbit answers the
comments read of `get_task`. What is new for people: the agents line on `2026-11-14`. Slack text (not posted):
`/root/ops/agent-assigned-tasks/slack.json`.

Migration (SQLite + Postgres, not destructive): `20261114000300_agent_task_runs` (new table `agent_task_runs`:
`workspace_id`/`task_id`/`agent_user_id`/`driver_user_id` → ON DELETE CASCADE, `comment_id`/`result_comment_id` →
`task_comments` ON DELETE SET NULL, CHECKs on `cause` and `status`; indexes `agent_task_runs_one_running` (unique,
`status = 'running'`), `_task`, `_queued`, `_thread`). No RESTRICT reference and nothing references it.

Checks on the merge commit (main checkout, mock model only): server debug build; web `bun --bun run build` (`tsc -b` +
vite); agent `tsc` and 90 tests (11 skipped). SQLite and Postgres: `agent_task_runs` 6, `tasks_api` 100,
`agent_api` 18, `guest_access` 1, `task_doc_links` 2, `openapi_contract` 19; lib `agent::` 49: all pass. Mobile and
`packages/api` not touched (not run). Scripts and logs: `/root/ops/agent-assigned-tasks/`.

Deploy id `20261114T033103Z-1285608`. `orbit:d4ee5e0` (image
`sha256:e21b079d3ccbb0ba865616570c15c1a34566804ea0ce9964037e1061548685d7`, label `d4ee5e07…`, about 33 min);
`deploy/smoke.sh` and `orbit config check` (production env) passed. Before the switch, the release binary's embedded app
on a seeded instance in `unshare -n` with the agent on and no sidecar (signed in, owner): `/`, `/tasks`, `/docs`,
`/drive`, `/chat`, `/activity`, `/admin`; task detail → Assignees lists "Orbit Agent" → pick → one `agent_task_runs` row
(`queued`, `assigned`, driver = the test user) → unassign → `cancelled` ("I was unassigned from this task"); 0 page or
console errors. Migration dry run (debug binary on a hot copy, and in the switch the release binary on a copy of the
stopped database): 1 applied, a second run 0, 117 rows, the 116 old checksums, counts and user names unchanged (9 users,
110 tasks, 6 projects, 1136 chat messages, 3 teamspaces, 57 pages), `agent_task_runs` 0 rows and 4 indexes,
quick-check ok, 0 foreign-key rows. Rollback test on the same copies: the old binary refuses ("database schema version
20261114000300 is newer"), `rollback-schema.sql` brings `sqlite_master` back equal to the source, counts and checksums
equal, the old binary applies 0, the new binary applies 1 again.

Switch: stopped both services at 04:33:23 UTC; snapshot `01a1d854-e094-7024-8a2d-13078715882c` with
`/usr/local/bin/orbit-9b483b6`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-d4ee5e0.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-d4ee5e0.tgz`; old sidecar code `/opt/orbit-agent.before-d4ee5e0`; new
sidecar `src` (`package*.json` unchanged); env unchanged. Binary installed (`/usr/local/bin/orbit-d4ee5e0` too), `orbit`
ready 04:33:38, then `orbit-agent` (15 s down). `vm-health.conf` (old copy `.before-d4ee5e0`): `PROD_TAG=d4ee5e0`,
`ROLLBACK_TAG=9b483b6`, `ROLLBACK_BACKUP=01a1d854-e094-7024-8a2d-13078715882c`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness 200 and `/health/live` 200; `/orbit-build.json`
revision `d4ee5e07…`; schema 20261114000300 (117 rows, table and 4 indexes, 0 runs); live quick-check ok, 0
foreign-key rows; 0 error lines in the journal after the start (only the known "allowed origin without a port" WARN;
no task-run worker error while the sidecar polls); sidecar `/health` ok (`mock: false`) and `src` equal to `main`.
Built bundle on production as David Dusi (temporary v7 session, `ui-check.ts`, all API writes blocked, none tried): task
GEN-12 (no assignees) → Assignees picker lists 8 people with "Donna"; closed with Escape, nothing picked, assignees still
`[]`; 0 console or page errors. Screenshots
`/root/.local/share/com.jean.desktop/pasted-images/agent-assigned-tasks/prod-{assignee-picker-agent,task-detail}.png`
(pre-switch: `pre-{assignee-picker,task-assigned-agent}.png`). `ORBIT_VERIFY_SPA=1 verify-production.sh` passed
(session logged out, the row deleted after; token file, TLS hop and its pem removed). `vm-health` OK except nginx (10
lines, upstream refused in the switch windows 03:27 and 04:33), docker (other worktrees' containers) and git-owner
(this merge's root-owned files given back to exedev). **Not** verified on production: a real task run (assigning a
task to an agent starts a paid model run), the comment output, stop of a running run.

Rollback to `9b483b6`. Way 1 keeps the work done after the switch, but loses the task run rows.

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit sqlite3 /var/lib/orbit/orbit.sqlite < /root/ops/agent-assigned-tasks/rollback-schema.sql
sudo install -m 755 /usr/local/bin/orbit-9b483b6 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-d4ee5e0 /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# (the same as /root/ops/agent-assigned-tasks/rollback.sh, which also keeps a copy of the database first)
# rollback-schema.sql: acknowledge the runs' pending agent_inputs rows, drop the 4 indexes and agent_task_runs,
#   delete the version row (the runs' DM headers and task comments stay)
# Way 2, only for a data problem (work after 04:33 UTC is lost), Orbit stopped: /usr/local/bin/orbit-9b483b6 backup verify +
#   backup restore 01a1d854-e094-7024-8a2d-13078715882c (as user orbit, with /etc/orbit/orbit.env), then the binary and sidecar lines above
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-d4ee5e0.tgz
# Plain copy of the database before the switch: /var/backups/orbit/pre-migration/orbit-before-d4ee5e0.sqlite
```

After this rollback, set `PROD_TAG=9b483b6`, `ROLLBACK_TAG=fceadb0`,
`ROLLBACK_BACKUP=01a1d818-8e21-7714-825a-d235a3b49155` in `/root/ops/vm-health.conf` (copy
`/root/ops/vm-health.conf.before-d4ee5e0`).

### November 14, 2026 upgrade: docs links on tasks (schema 20261114000100, 116 rows; 1 migration)

`main` from `454a0cdd` to `9b483b6a` (local, not pushed): merge of `docs-links` (`617dbcab..d00ce0aa`, worktree
`/root/jean/orbit/docs-links`, based on `0fbcf7ca`, so before port 5 and logos). "Add doc" on a task (beside Add
relation and Add sub-issue) opens a page picker and links Docs pages to the task (its "Docs" list, x unlinks); a Docs
page shows "Linked tasks" under its title. Both sides show only what the reader sees (`pages::visible()`, the same
visibility as port 5 teamspaces; `ProjectScope` for tasks); guests reach neither side; the audit and task activity
store only the page id (title added for readers who see the page). Routes `GET/POST /tasks/{id}/linked-docs`,
`DELETE /tasks/{id}/linked-docs/{page_id}`, `GET /pages/{id}/linked-tasks`. Donna: `link_doc_to_task`,
`unlink_doc_from_task`, links in `get_task` / `read_page`. Web and mobile. Conflicts: what's new (doc links put on the
`2026-11-14` Tasks line, still 4 lines), aboutOrbit (the branch's "Docs on tasks" line, `main`'s task references line
and Agentic list plus doc links), `TOOL_CATALOG` 59 (`main` 57 + 2), `openapi_contract` 301 paths and 391 operations
(both sides had 298/387 and 298/385 from base 295/381), `sdk.gen.ts` made again (`openapi.json` equal to the
auto-merge; new schemas only `LinkDocBody`, `LinkedDoc`, `LinkedTask`). What is new for people: the doc-link text on
the `2026-11-14` Tasks line. Slack text (not posted): `/root/ops/docs-links/slack.json`.

Migration (SQLite + Postgres, not destructive): `20261114000100_task_doc_links` (new table `task_doc_links`: `id` PK,
`workspace_id`/`task_id`/`page_id` → ON DELETE CASCADE, `created_by` → `users` ON DELETE SET NULL, `UNIQUE (task_id,
page_id)`, index `task_doc_links_page`). No RESTRICT reference, so no cascade order change.

Checks on the merge commit (main checkout): server debug build; `openapi` + `api:generate`; web `bun --bun run build`
(`tsc -b` + vite) and `bun test` of the touched parts (task detail, relations, sub-issues, TasksPage, models, `docs`:
376 pass); mobile `tsc` and 226 tests; `packages/api` `tsc`; agent `tsc` and 88 tests (11 skipped, catalog test
passes). SQLite and Postgres: `task_doc_links` 2, `tasks_api` 100, `guest_access` 1, `agent_api` 18, `openapi_contract`
19, `pages_api` 26, `teamspaces_api` 8; lib `agent::profiles` 4: all pass. Not run: the full web suite, the iOS bundle
on Metro. Scripts and logs: `/root/ops/docs-links/`.

Deploy id `20261114T022204Z-1215243`. `orbit:9b483b6` (image
`sha256:0963b9c04ad7a5e5c17965309b8264be2bd4f9d9db8e1f07ffd037593f0fbab8`, label `9b483b6a…`, about 35 min);
`deploy/smoke.sh` and `orbit config check` (production env) passed. Before the switch, the release binary's embedded app
on a seeded instance in `unshare -n` (signed in, owner; 2 pages made there by API): `/`, `/tasks`, `/docs`, `/drive`,
`/chat`, `/activity`, `/admin`; task detail "Add doc" → picker "Add doc" with 2 pages → pick → Docs list 1 row and
"linked doc" in the activity; the page shows "Linked tasks" with the task; Unlink → the Docs list is gone; 0 page or
console errors. Migration dry run (debug binary on a hot copy, and in the switch the release binary on a copy of the
stopped database): 1 applied, a second run 0, 116 rows, the 115 old checksums, counts and user names unchanged (9 users,
110 tasks, 6 projects, 1135 chat messages, 3 teamspaces, 57 pages), `task_doc_links` 0 rows, quick-check ok, 0
foreign-key rows. Rollback test on the same copies: the old binary refuses ("database schema version 20261114000100 is
newer"), `rollback-schema.sql` (`DROP INDEX`, `DROP TABLE`, delete the version row) brings `sqlite_master` back equal
to the source, counts and checksums equal, the old binary applies 0, the new binary applies 1 again.

Switch: stopped both services at 03:27:30 UTC; snapshot `01a1d818-8e21-7714-825a-d235a3b49155` with
`/usr/local/bin/orbit-fceadb0`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-9b483b6.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-9b483b6.tgz`; old sidecar code `/opt/orbit-agent.before-9b483b6`; new
sidecar `src` (`package*.json` unchanged); env unchanged. Binary installed (`/usr/local/bin/orbit-9b483b6` too), `orbit`
ready 03:27:44, then `orbit-agent` (14 s down). `vm-health.conf` (old copy `.before-9b483b6`): `PROD_TAG=9b483b6`,
`ROLLBACK_TAG=fceadb0`, `ROLLBACK_BACKUP=01a1d818-8e21-7714-825a-d235a3b49155`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness 200 and `/health/live` 200; `/orbit-build.json`
revision `9b483b6a…`; schema 20261114000100 (116 rows, table and index present, 0 links); live quick-check ok, 0
foreign-key rows; 0 error lines in the journal (only the known "allowed origin without a port" WARN); sidecar `/health`
ok and `src` equal to `main`. Built bundle on production as David Dusi (temporary v7 session, `ui-check.ts`, all API
writes blocked; the only write tried was the app's own Docs page `visit` POST, blocked): a task detail shows "Add doc"
(linked-docs GET 200 `[]`); "Add doc" opens the picker titled "Add doc" with 46 pages, closed with Escape, no pick; a
teamspace page's linked-tasks GET 200 `[]` and no section; the same page with that one GET stubbed in the browser (a
real task) shows "Linked tasks 1 GEN-1 Price Sheet for Studios"; 0 console or page errors. Screenshots
`/root/.local/share/com.jean.desktop/pasted-images/docs-links/prod-{task-add-doc,add-doc-picker,page-linked-tasks-stubbed}.png`.
`ORBIT_VERIFY_SPA=1 verify-production.sh` passed (session logged out, the row deleted after; token file, TLS hop and its
pem removed). `vm-health` OK except nginx (9 lines, upstream refused in the switch window 03:27:40–03:27:44), docker
(other worktrees' containers) and git-owner (other worktrees' metadata; this merge's root-owned files given back to
exedev). **Not** verified on production: creating or removing a real link, Donna's link tools (writes).

Rollback to `fceadb0`. Way 1 keeps the work done after the switch, but loses the doc links.

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit sqlite3 /var/lib/orbit/orbit.sqlite < /root/ops/docs-links/rollback-schema.sql
sudo install -m 755 /usr/local/bin/orbit-fceadb0 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-9b483b6 /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# (the same as /root/ops/docs-links/rollback.sh, which also keeps a copy of the database first)
# rollback-schema.sql: DROP INDEX task_doc_links_page; DROP TABLE task_doc_links; delete the version row
#   (task.doc_linked / task.doc_unlinked audit rows stay)
# Way 2, only for a data problem (work after 03:27 UTC is lost), Orbit stopped: /usr/local/bin/orbit-fceadb0 backup verify +
#   backup restore 01a1d818-8e21-7714-825a-d235a3b49155 (as user orbit, with /etc/orbit/orbit.env), then the binary and sidecar lines above
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-9b483b6.tgz
# Plain copy of the database before the switch: /var/backups/orbit/pre-migration/orbit-before-9b483b6.sqlite
```

After this rollback, set `PROD_TAG=fceadb0`, `ROLLBACK_TAG=21055b4`,
`ROLLBACK_BACKUP=01a1d7d8-a29f-70f5-96f7-a42e44a4d7f0` in `/root/ops/vm-health.conf` (copy
`/root/ops/vm-health.conf.before-9b483b6`).

### November 14, 2026 upgrade: workspace logos (schema 20261113190031, 115 rows; 1 migration)

`main` from `1ee54bb0` to `fceadb01` (local, not pushed): merge of `workspace-logos` (`715888dd..0eafe1e2`, worktree
`/root/jean/orbit/workspace-logos`, based on `e06631e6`). Owners and admins upload, change or remove a workspace logo
in Settings → General (cropped to a square in the app; PNG/JPEG/WebP by magic bytes, ≤ 512 KiB, SVG refused;
`PUT/DELETE/GET /api/v1/workspaces/{id}/logo`); `logo_url` on workspaces, the OAuth consent list and the invitation
preview; shown in the switcher and its list (letter tile without a logo), on the consent and invitation pages, in the
invitation email, and in the mobile drawer, switcher sheet and Settings. Conflicts: what's new (logo text put on the
image line of the `2026-11-14` entry, still 4 lines; `main`'s `2026-11-13` entry kept), aboutOrbit (`main`'s Agentic
list without Bots + "name and logo"), mobile settings imports (both), avatar upload (the branch's `read_image_upload`
with `main`'s `.cause`), OAuth consent (`main`'s multi-workspace checkboxes, each with the branch's logo),
`openapi_contract` 298 paths and 387 operations, `sdk.gen.ts` made again (`openapi.json`/`types.gen.ts` equal to the
auto-merge, no schema lost). What is new for people: the logo text on the `2026-11-14` entry.

Migration (SQLite + Postgres, not destructive): `20261113190031_workspace_logos` (new table `workspace_logos`:
`workspace_id` PK → `workspaces` ON DELETE CASCADE, `blob_id` → `attachment_blobs` ON DELETE RESTRICT, index
`workspace_logos_blob`). Workspace purge deletes the rows before the blobs, and `BLOB_REFERENCE_COUNT` (also used by the
task trash purge) and attachment reconcile count them.

Checks on the merge commit (main checkout): server debug build; `openapi` + `api:generate`; web `bun --bun run build`
(`tsc -b` + vite) and `bun test` of the touched pages (OAuth consent, invitation, General, `app/shell`: 92 pass); mobile
`tsc` and 225 tests; `packages/api` `tsc` and 27 tests; agent `tsc` and 85 tests. SQLite and Postgres:
`workspace_logo_api` 5, `workspaces_api` 30, `attachments_api` 34, `guest_access` 1, `oauth_api` 15, `openapi_contract`
19: all pass. Not run: the full web suite, the iOS bundle on Metro. Scripts and logs: `/root/ops/workspace-logos/`.

Deploy id `20261114T011825Z-1150489`. `orbit:fceadb0` (image
`sha256:8864da88a0415c3c11842563aa705dd36af08f5c1baed9bb4fb9951d4a04b18d`, label `fceadb01…`, about 30 min);
`deploy/smoke.sh` and `orbit config check` (production env) passed. Before the switch, the release binary's embedded app
on a seeded instance in `unshare -n` (signed in, owner): `/`, `/tasks`, project settings (11 section links), `/docs`,
`/drive`, `/chat`, `/activity`, `/admin`; Settings → General: "Upload logo", upload of a PNG → "Change logo", the
switcher shows the image (logo GET 200), Remove → the letter tile again; 0 page or console errors. Migration dry run
(debug binary on a hot copy, and in the switch the release binary on a copy of the stopped database): 1 applied, a
second run 0, 115 rows, the 114 old checksums, counts and user names unchanged (9 users, 110 tasks, 6 projects, 1134
chat messages, 3 teamspaces, 57 pages), `workspace_logos` 0 rows, quick-check ok, 0 foreign-key rows. Rollback test
on the same copies: the old binary refuses ("database schema version 20261113190031 is newer"),
`rollback-schema.sql` (`DROP INDEX`, `DROP TABLE`, delete the version row) brings `sqlite_master` back equal to the
source, counts and checksums equal, the old binary applies 0, the new binary applies 1 again.

Switch: stopped both services at 02:17:41 UTC; snapshot `01a1d7d8-a29f-70f5-96f7-a42e44a4d7f0` with
`/usr/local/bin/orbit-21055b4`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-fceadb0.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-fceadb0.tgz`; old sidecar code `/opt/orbit-agent.before-fceadb0`; new
sidecar `src` (`package*.json` unchanged); env unchanged. Binary installed (`/usr/local/bin/orbit-fceadb0` too), `orbit`
ready 02:17:55, then `orbit-agent` (14 s down). `vm-health.conf` (old copy `.before-fceadb0`): `PROD_TAG=fceadb0`,
`ROLLBACK_TAG=21055b4`, `ROLLBACK_BACKUP=01a1d7d8-a29f-70f5-96f7-a42e44a4d7f0`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness 200 and `/health/live` 200; `/orbit-build.json`
revision `fceadb01…`; schema 20261113190031 (115 rows, table and index present); live quick-check ok, 0 foreign-key
rows; 0 error lines in the journal (only the known "allowed origin without a port" WARN); sidecar `/health` ok and `src`
equal to `main`. Built bundle on production as David Dusi (temporary v7 session, `ui-check.ts`, all API writes blocked,
none tried; the logo file input not touched): Settings → General shows the "W" letter tile, "Upload logo" and the hint
for the owner; the switcher has no logo image, and its list shows the letter tiles W (Wellrox) and S (Slate); project
settings still 11 section links; no workspace has a logo; 0 console or page errors. Screenshots
`/root/.local/share/com.jean.desktop/pasted-images/workspace-logos/prod-{settings-general-logo,switcher-letter-tiles,project-settings-sidebar}.png`.
`ORBIT_VERIFY_SPA=1 verify-production.sh` passed (session logged out, the row deleted after; token file, TLS hop and its
pem removed). `vm-health` OK except oom (the old one of November 13, 03:06), nginx (9 lines, upstream refused in the
switch window 02:17:52–02:17:54) and docker (other worktrees' containers). **Not** verified on production: uploading
or removing a real logo, the invitation email with a logo (writes).

Rollback to `21055b4`. Way 1 keeps the work done after the switch, but loses the workspace logos.

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit sqlite3 /var/lib/orbit/orbit.sqlite < /root/ops/workspace-logos/rollback-schema.sql
sudo install -m 755 /usr/local/bin/orbit-21055b4 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-fceadb0 /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# (the same as /root/ops/workspace-logos/rollback.sh, which also keeps a copy of the database first)
# rollback-schema.sql: DROP INDEX workspace_logos_blob; DROP TABLE workspace_logos; delete the version row
# Way 2, only for a data problem (work after 02:17 UTC is lost), Orbit stopped: /usr/local/bin/orbit-21055b4 backup verify +
#   backup restore 01a1d7d8-a29f-70f5-96f7-a42e44a4d7f0 (as user orbit, with /etc/orbit/orbit.env), then the binary and sidecar lines above
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-fceadb0.tgz
# Plain copy of the database before the switch: /var/backups/orbit/pre-migration/orbit-before-fceadb0.sqlite
```

After this rollback, set `PROD_TAG=21055b4`, `ROLLBACK_TAG=da0cf82`,
`ROLLBACK_BACKUP=01a1d79e-4dea-703b-b26b-d220c7d5ce9a` in `/root/ops/vm-health.conf` (copy
`/root/ops/vm-health.conf.before-fceadb0`).

### November 14, 2026 upgrade: upstream port 5 (schema 20261113115539, 114 rows; 1 migration)

`main` from `4734b291` to `21055b4f` (local, not pushed): fast-forward to `21055b4f`, the merge of `main` `4734b291` into
`upstream-port5` (made and checked in that worktree by its session). Port 5: `76bae889` # button in the chat composer
(#128, web and mobile; a guest's # list has only their channels and no tasks, `9baf202b`), `8362d489` doc images open on
double-click and zoom with the mouse wheel, `aa0535da` the server shrinks large uploaded images (PNG as lossless WebP in
sRGB; the `image`/`webp` crates moved to `crates/platform`, plus `moxcms`), `27be01a6` teamspaces linked to projects for
project docs (#64/#127; `PATCH /teamspaces/{id}` `project_id`, MCP `list_docs` and `GET /docs` `project_id`), `1369fbac`
project settings sections in the sidebar (one page per section), docs and lessons. What is new for people: 2 lines on
top of the `2026-11-14` entry (# button, settings pages, project docs teamspace, image shrink and zoom).

Migration (SQLite + Postgres, not destructive): `20261113101100_teamspace_projects` (`teamspaces.project_id`
REFERENCES `projects` ON DELETE SET NULL, partial unique index `teamspaces_project`). Its version is older than the
applied `20261113115539`; the runner applied it, the max version stays `20261113115539`.

Checks on the merge (port 5 session, worktree): web `bun run build` (`tsc -b` + vite); `bun test src` web fails 2 + 1
error in `TaskList.test.tsx` (bulk toolbar > 100, bulk due date), the same on `main` `4734b291`, so not from this
merge; mobile `tsc` and 225 tests; agent `tsc` and 85 tests (with the `TOOL_CATALOG` profiles test); `openapi` +
`api:generate` gave no diff; server debug build; SQLite and Postgres `chat_api` 39, `openapi_contract` 19,
`teamspaces_api` 8, `guest_access` 1, `mcp_api` 19, `private_projects` 8, `home_layout_api` 6: all pass. Not run: the
iOS bundle on Metro. Scripts and logs: `/root/ops/upstream-port5/`.

Deploy id `20261114T000950Z-1075791`. `orbit:21055b4` (image
`sha256:e1c33a27da80e2a19d7c7ea4eb26c4e1b2ba93b8559711058bafec5d8e7e69d9`, label `21055b4f…`, about 35 min);
`deploy/smoke.sh` and `orbit config check` (production env) passed. Before the switch, the release binary's embedded app
on a seeded instance in `unshare -n` (signed in): `/`, `/tasks`, project settings (back link + 11 section links),
`settings/docs` (Docs teamspace select), `settings/milestones`, `/docs`, `/drive`, a channel (# button: Channels and
Tasks list), `/activity`, `/settings`, `/admin`; 0 page or console errors. Migration dry run (release binary on a hot
copy, and in the switch on a copy of the stopped database): 1 applied, a second run 0, 114 rows, the 113 old checksums,
counts and user names unchanged (9 users, 110 tasks, 6 projects, 1133 chat messages, 3 teamspaces, 57 pages),
quick-check ok, 0 foreign-key rows. Rollback test on the same copies: the old binary refuses ("migration 20261113101100
is not known"), `rollback-schema.sql` (`DROP INDEX`, `DROP COLUMN`, delete the version row) brings `sqlite_master` back
equal to the source, counts and checksums equal, the old binary applies 0, the new binary applies 1 again.

Switch: stopped both services at 01:13:58 UTC; snapshot `01a1d79e-4dea-703b-b26b-d220c7d5ce9a` with
`/usr/local/bin/orbit-da0cf82`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-21055b4.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-21055b4.tgz`; old sidecar code `/opt/orbit-agent.before-21055b4`; new
sidecar `src` (`package*.json` unchanged); env unchanged. Binary installed (`/usr/local/bin/orbit-21055b4` too), `orbit`
ready 01:14:16, then `orbit-agent` (18 s down). `vm-health.conf` (old copy `.before-21055b4`): `PROD_TAG=21055b4`,
`ROLLBACK_TAG=da0cf82`, `ROLLBACK_BACKUP=01a1d79e-4dea-703b-b26b-d220c7d5ce9a`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness 200 and `/health/live` 200; `/orbit-build.json`
revision `21055b4f…`; schema 20261113115539 (114 rows, `20261113101100` present); live quick-check ok, 0 foreign-key
rows; 0 error lines in the journal (only the known "allowed origin without a port" WARN); sidecar `/health` ok and `src`
equal to `main`. Built bundle on production as David Dusi (temporary v7 session, `ui-check.ts`, all API writes blocked;
one blocked: the doc page `visit` POST): project settings sidebar with the 5 groups and 11 section links, Settings →
Docs shows "No docs teamspace", the Docs sidebar lists the teamspaces (`teamspaces` GET 200), #general: the # button
puts "#" in the composer and lists Channels and Tasks (10 rows; the draft is only in `localStorage`); 0 console or page
errors. Screenshots
`/root/.local/share/com.jean.desktop/pasted-images/upstream-port5/prod-{project-settings-sidebar,project-settings-docs,docs-teamspaces,chat-hash-list}.png`.
`ORBIT_VERIFY_SPA=1 verify-production.sh` passed (session logged out, the row deleted after; token file, TLS hop and its
pem removed). `vm-health` OK except oom (the old one of November 13, 03:06), nginx (9 lines, upstream refused in the
switch windows 00:01 and 01:13:59–01:14:16) and docker (other worktrees' containers). **Not** verified on production:
linking a teamspace to a project, a server image shrink of a real upload, sending a # mention (all writes).

Rollback to `da0cf82`. Way 1 keeps the work done after the switch, but loses the teamspace ↔ project links.

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit sqlite3 /var/lib/orbit/orbit.sqlite < /root/ops/upstream-port5/rollback-schema.sql
sudo install -m 755 /usr/local/bin/orbit-da0cf82 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-21055b4 /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# (the same as /root/ops/upstream-port5/rollback.sh, which also keeps a copy of the database first)
# rollback-schema.sql: DROP INDEX teamspaces_project; ALTER TABLE teamspaces DROP COLUMN project_id; delete the version row
# Way 2, only for a data problem (work after 01:13 UTC is lost), Orbit stopped: /usr/local/bin/orbit-da0cf82 backup verify +
#   backup restore 01a1d79e-4dea-703b-b26b-d220c7d5ce9a (as user orbit, with /etc/orbit/orbit.env), then the binary and sidecar lines above
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-21055b4.tgz
# Plain copy of the database before the switch: /var/backups/orbit/pre-migration/orbit-before-21055b4.sqlite
```

After this rollback, set `PROD_TAG=da0cf82`, `ROLLBACK_TAG=e53e93c`,
`ROLLBACK_BACKUP=01a1d75b-3deb-73a0-8a4b-2915ac8820ae` in `/root/ops/vm-health.conf` (copy
`/root/ops/vm-health.conf.before-21055b4`).

### November 14, 2026 upgrade: upstream ports 3 (milestones, cycles) and 4 (schema 20261113115539, 113 rows; 5 migrations)

`main` from `ccaa8839` to `da0cf827` (local, not pushed at the switch): fast-forward of `upstream-port4`. It holds the
upstream port 3 milestones work (`e7ea6528` milestones, cycles and estimates #113, `9266dbf5` carried-over tasks #129,
`b0b94194` Donna tool `list_milestones_cycles`), port 4 (`c8c16734` public API moves, trashes and lists versions of
docs #121, `727d3f9c` `Idempotency-Key` on public task and comment creates #122, `92be1eef` public task list due-date
filter #123, `b9c3e5a0` one MCP OAuth connection for several workspaces #132, `a5d82dc1` S3 file move progress,
`9bd43864` Admin → Storage "Keep deleted files in S3"), merge `10dafbcc` of `main` and `da0cf827`. Merge fixes: the
generated client made again from the merged server; `openapi_contract` 297 paths, 384 operations; `TOOL_CATALOG` 57
tools (`drive_image_to_shopify` + `list_milestones_cycles`); the iPhone Share → new task sends the new milestone,
cycle and estimate fields. Upstream #125 (titles of a database row's sub-page links) is not in this fork: the fork has no
Docs databases. What is new for people: new entry `2026-11-14` (milestones, cycles and estimates; multi-workspace MCP
OAuth, public API docs moves/trash/versions, due-date filter, Idempotency-Key, S3 move progress and keep-days).
`aboutOrbit()` covers all of it.

Migrations (SQLite + Postgres, none destructive): `20261113100600_task_milestones_cycles` (tables `task_milestones`,
`task_cycles`, columns `tasks.milestone_id`, `cycle_id`, `estimate`, triggers), `20261113100700_task_cycle_carryovers`,
`20261113100800_api_idempotency_keys`, `20261113100900_oauth_grant_groups` (`oauth_grants.group_id = id` for existing
grants), `20261113101000_s3_keep_deleted_days` (`instance_settings.s3_keep_deleted_days` default 40). Their versions are
older than the applied `20261113115539`; the runner applies every version that is not in `schema_migrations`, so the
max version stays `20261113115539`.

Checks on the merge (`orbit-build`, `CARGO_PROFILE_*_DEBUG=0`): SQLite `oauth_api` 15, `views_api` 25, `storage_api` 7,
`workspace_storage_api` 3, `tasks_api` 100, `private_projects` 8, `public_api` 34, `mcp_api` 19, `guest_access` 1,
`agent_api` 18, `agent_profiles` 9, `agent_shopify_image` 5, `home_layout_api` 6, `openapi_contract` 19, platform
`database` 55, server `--lib` 292; the same (without `openapi_contract` and `--lib`) with `ORBIT_TEST_POSTGRES_URL`: all
pass. Web build (`tsc -b && vite build`), `tsc` of `packages/api`, `apps/agent`, `apps/mobile`; agent tests 85 pass;
mobile `bun test src` 225 pass. Not run: the iOS bundle on Metro. Scripts and logs: `/root/ops/upstream-port4/`.

Deploy id `20261113T222638Z-962969` (the first id `20261113T201729Z-807761` lost its lease during the tests, before any
step; production was unchanged). `orbit:da0cf82` (image
`sha256:2daf023fcf390c7ac968ad6b62b981d9b7a70c966732f202e2a7d672c1ce805b`, label `da0cf827…`, one cold build);
`deploy/smoke.sh` and `orbit config check` (production env) passed. Before the switch, the release binary's embedded app
on a seeded instance in `unshare -n` (signed in): `/`, `/tasks`, project settings (Milestones and Cycles cards),
`/docs`, `/drive`, `/chat`, `/activity`, `/settings`, `/settings/agentic`, `/admin`, `/admin/storage`, 0 page or
console errors. Migration dry run (release binary on a hot copy, and in the switch on a copy of the stopped database):
5 applied, a second run 0, 113 rows, the 108 old checksums, counts and user names unchanged (9 users, 110 tasks, 6
projects, 1132 chat messages, 0 OAuth grants), `s3_keep_deleted_days` 40, quick-check ok, 0 foreign-key rows. Rollback
test on the same copies: the old binary refuses ("migration 20261113100600 is not known"), `rollback-schema.sql` brings
`sqlite_master` back equal to the source, counts and checksums equal, the old binary applies 0, and the new binary
applies the 5 again. **Production has 0 MCP OAuth grants**, so no existing connection could be tested there. Instead
`oauth-check.sh` (copy of the stopped database, `unshare -n`) added a grant and token pair as `e53e93c` stores them:
before the migration `e53e93c` answered `list_projects` (its `get_me` answers "Resource is not available" for every
OAuth connection, an old bug that port 4 fixes); after the migration the same token got `get_me` and `list_projects`,
`group_id = id`, a refresh worked, and the refreshed token got `get_me`.

Switch: stopped both services at 00:00:43 UTC; snapshot `01a1d75b-3deb-73a0-8a4b-2915ac8820ae` with
`/usr/local/bin/orbit-e53e93c`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-da0cf82.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-da0cf82.tgz`; old sidecar code `/opt/orbit-agent.before-da0cf82`; new
sidecar `src` (`package*.json` unchanged); env unchanged. Binary installed (`/usr/local/bin/orbit-da0cf82` too), `orbit`
ready 00:01:36, then `orbit-agent` (53 s down). `vm-health.conf` (old copy `.before-da0cf82`): `PROD_TAG=da0cf82`,
`ROLLBACK_TAG=e53e93c`, `ROLLBACK_BACKUP=01a1d75b-3deb-73a0-8a4b-2915ac8820ae`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness (all 6 checks) and `/health/live` 200; schema
20261113115539 (113 rows, the 5 new versions present); live quick-check ok, 0 foreign-key rows; 0 error lines in the
journal (only the known "allowed origin without a port" WARN); sidecar `/health` ok and `src` equal to `main`, deployed
`aboutOrbit()` has milestones/cycles, Idempotency-Key and "Keep deleted files in S3". `/.well-known/oauth-protected-
resource/mcp` 200, `/mcp` without a token 401. Built bundle on production as David Dusi (temporary v7 session,
`ui-check.ts`, all API writes blocked, none was attempted): General project settings shows the Milestones and Cycles
cards (`milestones` and `cycles` GET 200); Admin → Storage shows "Keep deleted files in S3" = 40 days; the task list
loads; 0 console or page errors. Screenshots
`/root/.local/share/com.jean.desktop/pasted-images/upstream-port4/prod-{project-milestones-cycles,admin-storage-keep-days,tasks}.png`.
`ORBIT_VERIFY_SPA=1 verify-production.sh` passed (session logged out, the row deleted after; token file, TLS hop and its
pem removed). `vm-health` OK except oom (an older one), nginx (14 lines, upstream refused 00:01:04–00:01:35, the switch
window) and docker (other worktrees' containers). **Not** verified on production: a real MCP OAuth consent with several
workspaces, creating a milestone or cycle, an Idempotency-Key retry (all writes).

Rollback to `e53e93c`. Way 1 keeps the work done after the switch, but loses milestones, cycles, carry-overs, task
estimates, idempotency keys, OAuth grant groups (the extra grants of a multi-workspace consent stay as separate grants)
and the keep-days setting (back to the fixed 40).

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit sqlite3 /var/lib/orbit/orbit.sqlite < /root/ops/upstream-port4/rollback-schema.sql
sudo install -m 755 /usr/local/bin/orbit-e53e93c /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-da0cf82 /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# (the same as /root/ops/upstream-port4/rollback.sh, which also keeps a copy of the database first)
# rollback-schema.sql: drop the 4 task triggers, 2 task indexes, task_cycle_carryovers, tasks.milestone_id/cycle_id/
#   estimate, task_milestones, task_cycles, api_idempotency_keys, oauth_grants_group + group_id,
#   instance_settings.s3_keep_deleted_days; delete the 5 version rows (one transaction)
# Way 2, only for a data problem (work after 00:00 UTC is lost), Orbit stopped: /usr/local/bin/orbit-e53e93c backup verify +
#   backup restore 01a1d75b-3deb-73a0-8a4b-2915ac8820ae (as user orbit, with /etc/orbit/orbit.env), then the binary and sidecar lines above
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-da0cf82.tgz
# Plain copy of the database before the switch: /var/backups/orbit/pre-migration/orbit-before-da0cf82.sqlite
```

After this rollback, set `PROD_TAG=e53e93c`, `ROLLBACK_TAG=4b1b607`,
`ROLLBACK_BACKUP=01a1d688-2f92-74b7-b90d-b0f423e4aa13` in `/root/ops/vm-health.conf` (copy
`/root/ops/vm-health.conf.before-da0cf82`).

### November 13, 2026 upgrade: Donna tool drive_image_to_shopify (no migration)

`main` from `92e1681b` to `e53e93c6` (fast-forward of `drive-to-shopify`, rebased on `92e1681b`): `cd48a1b9` + `b0f93551`
(new tool `drive_image_to_shopify`: `POST /api/v1/agent-tools/shopify-image` uploads a Drive image to a Shopify store's
Files through a ticked Shopify HTTP API integration, optionally as a blog article's image or at the end of its body;
needs the driver's yes; tool catalog 55 → 56), `e53e93c6` (what's new). What is new for people: one line added to
today's `2026-11-13` entry ("Drive pictures to Shopify…", now 5 lines). `aboutOrbit()` names the tool and the
Shopify scopes `write_files` / `write_content`. No migration, no new env key, `package*.json` of the sidecar unchanged.

Checks after the rebase (`orbit-build`, `CARGO_PROFILE_*_DEBUG=0`): SQLite `agent_shopify_image` 5 pass; `apps/agent`
`tsc --noEmit` ok, `npm test` with `/opt/orbit-node` (Node 24) 82 pass, 0 fail (Node 18 in `/usr/bin` cannot run `.ts`
tests). Postgres and the full suites were not run. Scripts and logs: `/root/ops/drive-to-shopify/`.

Deploy id `20261113T185830Z-723278`. `orbit:e53e93c` (image
`sha256:1f76809f9e45cc7cfe6e6973ce32a88b110d55c089c7168c480468fd5325506d`, label `e53e93c6…`, one cold build, 50 min with the
wait for the build lock); `deploy/smoke.sh` and `orbit config check` (production env) passed. Migration dry run
(release binary on a hot copy, and in the switch on a copy of the stopped database): 0 applied, schema 20261113115539
(108 rows), counts, names and checksums unchanged (9 users, 110 tasks, 1131 chat messages), quick-check ok, 0
foreign-key rows.

Switch: stopped both services at 20:10:11 UTC; snapshot `01a1d688-2f92-74b7-b90d-b0f423e4aa13` with
`/usr/local/bin/orbit-4b1b607`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-e53e93c.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-e53e93c.tgz`; old sidecar code `/opt/orbit-agent.before-e53e93c`; new
sidecar `src`; env unchanged. Binary installed (`/usr/local/bin/orbit-e53e93c` too), `orbit` ready 20:10:30, then
`orbit-agent` (19 s down). `vm-health.conf` (old copy `.before-e53e93c`): `PROD_TAG=e53e93c`, `ROLLBACK_TAG=4b1b607`,
`ROLLBACK_BACKUP=01a1d688-2f92-74b7-b90d-b0f423e4aa13`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness (all 6 checks) and `/health/live` 200; schema
20261113115539 (108 rows), quick-check ok; 0 error lines in the journal; sidecar `src` equal to `main`, its
`ORBIT_TOOLS` (52) has `drive_image_to_shopify`; deployed `aboutOrbit()` names the tool, `write_files` and
`write_content`. `GET /workspaces/{Wellrox}/agents` without a session 401; with a temporary v7 session of David Dusi
(inserted live, logged out 204 after, row deleted, token file removed) 200: the tool catalog has 56 tools with
`drive_image_to_shopify` (Connected services, writes), so Donna (the default agent, all tools) has it; the custom
agent Arthur does not (off until ticked). `vm-health` OK except oom (the 03:06 one), nginx (the switch window),
docker (other worktrees' containers) and git-owner (other worktrees' metadata in `.git`). **Not** verified on
production: a real upload to Shopify (a write in the store).

Rollback to `4b1b607` (no schema change, so no data is lost):

```sh
sudo systemctl stop orbit-agent orbit
sudo install -m 755 /usr/local/bin/orbit-4b1b607 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-e53e93c /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# (the same as /root/ops/drive-to-shopify/rollback.sh)
# Way 2, only for a data problem (work after 20:10 UTC is lost), Orbit stopped: /usr/local/bin/orbit-4b1b607 backup verify +
#   backup restore 01a1d688-2f92-74b7-b90d-b0f423e4aa13 (as user orbit, with /etc/orbit/orbit.env), then the lines above
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-e53e93c.tgz
# Plain copy of the database before the switch: /var/backups/orbit/pre-migration/orbit-before-e53e93c.sqlite
```

After this rollback, set `PROD_TAG=4b1b607`, `ROLLBACK_TAG=d5dfd47`,
`ROLLBACK_BACKUP=01a1d63c-8b2f-77a1-9f83-4360004da19b` in `/root/ops/vm-health.conf` (copy
`/root/ops/vm-health.conf.before-e53e93c`).

### November 13, 2026 upgrade: iPhone release server part (Home cards, app icon badge; schema 20261113115539)

`main` from `0cb2486a` to `4b1b6071` (local, not pushed): merge `05079580` of `iphone-home-cards` (`7a357b5e`: contains
`iphone-followup` `03cba6e6` with the server APNs badge, `iphone-settings` `49bb83d5`, `home-cards` `dad0e3c3`).
Conflicts, both sides kept: `guest_access.rs` (unread threads + home layout routes), `attachments_api.rs` (both new
tests), `openapi_contract.rs` (368 operations; the path count 286 came in `881fdfb2`, a test-only commit after the
build), what's new; the generated client made again from the merged server (every path and schema of both sides).
`4b1b6071` fixes a semantic conflict: port 3 replaced `unreadConversations`/`useUnreadConversations` with
`unreadGroups`, so the Home "Unread chat" card (web and iPhone) now takes the conversation groups of `unreadGroups`
(unread conversations only, as before). New: `GET/PUT /workspaces/{id}/home-layout` (one layout per user, web and
iPhone), `GET /push/badge` (unread chat + unread inbox, all workspaces) and the APNs badge count (APNs is still off
here). What is new for people: today's `2026-11-13` entry has the full iPhone line (Drive, badge, Share → Orbit,
settings), a Home cards line, and the port 3 Chat line merged with the MCP/sessions line.

Migration (SQLite + Postgres): `20261113115539_home_layouts` (new table `home_layouts`, no data change).

Checks on `main` (`orbit-build`, `CARGO_PROFILE_*_DEBUG=0`): SQLite `home_layout_api` 6, `attachments_api` 34,
`chat_api` 38 (1 ignored), `chat_live` 9, `guest_access` 1, `agent_api` 18, `app_smoke` 21, lib `push` 9, platform
`database` 55; `openapi_contract` failed on the path count (284 → 286, fixed in `881fdfb2`; its rerun with
`push_devices_api` on SQLite was queued behind other sessions' builds); with `ORBIT_TEST_POSTGRES_URL` the same plus
`push_devices_api` 4: all pass. Web build (`tsc -b && vite build`), `tsc` of `packages/api`, `apps/agent`,
`apps/mobile`; mobile `bun test src` (224 pass), web home/shell/chat wire tests (19 pass), agent `npm test` (pass).
Scripts and logs: `/root/ops/iphone-release/`.

Deploy id `20261113T164413Z-589869`. `orbit:4b1b607` (image
`sha256:f8b9bf3b8f3ca1eab0799bd689bca1a82cdf0024d256bc05170e4e3850253489`, label `4b1b6071…`, one cold build of 39 min);
`deploy/smoke.sh` and `orbit config check` (production env) passed. Migration dry run (release binary on a hot copy,
and in the switch on a copy of the stopped database): 1 applied, a second run 0, schema 20261113115539 (108 rows), the
107 old checksums, counts and user names unchanged (9 users, 110 tasks, 1130 chat messages), 0 `home_layouts` rows,
quick-check ok, 0 foreign-key rows. Rollback test on the same copies: the old binary refuses, `rollback-schema.sql`
brings `sqlite_master` back equal to the source, counts and checksums equal, the old binary applies 0.

Switch: stopped both services at 18:47:34 UTC; snapshot `01a1d63c-8b2f-77a1-9f83-4360004da19b` with
`/usr/local/bin/orbit-d5dfd47`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-4b1b607.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-4b1b607.tgz`; old sidecar code `/opt/orbit-agent.before-4b1b607`; new
sidecar `src` (`package*.json` unchanged); env unchanged. `switch.sh` stopped after `config check` with exit 127
(`bun` is not in `sudo`'s `PATH` for the test-session id; fixed to `/root/.bun/bin/bun`); its last part ran at once
as `switch-rest.sh`: binary installed (`/usr/local/bin/orbit-4b1b607` too), `orbit` ready 18:48:04, then
`orbit-agent` (30 s down). `vm-health.conf` (old copy `.before-4b1b607`): `PROD_TAG=4b1b607`, `ROLLBACK_TAG=d5dfd47`,
`ROLLBACK_BACKUP=01a1d63c-8b2f-77a1-9f83-4360004da19b`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness (all 6 checks) and `/health/live` 200; schema
20261113115539 (108 rows); live quick-check ok, 0 foreign-key rows; 0 error lines in the journal (only the known
"allowed origin without a port" WARN); sidecar `/health` ok and `src` equal to `main`; deployed `aboutOrbit()` has
"Customize", "iPhone", "Share" and port 3's "Unread replies" and "Two-way bots". Without a session `GET push/badge` and
`GET workspaces/{id}/home-layout` answer 401; with one 200 (`{"count":1}`; the default 5 cards, `customized: false`).
Built bundle on production as David Dusi (temporary v7 session, `ui-check.ts`, all API writes blocked, none was
attempted): Home loads with `home-layout` 200 and 5 cards (Overdue, Due soon, Unread in Inbox, Recently updated docs,
Unread chat with 2 conversations); Customize shows Add card and the Remove buttons, Done closes it; `push/badge` 200
from the page; Unreads loads with `unread-threads` 200 and 3 groups; 0 console or page errors. Screenshots
`/root/.local/share/com.jean.desktop/pasted-images/iphone-release/prod-{home,home-customize,unreads}.png`.
`ORBIT_VERIFY_SPA=1 verify-production.sh` passed (signed out `/` → `/login`, signed in `/` and `/tasks`; session logged
out, the row deleted after; token file, TLS hop and its pem removed). `vm-health` OK except disk (81%), oom (the 03:06
one), nginx (12 lines, upstream refused 18:47:34–18:48:04, the switch window) and docker (other worktrees'
containers). **Not** verified on production: saving a Home layout (a write), an APNs badge push (APNs is off).

Rollback to `d5dfd47`. Way 1 keeps the work done after the switch, but loses saved Home layouts.

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit sqlite3 /var/lib/orbit/orbit.sqlite < /root/ops/iphone-release/rollback-schema.sql
sudo install -m 755 /usr/local/bin/orbit-d5dfd47 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-4b1b607 /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# (the same as /root/ops/iphone-release/rollback.sh, which also keeps a copy of the database first)
# rollback-schema.sql: DROP TABLE home_layouts; delete the version row 20261113115539 (one transaction)
# Way 2, only for a data problem (work after 18:47 UTC is lost), Orbit stopped: /usr/local/bin/orbit-d5dfd47 backup verify +
#   backup restore 01a1d63c-8b2f-77a1-9f83-4360004da19b (as user orbit, with /etc/orbit/orbit.env), then the binary and sidecar lines above
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-4b1b607.tgz
# Plain copy of the database before the switch: /var/backups/orbit/pre-migration/orbit-before-4b1b607.sqlite
```

After this rollback, set `PROD_TAG=d5dfd47`, `ROLLBACK_TAG=68b2d48`,
`ROLLBACK_BACKUP=01a1d5c1-30a9-7617-b5e3-732fab04b6f6` in `/root/ops/vm-health.conf` (copy
`/root/ops/vm-health.conf.before-4b1b607`).

### November 13, 2026 upgrade: upstream port 3 (unread thread replies, two-way webhook bots; schema 20261113100500, one destructive)

`main` from `6fda5792` to `d5dfd476` (local, not pushed): `upstream-port3` (7 commits, already rebased on `6fda5792`)
fast-forwarded. `c8b91fd2` Unreads shows threads with new replies, followed or not (#99; `GET chat/unread-threads`);
`c5b03012` + `b153c8cb` chat webhooks can be two-way bots (#98: "Send messages as events" sends each member's message,
signed, to the request URL; the webhook secret can read the channel at `GET /api/v1/hooks/chat/{id}/messages`); they
replace the fork's webhook bots (Settings → Agentic → Bots, `webhook_bots` code, routes and `docs/webhook-bots.md` are
gone); `0da8926a` `aboutOrbit()`; `efa62cf4` generated client; `78a7c62c` mobile unread threads; `d5dfd476` tests. What
is new for people: today's `2026-11-13` entry gets a Chat line (Unreads threads, two-way bots, Bots tab gone); the
`2026-11-12` "Bots that answer" line is removed.

Migrations (SQLite + Postgres): `20261113100300_chat_thread_unreads` (`chat_members.threads_read_to`, set to the newest
message id), `20261113100400_chat_webhook_events` (`chat_webhooks.events` default 0),
`20261113100500_drop_webhook_bots.destructive` (suspends and renames live bot users, drops `webhook_bot_deliveries`,
`webhook_bot_channels`, `webhook_bots`). **Production had 0 `webhook_bots` rows** (0 live, 0 channels, 0 deliveries;
counted read-only before the merge and again by `switch.sh` with Orbit stopped, a gate that would have stopped the
switch), so the drop lost no data and no user was renamed. The server makes and verifies its own pre-migration backup
before a destructive migration. The old binary refuses the migrated database.

Checks on `main` (`orbit-build`, `CARGO_PROFILE_*_DEBUG=0`): SQLite `chat_apps` 5, `chat_api` 37 (1 ignored),
`chat_live` 9, `guest_access` 1, `agent_api` 18, `openapi_contract` 19, `app_smoke` 21, platform `database` 55; the
same (without `openapi_contract`, `app_smoke`) with `ORBIT_TEST_POSTGRES_URL`: all pass. Web build (`tsc -b && vite
build`); `tsc` of `packages/api`, `apps/agent`, `apps/mobile`; web `bun test` of the 4 touched files (25 pass), mobile
`chat.test.ts` (25 pass), agent `files.test.ts` (pass). `target` removed after. Scripts and logs:
`/root/ops/upstream-port3/`.

Deploy id `20261113T150221Z-504225`. `orbit:d5dfd47` (image
`sha256:a863be61e13e16cc4bd78798c550b1bd515748622a69d28138c5dfabcf72590a`, label `d5dfd476…`); `deploy/smoke.sh` and
`orbit config check` (production env) passed. Migration dry run (release binary on a hot copy, and in the switch on a
copy of the stopped database): 3 applied, a second run 0, schema 20261113100500 (107 rows), the 104 old checksums, the
counts and all user names unchanged (8 users, 0 suspended, 110 tasks, 1121 chat messages, 85 chat members, 24
conversations, 2 chat webhooks), 0 bot tables left, `threads_read_to` = newest message for 85 of 85 members,
quick-check ok, 0 foreign-key rows. Rollback test on the same copies: the old binary refuses, `rollback-schema.sql`
brings `sqlite_master` back equal to the source, counts and checksums equal, the old binary applies 0.

Switch: stopped both services at 16:32:50 UTC; snapshot `01a1d5c1-30a9-7617-b5e3-732fab04b6f6` with
`/usr/local/bin/orbit-68b2d48`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-d5dfd47.sqlite`; the server's own pre-migration backup
`/var/backups/orbit/pre-migration/01a1d5c1-8440-7583-b0d2-af7ccf1694fd`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-d5dfd47.tgz`; old sidecar code `/opt/orbit-agent.before-d5dfd47`; new
sidecar `src` (`package*.json` unchanged); env unchanged; binary installed (`/usr/local/bin/orbit-d5dfd47` too); `orbit`
ready 16:33:12, then `orbit-agent`. `vm-health.conf` (old copy `.before-d5dfd47`): `PROD_TAG=d5dfd47`,
`ROLLBACK_TAG=68b2d48`, `ROLLBACK_BACKUP=01a1d5c1-30a9-7617-b5e3-732fab04b6f6`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness (all 6 checks) and `/health/live` 200; schema
20261113100500 (107 rows); live quick-check ok, 0 foreign-key rows, 0 bot tables; 0 error lines in the journal (only
the known "allowed origin without a port" WARN); sidecar `/health` ok and `src` equal to `main`; deployed `aboutOrbit()`
has "Two-way bots", "Send messages as events" and "Unread replies: Unreads shows each thread", and no Bots tab in the
Agentic list. Without a session `GET chat/unread-threads` answers 401; `GET hooks/chat/{id}/messages` without its
secret answers 404 `webhook_not_found` (by design, as the hook POST: also for a real webhook id); the old
`/workspaces/{id}/bots` route 404. Built bundle on production as David Dusi (temporary v7 session, `ui-check.ts`, all
API writes blocked: the one write, `POST …/conversations/{id}/read` on opening #orbit-updates, was blocked): Unreads
loads with `unread-threads` 200 and 1 thread group; #orbit-updates → Edit channel → Integrations → Updates →
Interactivity shows the Request URL field; Settings → Agentic has Routines and no Bots tab; 0 console or page errors.
Screenshots `/root/.local/share/com.jean.desktop/pasted-images/upstream-port3/prod-{unreads,channel-integrations,agentic-tabs}.png`.
`ORBIT_VERIFY_SPA=1 verify-production.sh` passed (signed out `/` → `/login`, signed in `/` and `/tasks`; session logged
out, the row deleted after). `vm-health` OK except oom (1 line in 24 h, earlier), nginx (20 lines, upstream refused
16:32:50–16:33:12, the switch window), docker (other worktrees' containers) and git-owner. **Not** verified on
production: turning on events for a webhook, a signed event delivery, a bot reply, opening a thread to clear it (no
user data changed).

Rollback to `68b2d48`. Way 1 keeps the work done after the switch, but loses what only port 3 stores: webhook "events"
settings and thread read positions; the bot tables come back empty (as they were).

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit sqlite3 /var/lib/orbit/orbit.sqlite < /root/ops/upstream-port3/rollback-schema.sql
sudo install -m 755 /usr/local/bin/orbit-68b2d48 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-d5dfd47 /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# (the same as /root/ops/upstream-port3/rollback.sh, which also keeps a copy of the database first)
# rollback-schema.sql: DROP COLUMN chat_members.threads_read_to, chat_webhooks.events; CREATE the 3 webhook_bot tables
#   and 4 indexes with their exact old text; delete the 3 version rows (one transaction)
# Way 2, only for a data problem (work after 16:32 UTC is lost), Orbit stopped: /usr/local/bin/orbit-68b2d48 backup verify +
#   backup restore 01a1d5c1-30a9-7617-b5e3-732fab04b6f6 (as user orbit, with /etc/orbit/orbit.env), then the binary and sidecar lines above
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-d5dfd47.tgz
# Plain copy of the database before the switch: /var/backups/orbit/pre-migration/orbit-before-d5dfd47.sqlite
```

After this rollback, set `PROD_TAG=68b2d48`, `ROLLBACK_TAG=18568f9`,
`ROLLBACK_BACKUP=01a1d564-40f0-740c-9831-68328951d98d` in `/root/ops/vm-health.conf` (copy
`/root/ops/vm-health.conf.before-d5dfd47`).

### November 13, 2026 upgrade: upstream fixes (new-task description uploads, 5xx logging, info tips; no migration)

`main` from `e06631e6` to `68b2d488` (local, not pushed): `upstream-fixes` (10 commits on `e06631e6`) fast-forwarded.
`e0bfc4f4` images and files in a new task's description (/image, /file) stay as `blob:` URLs until the task exists, then
upload as task attachments; `027968cb` info icons get a pointer and open their tooltip after 100 ms; `8bd700e9` admin
"Files in S3"; `371331d5` a deleted backup also loses its S3 copy; `41507914`, `1204bf25`, `f46f3195` log the cause,
path and request of every 5xx (avatars whose file is gone are dropped); `6134c0c8` test; `5ffbe311` what's new (today's
`2026-11-13` entry, tasks line: "Images and files in a new task's description are now saved with it") and `aboutOrbit()`;
`68b2d488` lesson. Checks: web `tsc -b` (merged main); the image build ran `tsc -b && vite build` and the release
server build. The branch was tested in its worktree. No migration: the new binary on a hot copy applies 0 (schema
20261113100200, 104 rows); copy deleted. Scripts and logs: `/root/ops/upstream-fixes/`.

Deploy id `20261113T141157Z-463767`. `orbit:68b2d48` (image
`sha256:80cd2eabe3003ecc6db25e7038e96ee29f09e884d9092e1ffe474f137ca0b05e`, label `68b2d488…`, 31 min after the
`orbit-build` lock of other worktrees); `deploy/smoke.sh` and `orbit config check` (production env) passed.

Switch: stopped both services at 14:51:19 UTC; snapshot `01a1d564-40f0-740c-9831-68328951d98d` with
`/usr/local/bin/orbit-18568f9`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-68b2d48.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-68b2d48.tgz`; old sidecar code `/opt/orbit-agent.before-68b2d48`; new
sidecar `src` (`package*.json` unchanged); env unchanged; binary installed (`/usr/local/bin/orbit-68b2d48` too); `orbit`
ready 14:51:36, then `orbit-agent`. `vm-health.conf` (old copy `.before-68b2d48`): `PROD_TAG=68b2d48`,
`ROLLBACK_TAG=18568f9`, `ROLLBACK_BACKUP=01a1d564-40f0-740c-9831-68328951d98d`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness (all 6 checks) and `/health/live` 200; revision
`68b2d488`; schema 20261113100200 (104 rows, unchanged); live quick-check ok; 0 error lines in the journal (only the
known "allowed origin without a port" WARN); sidecar `src` equal to `main`. Built bundle on production as David Dusi
(temporary v7 session, `ui-check.ts`, all API writes blocked in the browser): Tasks → New task → description `/image`
→ upload a PNG → the `blob:` image shows in the editor (dialog closed, no task created); Project settings → the Tag
info icon has `cursor: pointer` and its tooltip opens in ~200 ms; 0 console or page errors. Screenshots
`/root/.local/share/com.jean.desktop/pasted-images/upstream-fixes/prod-{new-task-slash-menu,new-task-image,info-tooltip}.png`.
`ORBIT_VERIFY_SPA=1 verify-production.sh` passed (signed out `/` → `/login`, signed in `/` and `/tasks`; session logged
out, the row deleted after: 46 sessions, as before). `vm-health` OK except oom (1 line from 03:06 UTC, an earlier
build), nginx (19 lines, upstream refused 14:51:35–14:51:36, the start window) and docker (other worktrees'
containers). **Not** verified on production: creating a task with an image (the upload after create), the 5xx log
lines, a backup delete with S3.

Side effect: an earlier run of the check script (before the write block) picked the chat bar's file input and
uploaded the test PNG as an unsent chat file of David Dusi (`chat_message_files` `01a1d565-81a3-71eb-a628-4a134852d773`,
blob `01a1d565-81a2-7263-b147-82432031e959`, 574 bytes, no message, no conversation; `POST chat/dms` opened his
existing DM). Nobody else can see it. The daily retention job deletes unsent chat files after 24 h, then the hourly
attachment job removes the blob; not deleted by hand.

Rollback to `18568f9` (no migration, env unchanged):

```sh
sudo systemctl stop orbit-agent orbit
sudo install -m 755 /usr/local/bin/orbit-18568f9 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-68b2d48 /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# (the same as /root/ops/upstream-fixes/rollback.sh)
# Data, only if needed (work after 14:51 UTC is lost), Orbit stopped: /usr/local/bin/orbit-18568f9 backup verify +
#   backup restore 01a1d564-40f0-740c-9831-68328951d98d (as user orbit, with /etc/orbit/orbit.env)
```

After this rollback, set `PROD_TAG=18568f9`, `ROLLBACK_TAG=7b548ac`,
`ROLLBACK_BACKUP=01a1d4af-7c96-711d-ab7b-ceb6798d5ff3` in `/root/ops/vm-health.conf` (copy
`/root/ops/vm-health.conf.before-68b2d48`).

### November 13, 2026 upgrade: upstream port 2 (trash, recurring tasks, private projects, project order; schema 20261113100200)

`main` from `53a80bc0` to `18568f91` (local, not pushed): `upstream-port2` (13 commits on `c5735ed8`, old tip
`ba21bbda`) rebased onto main in its worktree, main fast-forwarded. Rebased commits `f7a6063a` (trash: delete forever, empty trash, search, read-only previews) …
`77ce3d70` (lesson), plus 3 new: `fdc4d273` (what's new), `607f6ada` (generated client), `18568f91` (follower test).
Conflicts: `openapi_contract.rs` counts (main 272/351 + branch 10 paths/12 operations = 282 paths, 363 operations) and
`sdk.gen.ts` (branch side, then made again from the merged binary: `orbit openapi` gives an `openapi.json` equal to the
merged file, `types.gen.ts` unchanged, `sdk.gen.ts` gets back main's push device and Drive file content calls).
`prompt.ts` and `whats-new.md` merged without a conflict. `aboutOrbit()` keeps main's iPhone, hooks and sheet-grid text
and has private projects, recurring tasks, sidebar order and the trash previews / delete forever. What is new for
people: the branch's `2026-11-14` entry folded into today's `2026-11-13` entry, still 4 lines (all 4 rewritten shorter;
line 2 is the tasks line: private projects, Repeat, own sidebar order, trash search / delete forever, # task links).

Security (private projects): tests cover hidden tasks, comments, relations, search (REST and the public API that MCP
uses), notifications (a mention of a non-member, inbox after removal), label rules, Donna's session and guests. Follower
scoping (`989859bc`) had no test: added `a_member_taken_off_a_private_project_stops_following_its_tasks`
(`private_projects.rs`); it fails without the filter in `followers_in_tx` (checked, file restored).

Migrations (SQLite + Postgres): `20261113100000_task_recurrences` (table `task_recurrences`, `tasks.recurrence_id`,
`tasks.recurrence_next_id`, index), `20261113100100_private_projects` (`projects.private` default 0, table
`project_members`), `20261113100200_project_positions` (table `project_positions`); additive. The old binary refuses
the migrated database ("schema version 20261113100200 is newer than binary schema version 20261113042745").

Checks on the rebased tree (= `main`; `orbit-build`, `CARGO_PROFILE_*_DEBUG=0`): SQLite `private_projects` 7,
`recurring_tasks_api` 3, `task_trash_api` 2, `views_api` 25 (project order), `label_rules_api` 9,
`task_followers_api` 5, `agent_api` 18, `guest_access` 1, `public_api` 30, `openapi_contract` 19, `app_smoke` 21,
platform `database` 54 and `backup` 23; the same (without `openapi_contract`, `app_smoke`) with
`ORBIT_TEST_POSTGRES_URL`: all pass. Web `tsc -b`; web `bun test` of the 11 touched test files (72 pass); `packages/api`
`projectOrder` + `recurrence` (7 pass); agent `tsc` and `npm test` with Node 24 (80 pass, 11 skipped, 0 fail). Scripts
and logs: `/root/ops/upstream-port2/`.

Deploy id `20261113T094334Z-251697`. `orbit:18568f9` (image
`sha256:e0b1d43cde95ef4929ea9dad3e428ca505a1b2cb134b94d898dbccab1f114d3c`, label `18568f91…`); `deploy/smoke.sh` and
`orbit config check` (production env) passed. Migration dry run (debug binary on a hot copy, release binary on a hot
copy, and in the switch on a copy of the stopped database): 3 applied, a second run 0, schema 20261113100200 (104
rows), the 101 old checksums and the counts unchanged (8 users, 110 tasks, 18 comments, 102 followers, 94
notifications, 6 projects, 12 memberships), quick-check ok, 0 foreign-key rows, no backup file made. Rollback test on
the same copies: the old binary refuses, `rollback-schema.sql` (DROP steps) brings the schema text back equal to the
source, counts equal, the old binary applies 0.

Switch: stopped both services at 11:33:52 UTC; snapshot `01a1d4af-7c96-711d-ab7b-ceb6798d5ff3` with
`/usr/local/bin/orbit-7b548ac`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-18568f9.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-18568f9.tgz`; old sidecar code `/opt/orbit-agent.before-18568f9`; new
sidecar `src` (`package*.json` unchanged); env unchanged; binary installed (`/usr/local/bin/orbit-18568f9` too); `orbit`
ready 11:34:11, then `orbit-agent`. `vm-health.conf` (old copy `.before-18568f9`): `PROD_TAG=18568f9`,
`ROLLBACK_TAG=7b548ac`, `ROLLBACK_BACKUP=01a1d4af-7c96-711d-ab7b-ceb6798d5ff3`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness (all 6 checks) and `/health/live` 200; revision
`18568f91`; schema top 20261113100200 (104 rows); live quick-check ok, 0 foreign-key rows; 0 private projects, 0
`project_members` rows; 0 error lines in the journal (only the known "allowed origin without a port" WARN); sidecar
`/health` ok and `src` equal to `main`. Without a session all 12 new operations answer 401 (`PUT project-order` and
`POST projects/{id}/members` with a valid body; with `{}` they give 400 first). Existing projects: as David Dusi (owner)
with a temporary v7 session, `GET projects` lists 2 of 2 live projects in Wellrox and in Slate, `GET project-order`
200; in the database every membership sees all live projects of its workspace (members 17 of 17, owners 5 of 5).
`ORBIT_VERIFY_SPA=1 verify-production.sh` passed (signed out `/` → `/login`, signed in `/` and `/tasks`, 0 console or
page errors; session logged out, the row deleted after: 47 sessions, as before). `vm-health` OK except oom (1 line from
03:06 UTC, an earlier build), nginx (19 lines, upstream refused 11:34:01–11:34:11, the start window), docker (other
worktrees' containers) and git-owner. **Not** verified on production: creating a private project, a repeat rule, a
sidebar reorder, or delete forever (no user data changed).

Rollback to `7b548ac`. Way 1 keeps the work done after the switch, but loses what only port 2 stores: private
projects become workspace-wide again (everyone sees them), member lists, repeat rules and sidebar orders are gone. If a
private project exists by then, check with its owners before Way 1, or use Way 2.

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit sqlite3 /var/lib/orbit/orbit.sqlite < /root/ops/upstream-port2/rollback-schema.sql
sudo install -m 755 /usr/local/bin/orbit-7b548ac /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-18568f9 /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# (the same as /root/ops/upstream-port2/rollback.sh, which also keeps a copy of the database first)
# rollback-schema.sql: DROP TABLE project_positions, project_members; DROP COLUMN projects.private; DROP INDEX
#   tasks_recurrence_pending; DROP COLUMN tasks.recurrence_next_id, tasks.recurrence_id; DROP TABLE task_recurrences;
#   delete the 3 version rows (one transaction)
# Way 2, only for a data problem (work after 11:33 UTC is lost), Orbit stopped: /usr/local/bin/orbit-7b548ac backup verify +
#   backup restore 01a1d4af-7c96-711d-ab7b-ceb6798d5ff3 (as user orbit, with /etc/orbit/orbit.env), then the binary and sidecar lines above
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-18568f9.tgz
# Plain copy of the database before the switch: /var/backups/orbit/pre-migration/orbit-before-18568f9.sqlite
```

After this rollback, set `PROD_TAG=7b548ac`, `ROLLBACK_TAG=f14e780`,
`ROLLBACK_BACKUP=01a1d448-08e5-76ee-9499-bb1be86b74d9` in `/root/ops/vm-health.conf` (copy
`/root/ops/vm-health.conf.before-18568f9`).

### November 13, 2026 upgrade: sheet grid viewer (xlsx, xls, csv, tsv; no migration)

`main` from `3463ad1d` to `7b548ac0` (local, not pushed): `sheet-grid` (`8b2d3b55`, on an older main `bdc34371`)
cherry-picked onto main. New route `GET /api/v1/workspaces/{id}/drive/nodes/{id}/content` (`drive_content_routes.rs`:
member, not guest, live file of the workspace, files up to 16 MiB); web `SheetPreview` in the chat viewer and
`DriveViewer` (Word and PowerPoint keep the PDF; the PDF is the fallback). Conflicts: `openapi_contract.rs` (counts
merged: 272 paths, 351 operations), `whats-new.md`, generated client (`index.ts` deleted on main stays deleted;
`openapi.json` and the client made again from the merged server with `orbit openapi` + `api:generate`: `openapi.json` and
`types.gen.ts` equal the auto-merge, `sdk.gen.ts` gets `getDriveFileContent`). What is new for people: today's
`2026-11-13` entry stays 4 lines; line 3 starts with the sheet sentence (xlsx, xls, csv, tsv in chat or Drive open as a
read-only table with a tab for each sheet), then the bell and pickers sentences shortened into one. `aboutOrbit()` has the
branch's sheet text under "Drive" and "Chat".

Checks on merged `main` (through `orbit-build`, `CARGO_PROFILE_*_DEBUG=0`): `drive_api` (11), `openapi_contract` (19),
`app_smoke` (21) on SQLite; `drive_api` (11) with `ORBIT_TEST_POSTGRES_URL`; web `tsc -b` and `bun test
src/lib/filePreview.test.ts` (3 pass); agent `tsc` and `npm test` with Node 24 (80 pass, 11 skipped, 0 fail). No
migration: the new binary on a hot copy applies 0 (schema 20261113042745, 101 rows). Scripts: `/root/ops/sheet-grid/`.

Deploy id `20261113T080829Z-184059`. `orbit:7b548ac` (image
`sha256:25735ec548f00a2460d821a4829496f017ea3fd1448097fe9d965966ae8789a8`, waited for the build lock behind other
worktrees); `deploy/smoke.sh` and `orbit config check` (production env) passed.

Switch: stopped both services at 09:40:52 UTC; snapshot `01a1d448-08e5-76ee-9499-bb1be86b74d9` with
`/usr/local/bin/orbit-f14e780`, verified; plain copy `/var/backups/orbit/pre-migration/orbit-before-7b548ac.sqlite`;
sidecar data `/var/backups/orbit-agent/orbit-agent-before-7b548ac.tgz`; old sidecar code
`/opt/orbit-agent.before-7b548ac`; new sidecar `src` (`package*.json` unchanged); env unchanged; binary installed
(`/usr/local/bin/orbit-7b548ac` too); `orbit` ready 09:41:09, then `orbit-agent`. `vm-health.conf` (old copy
`.before-7b548ac`): `PROD_TAG=7b548ac`, `ROLLBACK_TAG=f14e780`, `ROLLBACK_BACKUP=01a1d448-08e5-76ee-9499-bb1be86b74d9`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness (all 6 checks) and `/health/live` 200; revision
`7b548ac0`; schema 20261113042745 (101 rows, unchanged); 0 error lines in the journal (only the known "allowed origin
without a port" WARN); sidecar `/health` ok and `src` equal to `main`; the new content route without a session 401.
Sheet grid with a temporary v7 session (David Dusi), reads only: Drive → Wellrox → the existing
`adjust_ETM_Inventory_Import_2026-11-09.xlsx` opened as the table (content request 200, rows shown, 0 console or page
errors; screenshot `/root/.local/share/com.jean.desktop/pasted-images/sheet-grid/drive-xlsx-sheet-grid.png`).
`ORBIT_VERIFY_SPA=1 verify-production.sh` passed (signed out `/` → `/login`, signed in `/` and `/tasks`, 0 console or
page errors; session logged out, the row deleted after: 46 sessions, as before). `vm-health` OK except oom (3 lines from
03:06 UTC, an earlier build), nginx (16 lines, upstream refused 09:40:57–09:41:08, the start window), docker (another
worktree's `d2s-s3` container) and git-owner. **Not** verified on production: a csv/tsv file, a workbook with several
sheets, and the sheet grid in chat (no such files there).

Rollback to `f14e780` (no migration, env unchanged):

```sh
sudo systemctl stop orbit-agent orbit
sudo install -m 755 /usr/local/bin/orbit-f14e780 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-7b548ac /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# (the same as /root/ops/sheet-grid/rollback.sh)
# Data, only if needed (work after 09:40 UTC is lost), Orbit stopped: /usr/local/bin/orbit-f14e780 backup verify +
#   backup restore 01a1d448-08e5-76ee-9499-bb1be86b74d9 (as user orbit, with /etc/orbit/orbit.env)
```

After this rollback, set `PROD_TAG=f14e780`, `ROLLBACK_TAG=08067fd`,
`ROLLBACK_BACKUP=01a1d3f0-85dc-73c7-ba6e-4f293864e460` in `/root/ops/vm-health.conf` (copy
`/root/ops/vm-health.conf.before-7b548ac`).

### November 13, 2026 upgrade: public hooks URL (`ORBIT_PUBLIC_HOOKS_URL`, no migration)

`main` from `c6c03fc5` to `f14e7807` (local, not pushed): `public-hooks-url` (`88756016`, on an older main)
cherry-picked onto main. Conflict only in `whats-new.md`: the branch's own `2026-11-13` entry was folded into today's
entry, still 4 lines (one sentence added to line 4: webhook URLs start with https://orbit-hooks.easytomanage.com, no
login; for an older webhook change only the start of its URL). `aboutOrbit()` keeps the branch's hooks sentence under
"Chat" (Incoming webhooks). New setting `ORBIT_PUBLIC_HOOKS_URL`: origin of the incoming webhook URL, the "Try it"
command and each click's `response_url`; unset = `ORBIT_PUBLIC_ORIGIN`; a trailing slash is trimmed, a path or query is
refused at startup; `orbit config show` prints it. Webhook **bots'** reply URLs still use `ORBIT_PUBLIC_ORIGIN`.

Checks on merged `main` (through `orbit-build`, `CARGO_PROFILE_*_DEBUG=0`): platform `config::` unit tests (5 pass);
`public_hooks_url` (2), `webhook_bots` (9), `openapi_contract` (19), `app_smoke` (21) on SQLite; `public_hooks_url` and
`webhook_bots` again with `ORBIT_TEST_POSTGRES_URL`; agent `tsc` and `npm test` with Node 24 (80 pass, 11 skipped, 0 fail).

Deploy id `20261113T063711Z-107115`. `orbit:f14e780` (image
`sha256:abda8967f005b235cb4dc39e03a7f2e615777a74726973017cb9ead83f7cfbd8`, waited for the build lock behind
`upstream-port2`); `deploy/smoke.sh` passed. `orbit config check` / `config show` of the new binary with the production env
plus the new key: valid, `ORBIT_PUBLIC_HOOKS_URL=https://orbit-hooks.easytomanage.com`; with `https://x.example/api`
refused; the old binary refuses the key ("unknown Orbit environment setting"). Scripts: `/root/ops/public-hooks/`.

Switch: stopped both services at 08:05:17 UTC; snapshot `01a1d3f0-85dc-73c7-ba6e-4f293864e460` with
`/usr/local/bin/orbit-08067fd` and the old env, verified; plain copy
`/var/backups/orbit/pre-migration/orbit-before-f14e780.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-f14e780.tgz`; old sidecar code `/opt/orbit-agent.before-f14e780`; new
sidecar `src` (`package*.json` unchanged); env copy `/etc/orbit/orbit.env.before-hooks`, then the key added; binary
installed (`/usr/local/bin/orbit-f14e780` too); `orbit` ready 08:05:37, then `orbit-agent`. `vm-health.conf` (old copy
`.before-f14e780`): `PROD_TAG=f14e780`, `ROLLBACK_TAG=08067fd`, `ROLLBACK_BACKUP=01a1d3f0-85dc-73c7-ba6e-4f293864e460`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness (all 6 checks) and `/health/live` 200; revision
`f14e7807`; schema 20261113042745 (101 rows, unchanged); 0 error lines in the journal (only the known "allowed origin
without a port" WARN); sidecar `/health` ok and `src` equal to `main`. Webhook URL: with a temporary v7 session (David
Dusi) a temporary webhook in `#orbit-updates` was created by the API; its URL was
`https://orbit-hooks.easytomanage.com/api/v1/hooks/chat/<id>/<secret>`. Through the public hooks host: a wrong secret
of the same shape gave Orbit's 404 `webhook_not_found` (problem+json), the real URL with `{}` gave 422 (nothing posted;
a secret of another shape gets the gateway's own HTML 404). The webhook was deleted by the API (204) and its revoked row
removed; 0 new messages in `#orbit-updates`. `ORBIT_VERIFY_SPA=1 verify-production.sh` passed (signed out `/` →
`/login`, signed in `/` and `/tasks`, 0 console or page errors; session logged out, the row deleted after: 46 sessions,
as before). `vm-health` OK except oom (3 lines from 03:06 UTC, an earlier build), nginx (20 lines, upstream refused
08:05:18, the start window) and git-owner (root-owned files in `.git`). **Not** verified: a real outside tool posting a
message, and a button click's `response_url` on production.

Rollback to `08067fd` (no migration; the env file must go back too, the old binary refuses the new key):

```sh
sudo systemctl stop orbit-agent orbit
sudo cp -a /etc/orbit/orbit.env.before-hooks /etc/orbit/orbit.env
sudo install -m 755 /usr/local/bin/orbit-08067fd /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-f14e780 /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# (the same as /root/ops/public-hooks/rollback.sh)
# Data, only if needed (work after 08:05 UTC is lost), Orbit stopped: /usr/local/bin/orbit-08067fd backup verify +
#   backup restore 01a1d3f0-85dc-73c7-ba6e-4f293864e460 (as user orbit, with the old /etc/orbit/orbit.env)
```

After this rollback, set `PROD_TAG=08067fd`, `ROLLBACK_TAG=c5735ed`,
`ROLLBACK_BACKUP=01a1d39c-33ee-7608-a91e-264705f889f3` in `/root/ops/vm-health.conf` (copy
`/root/ops/vm-health.conf.before-f14e780`). Old webhook URLs on `orbit.easytomanage.com` keep working behind the login;
URLs on the hooks host also work after a rollback (same path), but new ones show the login origin again.

### November 13, 2026 upgrade: iPhone app server part, APNs push and Ask Donna for the phone (schema 20261113042745)

`main` from `138b8291` to `08067fd6` (local, not pushed): `iphone-app` fast-forwarded (`f3d87510` feature: APNs sender,
`POST /api/v1/push/devices` and `DELETE /api/v1/push/devices/{token}`, Ask Donna for the phone, `ORBIT_APNS_*` settings;
`53d02d3c` clippy; `b7433b01` icon; `d8d90afa` generated client), then `08067fd6` (what's new). What is new for people
(`whats-new.md`): the branch's `2026-11-14` entry became part of today's `2026-11-13` entry, still 4 lines (one iPhone
line on top: the app by TestFlight invitation, push, the Ask Donna bar; the 3 other lines of upstream port 1 shortened
into 3). `aboutOrbit()` has the iPhone app line under "Elsewhere", the Ask bar line for the iPhone and a "How do I…" line
for iPhone notifications. The iPhone app itself is **not** on TestFlight yet (waits for the App Store Connect record).

Migration `20261113042745_push_devices` (SQLite + Postgres): new table `push_devices` (token unique, session cascade,
2 indexes), additive. `latest_version()` 20261113042745: the old binary refuses the migrated database ("schema version
20261113042745 is newer than binary schema version 100").

Checks on merged `main` (the tree equals `iphone-app` plus `whats-new.md`; the branch passed push, push_devices_api,
push_api, chat_api, openapi_contract, app_smoke, platform database + backup on SQLite and Postgres): after
`bun install --frozen-lockfile` and `npm ci` in `apps/agent`: agent `tsc` and `npm test` (80 pass, 11 skipped, 0 fail);
mobile `tsc` and `bun test` (175 pass). No Rust test run again on `main` (same Rust tree as the branch).

Deploy id `20261113T055300Z-86905`. `orbit:08067fd` (image
`sha256:03607bceb24903bb2bbf3a66959937d3e2e4e504fd4fc6363bb5ef2958868bdf`, about 30 min, no OOM);
`deploy/smoke.sh` and `orbit config check` (production env, no APNs keys) passed. Migration dry run on a hot read-only
copy and again on a copy of the stopped database: 1 applied, schema 100 → 20261113042745 (101 rows), the 100 old
checksums unchanged, a second run applies 0, quick-check ok, 0 foreign-key rows, no pre-migration backup made, counts
unchanged (8 users, 110 tasks, 18 task comments, 94 notifications, 1104 chat messages, 46 sessions, 2 push
subscriptions, 6 projects). Rollback without the snapshot, tested on a copy: `DROP TABLE push_devices` and delete the
version row, then the old binary applies 0 and starts (quick-check ok, schema 100). Scripts: `/root/ops/iphone-app/`.

Switch: stopped both services at 06:33:11 UTC; snapshot `01a1d39c-33ee-7608-a91e-264705f889f3` with
`/usr/local/bin/orbit-c5735ed`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-08067fd.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-08067fd.tgz`; old sidecar code `/opt/orbit-agent.before-08067fd`; new
sidecar `src` (`package*.json` unchanged, no `npm ci`); binary installed (`/usr/local/bin/orbit-08067fd` too); `orbit`
ready 06:33:26, then `orbit-agent`. `vm-health.conf` (old copy `.before-08067fd`): `PROD_TAG=08067fd`,
`ROLLBACK_TAG=c5735ed`, `ROLLBACK_BACKUP=01a1d39c-33ee-7608-a91e-264705f889f3`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness (all 6 checks) and `/health/live` 200; revision
`08067fd6`; schema top 20261113042745 (101 rows), `push_devices` 0 rows; live quick-check ok, 0 foreign-key rows; without
a session `POST /api/v1/push/devices` (valid body) 401, `DELETE /api/v1/push/devices/{token}` 401, `POST
/api/v1/push/test` 401 (with a `{}` body the POST gives 400 first: the body is read before the session, as on the other
JSON routes); an unknown API route 404; sidecar `/health` ok and `src` equal to `08067fd6`; 0 error lines in the
journal (only the known "allowed origin without a port" WARN; no APNs line, as expected with no keys);
`ORBIT_VERIFY_SPA=1 verify-production.sh` passed (signed out `/` → `/login`, signed in `/` and `/tasks` as David Dusi
with a temporary v7 session, 0 console or page errors; session logged out, the row deleted after: 46 sessions, as
before). `vm-health` OK except oom (3 lines from 03:06 UTC, the earlier build) and nginx (15 lines, upstream refused
06:33:12–06:33:26, the start window). **Not** verified on production: a real iPhone registration and an APNs push (no
keys yet, no TestFlight build), Ask Donna from the phone.

Rollback to `c5735ed`. Way 1 keeps the work done after the switch (the only loss is the iPhones' device rows):

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit sqlite3 /var/lib/orbit/orbit.sqlite "DROP TABLE push_devices; DELETE FROM schema_migrations WHERE version = 20261113042745;"
sudo install -m 755 /usr/local/bin/orbit-c5735ed /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-08067fd /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# Way 2, only for a data problem (work after 06:33 UTC is lost), Orbit stopped: /usr/local/bin/orbit-08067fd backup verify +
#   backup restore 01a1d39c-33ee-7608-a91e-264705f889f3 (as user orbit, with /etc/orbit/orbit.env), then the binary and sidecar lines above
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-08067fd.tgz
# Plain copy of the database before the switch: /var/backups/orbit/pre-migration/orbit-before-08067fd.sqlite
```

After this rollback, set `PROD_TAG=c5735ed`, `ROLLBACK_TAG=3a468aa`,
`ROLLBACK_BACKUP=01a1d315-5386-73ad-a657-d92c30da0283` in `/root/ops/vm-health.conf` (copy
`/root/ops/vm-health.conf.before-08067fd`).

### November 13, 2026 upgrade: upstream port 1 (schema 100, no new migration)

`main` from `bdc34371` to `c5735ed8` (local, not pushed): `upstream-port1` (21 commits `20800092` … `d73f7dea` on
`fb131eb4`) rebased on a temporary branch, main fast-forwarded. Kept 19 (new shas `f64427dc` … `157a7c4b`): generated
migration catalog (`f64427dc`), push hosts (#100), spinner, root account cannot be deleted, OAuth consent (workspaces
without projects; one workspace skips the picker), idle sessions removed after 30 days, `#` task references + sectioned
⌘K + reference previews (#93), "Searching tasks…", mention ranking, searchable pickers (#96), bell popover (#110), MCP
comments / assignees / lookups / `create_project` and labels (#101), retention purge with token grants, api-token badges
on phones, retention tests, aboutOrbit + what's new. Dropped 2, main has them: `75ac9ccc` (`strictExecutionOrder`, main's
`72fc5049` with its comment) and `d73f7dea` (lesson; main's Rolldown lesson is fuller). Added 2: `25a40b77` (generated
client made again from the merged `openapi.json`: the rebase kept the branch's `sdk.gen.ts`, without main's label-rule,
chat Drive file and Drive preview calls; `openapi.json` from the binary equals the merged file) and `c5735ed8` (main's
`label_rules_go_with_their_label_and_project` test came back: the conflict resolution of `tests/database.rs` had dropped
it). What is new for people (`whats-new.md`, new `2026-11-13` entry, 4 lines): `#` task links and previews, ⌘K
sections; the bell popover; search in the project, assignee and repository pickers; MCP comments, assignees, labels,
projects, and sessions idle 30 days are signed out. `aboutOrbit()` keeps main's label-chip sentence and adds the
task-reference and picker lines.

Rebase conflicts: schema numbers in `backup.rs`, `app_smoke`, platform `database` and `backup` tests (branch side:
`MigrationRunner::latest_version()`); `migrate.rs` (branch side; main had only changed catalog entries); `build.rs`
`LAST_SEQUENTIAL_VERSION` 97 → 100 (also "after 0100" in `.ai/lessons.md`, "0001–0100" in `AGENTS.md`, and the commit
message); `workspaces.rs` retention keeps main's `chat_drive_files` delete and adds the session delete; generated
`index.ts` removed (the new generator does not write it, nothing imports it); `openapi_contract` 347 + 1 = 348
operations (paths 269); `InboxPage.tsx` takes the branch's `NotificationRow`, and `useNotificationRows.actor` gets main's
GitHub actor ("login (GitHub)"); `prompt.ts` as above.

Migrations: no new one. Build-time catalog checked against main's hand-written one: SQLite 1–100 (100), Postgres 46–100
(55), same order, same SHA-256 per file, same destructive flags (21 and 76; Postgres 76). The runner stores only
`version` + `checksum` (SHA-256 of the SQL) in `schema_migrations`, no file name; `0021_task_relations` and
`0076_api_token_scopes` (SQLite + Postgres) are renames to `.destructive.sql` with the same bytes (R100), so nothing
re-runs and no backup is forced. New migrations now need a UTC timestamp version (`just new-migration`): a 4-digit
`0101` (iphone-app `push_devices`) is refused by `build.rs` unless that branch sets `LAST_SEQUENTIAL_VERSION` to 101 or
takes a timestamp.

Retention: the job is a persisted daily schedule (runs at about 14:09 UTC; the last 8 runs passed with 0 purged), so the
first run with the new code is not at the switch. Its new statements, run on a copy as of 2026-11-13 14:10 UTC: sessions
49 → 48 (one session of David Dusi, idle since 2026-10-12, idle-expired 2026-11-11, no push subscription, no agent
session), 0 projects (1 trashed, not 30 days old), 0 token grants changed; push subscriptions, agent sessions and tasks
unchanged.

Checks (after `bun install --frozen-lockfile`, `npm ci` in `apps/agent`, fresh web `dist`): agent `tsc` and `npm test`
(80 pass, 11 skipped, 0 fail); web `tsc -b`; mobile `tsc`; web `bun test` of `app/shell`, `components/common`,
`features/inbox`, OAuth consent, chat composer, docs editor, task references, `useHoverPreview`,
`useMentionAutocomplete` (241 pass); `features/tasks` 392 pass, 2 fail (the known `TaskList` bulk toolbar tests). Rust
under `orbit-build`, `CARGO_PROFILE_*_DEBUG=0`, SQLite: platform `backup` 23, `database` 54 (53 before `c5735ed8`); server
`--lib` 276, `account_api` 14, `api_tokens_backends` 4, `app_smoke` 21, `mcp_api` 19, `mcp_drive_api` 3,
`notifications_api` 3, `oauth_api` 12, `openapi_contract` 19, `public_api` 30, `push_api` 6, `tasks_api` 100,
`workspaces_api` 30: all pass. Not run: Postgres, the rest of the suites.

Image: the first `orbit-deploy image` (of `25a40b77`) was killed after 2 h 15 min: rustc OOM in the BuildKit container,
which had `--memory=4g` (2.5 billion limit hits, it thrashed); the release compile of `orbit-server` now needs about
5 GiB. `docker update --memory=6g --memory-swap=6g orbit-buildkit` (inside `orbit-builds.slice`, 8G/10G; undo with
`4g`), `deploy/README.md` updated. Second build of `c5735ed8`: about 30 min, peak 5.0 GiB. Deploy id
`20261113T002000Z-4106014`. `orbit:c5735ed` (image
`sha256:1e483169009763c751e1789779b20c422930aea1749d1bab7f7c126729c6469f`); `deploy/smoke.sh` and `orbit config check`
passed. Before the switch (`.ai/lessons.md`, Rolldown): the release binary's embedded `dist` on a seeded DB in
`unshare -n`, signed in as `test@example.com`: `/`, `/tasks`, a task, `/docs`, `/drive`, `/chat`, `/activity`,
`/settings`, `/settings/members`, `/settings/api-tokens`, `/settings/agentic/routines`, `/admin` render, 0 page errors,
only the known `view-preferences/all` 404; bell popover, ⌘K "Tasks" section for `GEN-1`, the assignee picker's search
box and the `#` task list in chat all show. Migration dry run on a hot read-only copy and again on a copy of the stopped
database: 0 applied, the 100 stored checksums accepted, no pre-migration backup made, quick-check ok, 0 foreign-key
rows, counts unchanged (schema 100, 8 users, 110 tasks, 18 task comments, 94 notifications, 17 labels, 140 task labels,
1093 chat messages, 38 GitHub links, 49 sessions, 6 projects). Scripts: `/root/ops/upstream-port1/`.

Switch: stopped both services at 04:05:52 UTC; snapshot `01a1d315-5386-73ad-a657-d92c30da0283` with
`/usr/local/bin/orbit-3a468aa`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-c5735ed.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-c5735ed.tgz`; old sidecar code `/opt/orbit-agent.before-c5735ed`; new
sidecar `src` (`package*.json`, `tsconfig.json` unchanged, no `npm ci`); binary installed (`/usr/local/bin/orbit-c5735ed`
too); `orbit` ready 04:06:06, then `orbit-agent`. `vm-health.conf` (old copy `.before-c5735ed`): `PROD_TAG=c5735ed`,
`ROLLBACK_TAG=3a468aa`, `ROLLBACK_BACKUP=01a1d315-5386-73ad-a657-d92c30da0283`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness (all 6 checks) and `/health/live` 200; revision
`c5735ed8`; schema 100 (100 rows); live quick-check ok, 0 foreign-key rows; `POST /mcp` without a token 401; an unknown
API route 404; web `/`, `/tasks`, `/settings` 200; sidecar `/health` ok and `src` equal to `c5735ed8`; 0 error lines in
the journal; `ORBIT_VERIFY_SPA=1 verify-production.sh` passed (signed out `/`, signed in `/` and `/tasks`, session logged
out and revoked). `vm-health` OK except oom (3 lines: the build's rustc in the BuildKit container, not Orbit) and nginx
(1 line, upstream refused at 04:06:06, the start window); git-owner (774 tracked files and 4 in `.git` written by root
during the rebase) given back to `exedev`. Signed-in browser check (temporary v7 session as David Dusi, Slate
workspace, read only): the bell popover, ⌘K with `SLATE-1` in the "Tasks" section, the assignee picker of SLATE-1 with
its search box, and the `#` task list in his DM with Donna (typed, not sent); 0 console or page errors. Nothing written
after the switch (0 new sessions, chat messages, task changes or comments); the test session row was deleted after the
logout (49 sessions, as before). Screenshots in `/root/.local/share/com.jean.desktop/pasted-images/upstream-port1/`
(`live-01-bell-popover`, `live-02-command-palette-task-id`, `live-03-assignee-picker`, `live-04-chat-hash-task-picker`,
and `preswitch-01` … `preswitch-04` of the seeded check). **Not** verified on production: the MCP tools with a real
token, the OAuth consent page, the retention run (first one about 14:09 UTC today: expect 1 session deleted).

Rollback (no migration, so the old binary starts on this database: checked on a copy, 0 applied, 100 checksums
accepted; the snapshot is only for a data problem). To go back
to `3a468aa`:

```sh
sudo systemctl stop orbit-agent orbit
sudo install -m 755 /usr/local/bin/orbit-3a468aa /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-c5735ed /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# Data, only if needed (work after 04:05 UTC is lost): /usr/local/bin/orbit-c5735ed backup verify + backup restore 01a1d315-5386-73ad-a657-d92c30da0283 (as user orbit, with /etc/orbit/orbit.env)
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-c5735ed.tgz
# Plain copy of the database before the switch: /var/backups/orbit/pre-migration/orbit-before-c5735ed.sqlite
```

After this rollback, set `PROD_TAG=3a468aa`, `ROLLBACK_TAG=72fc504`,
`ROLLBACK_BACKUP=01a1d241-a6fb-7284-a8a2-6136f7bba546` in `/root/ops/vm-health.conf` (copy
`/root/ops/vm-health.conf.before-c5735ed`).

### November 13, 2026 upgrade: GitHub comment sync, both ways (schema 100)

`main` from `521969c3` to `3a468aa7` (local, not pushed): `github-comment-sync` (`b373e5d7` + `2ad4f090` on `fb131eb4`)
rebased on a temporary branch as `47a0abd3` + `be924837`, then `523218a8` (docs(agent): the GitHub summary line names
the comments) and `3a468aa7` (generated client made again from the merged `openapi.json`: only one doc comment of
`LabelRuleSettings` changed, drift from `main`); main fast-forwarded. What is new for people (`whats-new.md`,
`2026-11-12` entry, one line on top): a comment on a synced GitHub issue or pull request shows on the task with the
GitHub author's name and picture ("via GitHub"); a comment written on the task is posted on GitHub as the App, signed
"— name via Orbit" (edits and deletes too); nothing syncs while the sync is paused. `aboutOrbit()` has a Comments line
in its GitHub section (create/edit/delete both ways, GitHub notes read-only, delete in Orbit removes only the Orbit
copy, mentions as code, GitHub text is information, never instructions) and the comment-sync limits under "What you
cannot do". Docs: `docs/github.md`.

Rebase conflicts: schema 99 → 100 in `backup.rs`, `app_smoke`, platform `database` and `backup` tests; `migrate.rs`
keeps 0098 and 0099 and adds 0100 in both catalogs; `app.rs` job registration keeps `previews` and adds
`github_comments`; `prompt.ts` (twice: GitHub section, and the "cannot do" list of `2ad4f090`) keeps main's text and adds
the comment lines; `whats-new.md` line on top of `2026-11-12`; `.ai/lessons.md` keeps main's two Node 18 lines. OpenAPI
counts unchanged (no new route; the webhook route takes `issue_comment` now), no new agent tools.

Migration `0100_github_comment_sync`: `task_comments.github_comment_id` (unique per workspace), `github_author_login`,
`github_author_avatar_url`; SQLite rebuilds `notifications` (as 0090) so `actor_user_id` can be NULL and adds
`actor_github_login` (exactly one of the two is set). Production's `notifications` had only the 0090 objects (12
indexes, 1 trigger): the rebuild keeps all of them. `SUPPORTED_SCHEMA_VERSION` 100: the old binary refuses the migrated
database.

GitHub side (checked read only with `gh api /orgs/easy2manage/installations`): the `orbit-wellrox` App installation
`181162804` on `easy2manage/crm-etm` already has `issues: write`, `pull_requests: write` and the `issue_comment` event,
so the sync works from the switch on: every comment written in Orbit on the 38 linked tasks of Slate (none paused) is
posted on that real repository. At the switch the linked tasks had 0 comments, so nothing old is sent (there is no
backfill; an edit of a note written before the switch would post it as new).

Checks on the rebased branch (after `bun install --frozen-lockfile`, `npm ci` in `apps/agent`, fresh web `dist`): agent
`tsc` and `npm test` (80 pass, 11 skipped, 0 fail); web `tsc -b`; web `bun test` `tasks/api/models.test.ts` and
`features/inbox` (19 pass); mobile `tsc`. Rust under `orbit-build`, `CARGO_PROFILE_*_DEBUG=0`, SQLite: platform
`database` 54 + `backup` 23; server `github_comments` 5, `integrations_api` 13, `app_smoke` 21, `openapi_contract` 19,
`tasks_api` 99: all pass. Not run: Postgres, the rest of the suites. `target` removed.

Deploy id `20261112T231336Z-4056897`. `orbit:3a468aa` (image
`sha256:9c394b3ad79b76a6964f699b51ccbc8179f814fcfbdcd31a2cbcff7209ce0fe5`; the first `orbit-deploy image` failed at the
export after the 51 min compile, "no active session … context deadline exceeded"; the second run used the cache and
passed); `deploy/smoke.sh` and `orbit config check` passed. Before the switch (`.ai/lessons.md`, Rolldown): the release
binary's embedded `dist` on a seeded DB in `unshare -n`, signed in as `test@example.com`, with a GitHub note and an Orbit
note on the seed task: `/`, `/tasks`, the task, `/docs`, `/drive`, `/chat`, `/activity`, `/settings`,
`/settings/members`, `/settings/agentic/routines`, `/admin` render, 0 page errors, only the known `view-preferences/all`
404; the task shows "octocat via GitHub". Migration dry run of 0100 on a hot read-only copy and again on a copy of the
stopped database: 1 applied, schema 99 → 100, quick-check ok, 0 foreign-key rows, the 94 notification rows identical,
counts unchanged (8 users, 110 tasks, 18 task comments, 94 notifications, 17 labels, 140 task labels, 1092 chat messages,
38 GitHub links); copies deleted. Scripts: `/root/ops/comment-sync/`.

Switch: stopped both services at 00:14:40 UTC; snapshot `01a1d241-a6fb-7284-a8a2-6136f7bba546` with
`/usr/local/bin/orbit-72fc504`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-3a468aa.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-3a468aa.tgz`; old sidecar code `/opt/orbit-agent.before-3a468aa`; new
sidecar `src` (`package*.json`, `tsconfig.json` unchanged, no `npm ci`); binary installed (`/usr/local/bin/orbit-3a468aa`
too); `orbit` ready 00:14:54, then `orbit-agent`. `vm-health.conf` (old copy `.before-3a468aa`): `PROD_TAG=3a468aa`,
`ROLLBACK_TAG=72fc504`, `ROLLBACK_BACKUP=01a1d241-a6fb-7284-a8a2-6136f7bba546`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness and `/health/live` 200; revision `3a468aa7`;
schema 100; live quick-check ok, 0 foreign-key rows; an unsigned `POST .../integrations/github/webhook` with
`X-GitHub-Event: issue_comment` 401 (`invalid_github_signature`); an unknown API route 404; web `/`, `/tasks`,
`/settings` 200; sidecar `/health` ok and `src` equal to `3a468aa7`; 0 error lines in the journal; `ORBIT_VERIFY_SPA=1
verify-production.sh` passed (signed out `/`, signed in `/` and `/tasks`, session logged out and revoked). `vm-health` OK
except nginx (2 lines, upstream refused at 00:14:53, the start window), docker (`cmp-test-s3`, not Orbit) and git-owner
(73 files in `.git` written by root during the rebase; given back to `exedev`). Signed-in browser check (temporary v7
session as David Dusi): one temporary note row put in by SQL on SLATE-1 (`github_author_login` `octocat`, fake GitHub
comment id 1; such notes are never sent to GitHub, and an SQL insert makes no notification or job); SLATE-1 shows it as
"octocat via GitHub"; Settings → GitHub shows `orbit-wellrox` and `easy2manage/crm-etm`; console only the known
`view-preferences/all` 404. The row was deleted after (0 notes with a GitHub id, 0 notifications with a GitHub actor, 0
`github.comment` jobs). Nothing was posted on GitHub. Screenshots in
`/root/.local/share/com.jean.desktop/pasted-images/github-comment-sync/` (`live-01-task-comment-from-github`,
`live-02-github-settings`, and `preswitch-01-seeded-task-github-note`). **Not** verified on production: a real signed
`issue_comment` event and a real post to GitHub (the first comment someone writes on a linked task is the first real
test: check the journal and `jobs` for `github.comment`).

Rollback (the old binary refuses schema 100: restore the snapshot together with the binary; work after 00:14 UTC is
lost, and comments already posted on GitHub stay there). To go back to `72fc504`:

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-3a468aa backup verify 01a1d241-a6fb-7284-a8a2-6136f7bba546 && /usr/local/bin/orbit-3a468aa backup restore 01a1d241-a6fb-7284-a8a2-6136f7bba546'
sudo install -m 755 /usr/local/bin/orbit-72fc504 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-3a468aa /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-3a468aa.tgz
# Plain copy of the database before the migration: /var/backups/orbit/pre-migration/orbit-before-3a468aa.sqlite
```

After this rollback, set `PROD_TAG=72fc504`, `ROLLBACK_TAG=9b163ce`,
`ROLLBACK_BACKUP=01a1d1e6-f76d-72bf-917e-a73a36d51c61` in `/root/ops/vm-health.conf` (copy
`/root/ops/vm-health.conf.before-3a468aa`).

### November 12, 2026 upgrade: label overflow fix and label rules, second try (schema 99)

`main` `72fc5049` (what the rolled-back `ceb396f1` had, plus `72fc5049`: `output.strictExecutionOrder` in
`apps/web/vite.config.ts` against the Rolldown chunk cycle) and `bcbb8add` (deploy tooling only, not in the binary).
What is new for people (`whats-new.md`, `2026-11-12`): a task row shows 2 labels and a board card 3, a "+N" chip shows
the rest; Project settings → Rules (if a task has label X, assign person Y; owner:<name> switch; owner name map); Donna
can list and add rules. Migration `0099_label_rules`. The code checks of the rolled-back entry below still hold (only the
web build config changed after them); the image build ran `tsc -b && vite build` again.

New check, `bcbb8add`: `ORBIT_VERIFY_SPA=1 deploy/verify-production.sh` runs `deploy/verify-spa.ts` (Playwright from
the repo's `node_modules`): loads `/` signed out and, with `ORBIT_VERIFY_SESSION_TOKEN_FILE`, `/` and `/tasks` signed in;
fails on a page error, a console error (the signed-out 401 of `/auth/me` is allowed) or a blank page; logs the session
out at the end. Passed on production `9b163ce` (signed out and in); failed on the `ceb396f` binary (`i is not a
function`, blank page; local seeded server in `unshare -n`). Known gap: a workspace with no saved task view gets a 404
from `view-preferences/all` on `/tasks`, a console error (seen on seeded DBs and in the Slate workspace, also on
`9b163ce`), so the signed-in check fails for such a person's default workspace. How it was run here (as root):

```sh
# TLS hop: socat openssl-listen:8443,bind=127.0.0.1,reuseaddr,fork,cert=<self-signed pem>,verify=0 tcp:127.0.0.1:8000
# CURL_HOME/.curlrc: connect-to = "orbit.easytomanage.com:443:127.0.0.1:8443", insecure, header = "X-Forwarded-Proto: https"
# token: a temporary v7 sessions row (see the memory note on signed-in checks), token in a mode-600 file
PLAYWRIGHT_CHROMIUM_EXECUTABLE=/root/.cache/ms-playwright/chromium_headless_shell-1243/chrome-headless-shell-linux64/chrome-headless-shell \
CURL_HOME=<dir> ORBIT_VERIFY_SPA=1 ORBIT_VERIFY_SESSION_TOKEN_FILE=<token file> \
ORBIT_VERIFY_ORIGIN=https://orbit.easytomanage.com ORBIT_VERIFY_CONNECT=127.0.0.1:8443 \
ORBIT_VERIFY_HEADER='X-Forwarded-Proto: https' bash deploy/verify-production.sh
```

Deploy id `20261112T213252Z-3980354`. `orbit:72fc504` (image
`sha256:17dbdbf22e865b70ce3f062b0b36a44b9252fcc55c9c77c7393c0a7474f819d4`); `deploy/smoke.sh` and `orbit config check`
passed. Before the switch (`.ai/lessons.md`, Rolldown): the release binary's embedded `dist` on a seeded DB in
`unshare -n`, signed in as the seed user: `/`, `/tasks`, `/docs`, `/drive`, `/chat`, `/activity`, `/settings`,
`/settings/members`, `/settings/agentic/routines`, `/admin` and project settings with the Rules card render, 0 page
errors; only the `view-preferences/all` 404 above, the same as `9b163ce`. Migration dry run of 0099 on a hot read-only
copy and again on a copy of the stopped database: 1 applied, schema 98 → 99, quick-check ok, 0 foreign-key rows, counts
unchanged (8 users, 110 tasks, 17 labels, 140 task labels, 76 assignees, 1091 chat messages); copies deleted.

Switch: stopped both services at 22:35:37 UTC; snapshot `01a1d1e6-f76d-72bf-917e-a73a36d51c61` with
`/usr/local/bin/orbit-9b163ce`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-72fc504.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-72fc504.tgz`; old sidecar code `/opt/orbit-agent.before-72fc504`; new
sidecar `src` (`package*.json`, `tsconfig.json` unchanged, no `npm ci`); binary installed (`/usr/local/bin/orbit-72fc504`
too); `orbit` started 22:35:56, ready 22:35:57, then `orbit-agent`. `vm-health.conf` (old copy `.before-72fc504`):
`PROD_TAG=72fc504`, `ROLLBACK_TAG=9b163ce`, `ROLLBACK_BACKUP=01a1d1e6-f76d-72bf-917e-a73a36d51c61`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness and `/health/live` 200; revision `72fc5049`;
schema 99; live quick-check ok, 0 foreign-key rows; without a session `GET .../label-rules` 401, `POST` 400, `GET
.../label-rule-settings` 405 (no GET route); with the test session `GET .../label-rules` of Slate 200 (no rules, owner
labels off); an unknown API route 404; web `/`, `/tasks`, `/settings` 200; sidecar `/health` ok and `src` equal to
`72fc5049`; 0 error lines in the journal (the known "allowed origin without a port" WARN); `verify-production.sh` with
`ORBIT_VERIFY_SPA=1` passed (signed out `/`, signed in `/` and `/tasks`, session logged out). `vm-health` OK except nginx
(2 lines, upstream refused at 22:35:56, the start window) and docker (`cmp-test-s3`, not Orbit). Signed-in browser
check (temporary v7 session as David Dusi, read only, nothing written; revoked after, 0 open test sessions): Slate →
project settings shows the Rules card; the task list shows 2 labels and "+N" per row, hover shows the rest.
Screenshots in `/root/.local/share/com.jean.desktop/pasted-images/label-fixes/` (`live-01-label-rules-settings`,
`live-02-task-row-many-labels`, `live-03-label-overflow-popover`).

Cleanup: `/usr/local/bin/orbit-ceb396f` and `orbit:ceb396f` removed by hand (the broken build; `orbit-deploy cleanup`
kept it as a newest copy). `orbit-deploy cleanup --apply` not run (it would remove 6 older binaries and images).

Rollback (the old binary refuses schema 99: restore the snapshot together with the binary; work after 22:35 UTC is
lost). To go back to `9b163ce`:

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-72fc504 backup verify 01a1d1e6-f76d-72bf-917e-a73a36d51c61 && /usr/local/bin/orbit-72fc504 backup restore 01a1d1e6-f76d-72bf-917e-a73a36d51c61'
sudo install -m 755 /usr/local/bin/orbit-9b163ce /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-72fc504 /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'Host: orbit.easytomanage.com' -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-72fc504.tgz
# Plain copy of the database before the migration: /var/backups/orbit/pre-migration/orbit-before-72fc504.sqlite
```

After this rollback, set `PROD_TAG=9b163ce`, `ROLLBACK_TAG=eb0baae`,
`ROLLBACK_BACKUP=01a1d065-150e-7023-821e-2f907b9762bd` in `/root/ops/vm-health.conf`.

### November 12, 2026 attempt: label overflow fix and label rules (schema 99) — ROLLED BACK

Production stays on `9b163ce` (schema 98). The new web bundle did not load: every page was blank with
`TypeError: i is not a function` at module load (two `shared~index~…` chunks import each other, the Rolldown cycle
of `.ai/lessons.md`, "Tooling"). Health, smoke test, `verify-production.sh` and `vm-health` all passed: none of them
runs the SPA's JavaScript. Only the signed-in browser check found it.

`main` from `aae50d2e` to `ceb396f1` (local, not pushed): `label-overflow` (was `9b1caada` on `fb131eb4`) rebased as
`14b904e5`; `label-rules` (was `901b6615` + `e3b9dca7`, also on `fb131eb4`, not on `aae50d2e`) rebased on top as
`db764124` + `a0baffc4`; main fast-forwarded; `ceb396f1` adds the what's-new line of the overflow fix. Rebase conflicts:
schema 98 → 99 in `backup.rs` and the tests (`app_smoke`, platform `database`, `backup`); `migrate.rs` keeps 0098 and
adds 0099 in both catalogs; OpenAPI counts 269 paths / 347 operations (259 → 262 / 334 → 339 of the branch on top of
main's 266 / 342); generated client made again from the merged `openapi.json`; `prompt.ts` keeps main's GitHub section
and adds the label rules line; `whats-new.md` lines on top of `2026-11-12`. Semantic conflict found by the build: main
and the branch each added 2 agent tools, so `TOOL_CATALOG` is `[…; 55]` and its unit test counts 55 (folded into
`db764124`).

Checks on the merged `main`: agent `tsc` and `npm test` (80 pass, 11 skipped, 0 fail); mobile `tsc`, `activity.test.ts`
(6 pass); web `tsc -b`; web `bun test src/features/tasks` 387 pass, 2 fail (`TaskList.test.tsx` bulk toolbar tests:
they fail the same way on `aae50d2e`); e2e `label-overflow.spec.ts` 5 pass (Vite dev server, so it did not see the
bundle bug). Rust under `orbit-build`, `CARGO_PROFILE_*_DEBUG=0`, SQLite: platform `database` + `backup` 76 pass, 1 flaky
(`page_search_migration_backfills_and_tracks_pages`, passes 3/3 alone); server `--lib` (276), `label_rules_api` 8,
`openapi_contract` 19, `guest_access` 1, `app_smoke` 21, `agent_profiles` 9, `integrations_api` 13, `tasks_api` 99: all
pass. `target` removed.

Deploy id `20261112T181905Z-3816669`. `orbit:ceb396f` (image
`sha256:fd8ad716b7bb17096d75cfe19523666afabe2eb2f1fd2f2bd8684135a48bf8b2`); `deploy/smoke.sh` and `orbit config check`
passed. Migration dry run of 0099 on a hot read-only copy and again on a copy of the stopped database: 1 applied,
schema 98 → 99, quick-check ok, 0 foreign-key rows, counts unchanged (8 users, 110 tasks, 17 labels, 140 task labels,
76 assignees, 1030 chat messages); copies deleted.

Switch: stopped both services at 20:23:19 UTC; snapshot `01a1d16d-d932-74d2-a660-0f966c0ab9d1` with
`/usr/local/bin/orbit-9b163ce`, verified, `backup-rehearsal.sh` passed; plain copy
`/var/backups/orbit/pre-migration/orbit-before-ceb396f.sqlite`; sidecar data
`/var/backups/orbit-agent/orbit-agent-before-ceb396f.tgz`; installed `orbit-ceb396f`, ready 20:23:39. Services
active, readiness 200, revision `ceb396f1`, schema 99, label-rules routes 401/400 without a session. Then the
signed-in check (temporary v7 session as David Dusi) showed the blank pages.

Rollback (20:26:26 → ready 20:26:33): `orbit-ceb396f backup verify` + `restore` of `01a1d16d…` (the restore kept the
schema-99 data as snapshot `01a1d170-b3da-7405-b30c-1163969deb3d`), the `9b163ce` binary installed again, sidecar code
`/opt/orbit-agent.before-ceb396f` moved back. Nothing but the test session had been written between the switch and the
rollback. Verified: services active, readiness 200, revision `9b163ced`, schema 98, quick-check ok, 0 foreign-key rows,
counts as before; the signed-in probe renders `/` and `/tasks` with no page error. Test session logged out (204, revoked);
socat stopped. `vm-health.conf` back to its `.before-ceb396f` copy (`PROD_TAG=9b163ce`, `ROLLBACK_TAG=eb0baae`).
`/usr/local/bin/orbit-ceb396f` and the image stay until cleanup.

Cause, local: the built `apps/web/dist` of `main` fails at load the same way (static server, no API); a build of
`label-overflow` alone (`14b904e5`) loads fine, so the cycle comes with the `label-rules` web changes
(`ProjectRulesCard` and its imports into the lazy task pages). Before the next try: fix the chunk cycle, then load the
built app in a browser (not only Vite dev).

### November 12, 2026 upgrade: routine script fixes, chat media and Office previews (schema 98)

`main` from `fb131eb4` to `9b163ced`: two `--no-ff` merges, `routine-script-fixes` (`1c17ec63`; merge `3780f712`) then
`chat-media-previews` (`55bc80ee`; merge `0bd97ceb`), both on `09781634`, and `9b163ced` (docs(agent): the exact GitHub
status mapping in `aboutOrbit()`). What is new for people:
- Routines: Donna attaches a saved script to an existing routine after a yes (or Settings → Agentic → Routines → Edit →
  Script); a failed script in a scheduled run is fixed and run again by Donna (at most 2 times, the fix makes no writes
  until approved); a run with no result after an hour counts as failed; the dialogs fit the screen.
- Chat: videos (also iPhone .mov) play with a poster (the server makes an MP4 copy); click a video, PDF or Office file to
  open it large; files over the chat limit (25 MiB here) go to Drive → Uploads → the conversation's folder (presigned
  multipart upload to the bucket); previews of xlsx, xls, csv, docx, doc, pptx, ppt, odt, ods (LibreOffice → PDF) in
  chat and Drive. Docs: `docs/file-previews.md`.

Merge conflicts: `apps/agent/src/prompt.ts` (the branch's own GitHub section, with the label sync of `eb0baae` added);
`whats-new.md` (both branches dated `2026-11-13`: their 6 lines go on top of the `2026-11-12` entry, now 14 lines, one
entry per date); `0098_chat_media_previews` after `0097_github_task_labels` in both catalogs of `migrate.rs`;
`SUPPORTED_SCHEMA_VERSION` and the test versions 98. `aboutOrbit()` names both features (routines: lines on attach,
self-repair, the one-hour rule; chat: media, big files in Drive, Office previews) and its GitHub section now says:
closed issue or merged pull request → Done, pull request closed without a merge → Cancelled, reopen → unstarted, and the
labels (one way).

Migration: `0098_chat_media_previews` (SQLite and Postgres): `drive_nodes.chat_conversation_id` and `chat_root`, new
tables `chat_drive_files`, `file_previews`, `file_preview_outputs`. `SUPPORTED_SCHEMA_VERSION` 98: the old binary
refuses the migrated database.

Host change: `apt-get install --no-install-recommends libreoffice-calc libreoffice-writer libreoffice-impress
libreoffice-core fonts-dejavu-core fonts-liberation2` (24.2.7); a csv → PDF conversion as `orbit` in
`unshare --user --map-root-user --net` works. Rollback of it: `sudo apt-get purge -y 'libreoffice*' && sudo apt-get
autoremove -y` (previews then say "Preview not available"). **Not checked:** the R2 CORS rule of `wellrox-files` (the Drive bucket). A `curl` preflight
answers 403 from any origin, but David's browser uploads to Drive (presigned PUT: a 137 MB mp4 on November 6, an xlsx
on November 12 at 13:09) worked, so a rule exists. Whether it has the `ExposeHeaders` of `docs/file-previews.md` (ETag,
Content-Range, Accept-Ranges, Content-Length; for pdf.js ranges and players on Drive objects) is not known: check it in
the Cloudflare dashboard. A chat file over 25 MiB was not tried.

Checks on the merged `main` (after `bun install --frozen-lockfile`, `npm ci` in `apps/agent`, fresh web `dist`):
agent `tsc` and `node --test` (76 pass, 11 skipped, 0 fail); web `bun test` of `features/chat`, `features/drive`,
`features/agent`, `lib`, `components/common` and `driveFileBlock` (313 pass, 0 fail); mobile `tsc` and
`src/data/chat.test.ts` (23 pass); web build (`tsc -b && vite build`, also in the image). Rust, under `orbit-build`
with `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0`, SQLite: `cargo test -p orbit-platform`
(all pass, 0 fail); `cargo test -p orbit-server --lib` and `routine_scripts`, `scheduled_api`, `agent_profiles`,
`agent_api`, `chat_media`, `chat_api`, `attachments_api`, `drive_api`, `drive_shares_api`, `page_drive_files_api`,
`agent_drive`, `mcp_drive_api`, `guest_access`, `openapi_contract`, `app_smoke`, `integrations_api` (467 pass, 0 fail,
1 ignored). Not run: Postgres, the rest of the server suite. `target` removed afterwards.

Migration dry run with the release binary on a hot read-only copy of production (14:50 UTC) and again on a copy of the
stopped database at the switch: 1 applied, schema 97 → 98, quick-check ok, 0 foreign-key rows, counts unchanged (8
users, 109 tasks, 915 chat messages, 24 chat files, 53 attachment blobs, 539 Drive nodes, 1 routine, 1 routine run);
copies deleted.

`orbit-deploy image` (deploy id `20261112T141500Z-3663194`) built `orbit:9b163ce` (image
`sha256:9d89ecfe6a19c758c4c8ba3bca953ab9d974cd7f7a739c1731b8e3d2e5057931`, 14:50 UTC); `deploy/smoke.sh` passed;
`orbit config check` passed with the extracted binary.

At the switch: stopped `orbit-agent` and `orbit` at 15:34:07 UTC, created snapshot
`01a1d065-150e-7023-821e-2f907b9762bd` with the old CLI (`/usr/local/bin/orbit-eb0baae`) and verified it,
`deploy/backup-rehearsal.sh` with the old binary passed (verify), kept a plain copy of the stopped database as
`/var/backups/orbit/pre-migration/orbit-before-9b163ce.sqlite` (mode 600), saved the sidecar data as
`/var/backups/orbit-agent/orbit-agent-before-9b163ce.tgz`, kept the old sidecar code as `/opt/orbit-agent.before-9b163ce`,
installed the binary (`/usr/local/bin/orbit-9b163ce` too) and the new sidecar `src` (`package*.json` and
`tsconfig.json` unchanged, so no `npm ci`), and started `orbit` (15:34:47, ready 15:34:48) then `orbit-agent`.
`vm-health` (`/root/ops/vm-health.conf`, old copy `.before-9b163ce`): `PROD_TAG=9b163ce`, `ROLLBACK_TAG=eb0baae`,
`ROLLBACK_BACKUP=01a1d065-150e-7023-821e-2f907b9762bd`.

Verified: `orbit`, `orbit-agent`, nginx active, 0 restarts; readiness (all checks) and `/health/live` 200;
`/orbit-build.json` revision `9b163ced`; schema 98; live quick-check ok, 0 foreign-key rows; the backfill logged "chat
file types fixed and video previews queued" (6 fixed, 3 queued) and the 3 video previews are `ready`; web `/`, `/chat`,
`/drive`, `/settings/agentic/routines` 200; without a session `GET`/`POST .../chat/files/{id}/preview`, `GET
.../preview/{role}`, `POST .../drive/nodes/{id}/preview`, `GET .../drive/nodes/{id}/preview/{role}`, `GET
.../chat/upload-limit` 401; an unknown API route 404; `deploy/verify-production.sh` passes against
`http://127.0.0.1:8000` (headers via a `CURL_HOME` `.curlrc`); sidecar `/health` `{"ok":true,"mock":false}` and
`/opt/orbit-agent/src` equal to `apps/agent/src` of `9b163ced`; 0 error lines in the journal (the one WARN line, "allowed
origin without a port", was there before). `vm-health` OK except nginx (24 lines, all in the stop window 15:34) and
docker (`cmp-test-s3`, not Orbit). `.git/config` and `.git/packed-refs` had been rewritten by root at 15:26 (not by this
deploy); given back to `exedev`.

Signed-in smoke test on production (15:38 UTC, as David Dusi: a temporary v7 session row, socat TLS hop, Playwright;
logged out after, 204): (a) a test script row and a new paused test routine; Edit → Script → the script → Save attached
it to that routine ("its writes are approved"); (b) one message in David's own DM (no other member) with a PNG, a 3 s
H.264 MP4 and an xlsx: the image shows, the video shows its poster and plays, the xlsx opens in the viewer as a PDF
page made by LibreOffice (`file_previews` video and document `ready`). Afterwards the message, the routine and the script
were deleted through the API (0 rows left); the two unreferenced blobs and their previews go with the attachment
reconcile. Console: only the known `/sw.js` 400, and once, with the routine Edit dialog open, a CSP "Applying inline
style" refusal (not compared with `eb0baae`; nothing visible broke). Screenshots in
`/root/.local/share/com.jean.desktop/pasted-images/deploy-routines-media/` (`01-routine-edit-attach-script`,
`02-routine-with-script`, `03-chat-image-video-xlsx`, `04-office-preview-xlsx`, `05-video-playing-inline`).

Rollback (the old binary refuses schema 98: restore the snapshot together with the binary; work after 15:34 UTC is
lost). To go back to `eb0baae`:

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-9b163ce backup verify 01a1d065-150e-7023-821e-2f907b9762bd && /usr/local/bin/orbit-9b163ce backup restore 01a1d065-150e-7023-821e-2f907b9762bd'
sudo install -m 755 /usr/local/bin/orbit-eb0baae /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-9b163ce /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-9b163ce.tgz
# Plain copy of the database before the migration: /var/backups/orbit/pre-migration/orbit-before-9b163ce.sqlite
```

After this rollback, set `PROD_TAG=eb0baae`, `ROLLBACK_TAG=5def772`,
`ROLLBACK_BACKUP=01a1cf69-99c2-76de-a139-15a66dcf7d5b` in `/root/ops/vm-health.conf`.

### November 12, 2026 upgrade: GitHub label sync (schema 97)

`main` from `09781634` to `eb0baaeb`: one `--no-ff` merge of `github-label-sync` (`134b9b03` feat(github), `0ac8a8f2`
docs(lessons)). What is new for people: a task synced from a GitHub issue or pull request gets its GitHub labels (all
except the project's sync label); a missing project label is made with the GitHub color; a label removed on GitHub goes
from the task; labels added in Orbit stay; nothing goes back to GitHub. Docs: `docs/github.md`. What's new: one line on
top of the `2026-11-12` entry (now 8 lines); `aboutOrbit()` in `apps/agent/src/prompt.ts` names the label sync.

Migration: `0097_github_task_labels` (SQLite and Postgres) only adds the table `github_task_labels`.
`SUPPORTED_SCHEMA_VERSION` 97: the old binary refuses the migrated database.

Checks on the merged `main` (after `bun install --frozen-lockfile` and a fresh web `dist`; the web build needs
`/root/.local/node/bin` on `PATH`, Node 18 fails in rolldown): under `orbit-build` with `CARGO_PROFILE_*_DEBUG=0`,
`cargo test -p orbit-server --test integrations_api` 13 pass on SQLite and 13 pass on Postgres (`orbit-dev-pg`).

Migration dry run on a hot read-only copy of production (10:51 UTC) and again on a copy of the stopped database at the
switch: 1 applied, schema 96 → 97, quick-check ok, 0 foreign-key rows, counts unchanged (7 users, 74 tasks, 1 label, 2
task labels, 889 chat messages, 66 agent memories); copies deleted.

`orbit-deploy image` (deploy id `20261112T100528Z-3485043`) built `orbit:eb0baae` (image
`sha256:6671fe3099a0168b5f96ec0866b69f5c02fecd2d95151fc8f1174f530d1b992c`, 10:39 UTC); `deploy/smoke.sh` passed;
`orbit config check` passed with the extracted binary.

At the switch: stopped `orbit-agent` and `orbit` at 10:59:26 UTC, created snapshot
`01a1cf69-99c2-76de-a139-15a66dcf7d5b` with the old CLI (`/usr/local/bin/orbit-5def772`) and verified it,
`deploy/backup-rehearsal.sh` with the old binary passed (verify), kept a plain copy of the stopped database as
`/var/backups/orbit/pre-migration/orbit-before-eb0baae.sqlite` (mode 600), saved the sidecar data as
`/var/backups/orbit-agent/orbit-agent-before-eb0baae.tgz`, kept the old sidecar code as `/opt/orbit-agent.before-eb0baae`,
installed the binary (`/usr/local/bin/orbit-eb0baae` too) and the new sidecar `src` (only `prompt.ts` and
`whats-new.md` changed; `package*.json` and `tsconfig.json` unchanged, so no `npm ci`), and started `orbit` (11:00:21)
then `orbit-agent` (11:00:26). `main` is **not** pushed. `vm-health` (`/root/ops/vm-health.conf`, old copy
`.before-eb0baae`): `PROD_TAG=eb0baae`, `ROLLBACK_TAG=5def772`, `ROLLBACK_BACKUP=01a1cf69-99c2-76de-a139-15a66dcf7d5b`.

Verified: `orbit`, `orbit-agent` active, 0 restarts; readiness (all checks) and `/health/live` 200;
`/orbit-build.json` revision `eb0baaeb`; schema 97; live quick-check ok, 0 foreign-key rows, `github_task_labels` empty;
web `/`, `/settings/agentic/agents` and an SPA route 200; an unsigned `POST /api/v1/integrations/github/webhook` 401
(`invalid_github_signature`); an unknown API route 404; `deploy/verify-production.sh` passes against
`http://127.0.0.1:8000` (headers via a `CURL_HOME` `.curlrc`); sidecar `/health` `{"ok":true,"mock":false}` and
`/opt/orbit-agent/src` equal to `apps/agent/src` of `eb0baaeb`; no warning lines in the journal. `vm-health` OK except
nginx (23 "connection refused" lines, all in the stop window 10:59:26-11:00:21; 0 5xx). **Not** verified on
production: a real signed GitHub event with labels, a signed-in check of the GitHub card. Open: the webhook's problem
body names `instance` `/api/public/v1/integrations/discord/events` (`INSTANCE` in `integration_routes.rs`, since
`c60a45f6`; cosmetic, not from this deploy).

Rollback (the old binary refuses schema 97: restore the snapshot together with the binary; work after 10:59 UTC is
lost). To go back to `5def772`:

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-eb0baae backup verify 01a1cf69-99c2-76de-a139-15a66dcf7d5b && /usr/local/bin/orbit-eb0baae backup restore 01a1cf69-99c2-76de-a139-15a66dcf7d5b'
sudo install -m 755 /usr/local/bin/orbit-5def772 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-eb0baae /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-eb0baae.tgz
# Plain copy of the database before the migration: /var/backups/orbit/pre-migration/orbit-before-eb0baae.sqlite
```

After this rollback, set `PROD_TAG=5def772`, `ROLLBACK_TAG=8b0df85`,
`ROLLBACK_BACKUP=01a1cd01-f234-71e3-9540-8b06c009985f` in `/root/ops/vm-health.conf`.

### November 11, 2026 upgrade: steps 9-13, chat search filters, webhook bots, agent settings, code mode, memory per agent (schema 96)

`main` from `49bcc1c5` to `5def7724`: five `--no-ff` merges, in this order: `chat-search-filters` (`30ce235e`, step 9;
merge `cdd703b8`), `webhook-bots` (`8319a208`, step 10; `f55a084c`), `agent-settings-v2` (`b722155f`, step 11;
`e3cc60ee`), `codemode-routines` (`b9f66c14`, step 12; `1506785b`), `agent-memory-cleanup` (`432fd28b`, step 13;
`5def7724`). Steps 9 and 10 were on `2bd86666` with the pre-rebase `02aed76e` (Insert from Drive, the same content as
`8b0df85c`): no conflict. What is new for people:
- Step 9: chat search filters Mentions me, Has link, Threads only; sort Newest/Oldest; marked matches; recent searches
  (browser only). `GET .../chat/search` takes `mentions_me`, `has_link`, `threads_only`, `sort`.
- Step 10: webhook bots (Settings → Agentic → Bots, owners and admins; at most 20): a bot user with a name, avatar,
  https URL and channels; a mention or DM is posted signed to the URL; the answer within 30 s (HTTP answer or the
  single-use `/api/v1/hooks/bots/{bot}/replies/{token}`). Author docs: `docs/webhook-bots.md`.
  `ORBIT_WEBHOOK_BOT_TEST_HOSTS` is for local tests only and must stay unset here. None is set up by this deploy.
- Step 11: Donna's own card (Default) in Settings → Agentic → Agents (instructions, model; name and avatar for
  installation admins); connected services ticked per agent (a new service is on for Donna, off for the others).
- Step 12: code mode (QuickJS sandbox, `@earendil-works/pi-codemode` 1.4.0, the only new sidecar dependency), saved
  scripts, routines that run a script (writes need a yes to the exact code), Test run (a dry run in the creator's DM)
  and run history.
- Step 13: the answer level is per agent (owners and admins, on the card; `agent_preferences` and
  `GET/PUT /api/v1/agent/preferences` are gone, now 404); the Default model card is on the Agents tab and the Admin tab
  is gone (`/settings/agentic/admin` and `/agentic/admin` open Agents); memory per agent (Account settings → Agentic →
  Memories has an agent picker); Donna notes lasting facts herself.

Migrations: `0093_agent_settings`, `0094_routine_scripts`, `0095_webhook_bots`, `0096_agent_answer_level_and_memory`
(SQLite and Postgres). The webhook bots migration was `0093` on its branch and is **renumbered to `0095`** at the merge
(files, both catalogs in `migrate.rs`); no database ever applied it as 93 except the branch's test instances.
`SUPPORTED_SCHEMA_VERSION` 96. 0096 drops `agent_preferences` after it gives every agent of a workspace the most common
level of its people (production: 1 row, `normal`, so nothing changes), and adds `agent_memory_scopes.agent_user_id`
(NULL = the default agent, so all memory written so far stays Donna's).

Merge conflicts: `migrate.rs` (catalog order 93, 94, 95, 96); `apps/agent/src/prompt.ts` (`aboutOrbit()` keeps the
step 10 Bots section and "How do I" line next to the step 11-13 text; Workspace settings lists "You, Workspace, Agents,
Routines, Bots"); `App.tsx`, `settingsPages.ts`, `settingsNav.ts` and its test (the Bots tab stays, without the agent
flag, next to step 13's tabs; the Admin route redirects to Agents). What's new: the branches' lines (dated `2026-11-15`,
`2026-11-16` and on top of `2026-11-11`) form one new `2026-11-12` entry of 7 lines (Israel date of the switch; it was
23:47 UTC on November 11); the `2026-11-11` entry keeps its 10 older lines; 6 entries.

Checks on the merged `main` (after `bun install --frozen-lockfile`, `npm ci` in `apps/agent`, fresh web `dist`):
agent `tsc` and `node --test test/*.test.ts` (74 pass, 11 skipped, 0 fail); web `tsc`; web `bun test src` (1591 pass,
2 fail: the two bulk TaskList tests, which fail the same way on `49bcc1c5`); mobile `tsc`. Rust, under `orbit-build`,
with `CARGO_PROFILE_DEV_DEBUG=0` (with debug info the test binaries filled the disk at the first try; see
`.ai/lessons.md`): `cargo test -p orbit-platform -p orbit-server` on SQLite, all 76 test binaries, 1372 pass, 0 fail, 1
ignored; on Postgres (`orbit-dev-pg`) `database`, `backup`, `agent_api`, `agent_profiles`, `agent_drive`,
`agent_integrations`, `routine_scripts`, `scheduled_api`, `webhook_bots`, `chat_api`, `app_smoke`, `guest_access`
(206 pass, 0 fail). `target` removed afterwards.

Migration dry run on a read-only copy of production (`sqlite3 -readonly .backup` at 21:58 UTC, migrated at 22:38 with the merged debug binary;
again at 23:46 on a hot copy with the release binary): 4 applied, schema 92 → 96, quick-check ok, 0 foreign-key rows;
the row count of every old table unchanged; the 57 `agent_memories` rows byte-identical; their 3 scopes (1 team with
54, 2 private with 3) have `agent_user_id` NULL, so Donna's scope query (`find_scope`) finds them.

Merged e2e on a separate test instance (`/root/orbit-test/merged-9-13`, Orbit 18641, mock sidecar 18841, echo bot
18746; stopped afterwards), 8 checks passed, screenshots in
`/root/.local/share/com.jean.desktop/pasted-images/orbit-9-13/`: `09-chat-search-has-link-filter`,
`10-webhook-bot-reply-in-thread`, `11-agents-tab`, `12a-routine-test-run-history`, `12b-test-run-result-in-dm`,
`13a-memory-agent-picker-scribe`, `13b-agentic-admin-redirects-to-agents`.

`orbit-deploy image` built `orbit:5def772` (image `sha256:3063fca5fca4f103294119f16467af7cf851cf07b687aca41ba9319bd22b010b`,
finished 23:39 UTC); `deploy/smoke.sh` passed; `orbit config check` passed with the extracted binary.

At the switch: stopped `orbit-agent` and `orbit` at 23:46:59 UTC, created snapshot
`01a1cd01-f234-71e3-9540-8b06c009985f` with the old CLI (`/usr/local/bin/orbit-8b0df85`; the verified download is
`/var/backups/orbit/downloads/01a1cd01-f234-71e3-9540-8b06c009985f`) and verified it, `deploy/backup-rehearsal.sh` with
the old binary passed (verify), kept a plain copy of the stopped database as
`/var/backups/orbit/pre-migration/orbit-before-5def772.sqlite` (mode 600), ran `migrate run` of the new binary on a copy
in `/var/lib/orbit/rehearsal-5def772-switch-*` (4 applied, schema 92 → 96, quick-check ok, 0 foreign-key rows, counts
unchanged: 7 users, 72 tasks, 202 agent inputs, 864 chat messages, 69 notifications, 1 routine, 57 memories with
identical content; deleted afterwards), saved the sidecar data as
`/var/backups/orbit-agent/orbit-agent-before-5def772.tgz`, kept the old sidecar code as `/opt/orbit-agent.before-5def772`,
installed the binary (`/usr/local/bin/orbit-5def772` too) and the new sidecar (`package.json`, `package-lock.json`,
`tsconfig.json`, `src`, `npm ci --omit=dev`), and started `orbit` (23:47:40, ready 23:47:41), then `orbit-agent`
(23:47:41). `main` is **not** pushed. `vm-health` (`/root/ops/vm-health.conf`, old copy `.before-5def772`):
`PROD_TAG=5def772`, `ROLLBACK_TAG=8b0df85`, `ROLLBACK_BACKUP=01a1cd01-f234-71e3-9540-8b06c009985f`.

Verified: `orbit`, `orbit-agent`, nginx active, no restarts; readiness (all checks) and `/health/live` 200;
`/orbit-build.json` revision `5def7724`; schema 96; live quick-check ok, 0 foreign-key rows, 57 memories in 3 scopes of
the default agent; web `/`, `/settings/agentic/agents`, `/settings/agentic/bots`, `/settings/agentic/admin`,
`/profile/agentic` 200; without a session 401 for `GET .../chat/search` with the new filters, `GET`/`POST .../bots`,
`PATCH .../bots/{id}`, `POST .../bots/{id}/secret`, `GET .../bots/{id}/deliveries`, `GET .../agents`, `PATCH
.../agents/default`, `GET .../scripts`, `GET .../routines/{id}/runs`, `POST .../routines/{id}/test`, `GET
.../agent-memory/team` (also `?agent=`); a reply URL with an unknown bot 404; `GET /api/v1/agent/preferences` 404
(removed); an unknown route 404; `deploy/verify-production.sh` passes against `http://127.0.0.1:8000`; sidecar `/health`
`{"ok":true,"mock":false}` and `/opt/orbit-agent/src` equal to `apps/agent/src` of `5def7724`; agent queue empty (0
unacked inputs); the only log warning is the `rmcp` line. `vm-health` OK except nginx (1 line) and docker (`ifd-s3`).
**Not** verified on production: a signed-in check of any step 9-13 screen, a real webhook bot, a code mode or routine
test run with the real model, the phone app.

Rollback (the old binary refuses schema 96: restore the snapshot together with the binary; work after 23:47 UTC is
lost). To go back to `8b0df85`:

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-5def772 backup verify 01a1cd01-f234-71e3-9540-8b06c009985f && /usr/local/bin/orbit-5def772 backup restore 01a1cd01-f234-71e3-9540-8b06c009985f'
sudo install -m 755 /usr/local/bin/orbit-8b0df85 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-5def772 /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-5def772.tgz
# Plain copy of the database before the migration: /var/backups/orbit/pre-migration/orbit-before-5def772.sqlite
```

After this rollback, set `PROD_TAG=8b0df85`, `ROLLBACK_TAG=65d7d0b`,
`ROLLBACK_BACKUP=01a1cb31-1edc-7650-974f-06d9b635919c` in `/root/ops/vm-health.conf`.

### November 11, 2026 upgrade: step 7, Insert from Drive (schema 92, no migration)

`main` from `c4b7c163` to `8b0df85c`: fast-forward to branch `insert-from-drive` (`02aed76e` rebased on `main`, now
`8b0df85c`; the rebase had no conflict). Docs: `/` → Insert from Drive picks a file of the workspace's Drive; the
`driveFile` block holds a reference (workspace + node id), never a copy, and every read resolves it for the reader:
available (images and videos inline, other files as a card that opens the Drive preview), locked (no Drive access, no
name) or unavailable (trash, deleted). Export and the token API show only what the reader may see; the phone app shows a
plain "Drive file" card. Donna's new tools `list_page_drive_files`, `find_drive_file_pages`, `insert_drive_file`,
`replace_drive_file`, `remove_drive_file` (as the asking person, no approval), in `TOOL_CATALOG` and `route_need`
for custom agents. New API `/api/v1/workspaces/{w}/pages/{p}/drive-files[/resolve|/{block}]` and
`/pages/drive-file-uses/{node}`. No migration, no new `ORBIT_*` setting, no new dependency; sidecar
`package.json`/`package-lock.json`/`tsconfig.json` unchanged (only `src`). `aboutOrbit()` describes Insert from Drive
and the tools. What's new: 1 line on top of the `2026-11-11` entry (now 10 lines; 5 entries; the other lines unchanged).

Checks before the merge, in the worktree after the rebase: agent `tsc` and `node --test test/*.test.ts` (70 pass, 11
skipped, 0 fail); web `bun test src/features/docs src/features/askDonna` (330 pass) and `tsc`; `packages/editor` tests
(19 pass); mobile `tsc`; web `bun run build`; orbit-server `--lib` (261) and the test binaries `page_drive_files_api`,
`guest_access`, `openapi_contract`, `agent_profiles`, `agent_api`, `agent_drive`, `collab_convert`,
`doc_markdown_writes`, `collab_reconcile`, `markdown_merge`, `export_api`, `export_markdown`, `workspace_export_api`,
`public_api`, `drive_api`, `pages_api`, `app_smoke` (443 tests in all, 0 fail), SQLite, under `orbit-build` (its
`target` removed afterwards). Postgres and the full suites were not run. `orbit-deploy image` built `orbit:8b0df85`
(image `sha256:3fff57285c5e0e55367ecd38cd4c399bd1b53fb1dfc46324dcb816576ec258bd`, finished 15:18 UTC);
`deploy/smoke.sh` passed; `orbit config check` passed with the extracted binary.

At the switch: stopped `orbit-agent` and `orbit` at 15:19:16 UTC, created snapshot `01a1cb31-1edc-7650-974f-06d9b635919c` with the old CLI
(`/usr/local/bin/orbit-65d7d0b`; the verified download is `/var/backups/orbit/downloads/01a1cb31-1edc-7650-974f-06d9b635919c`) and verified it,
`deploy/backup-rehearsal.sh` with the old binary passed (verify), ran `migrate run` of the new binary on a copy in
`/var/lib/orbit/rehearsal-8b0df85-*` (0 applied, schema 92, quick-check ok; deleted afterwards), saved the sidecar data
as `/var/backups/orbit-agent/orbit-agent-before-8b0df85.tgz`, kept the old sidecar code as
`/opt/orbit-agent.before-8b0df85`, installed the binary (`/usr/local/bin/orbit-8b0df85` too) and the new sidecar `src`,
and started `orbit` (15:19:30, ready), then `orbit-agent` (15:19:34). `main` is **not** pushed. `vm-health`
(`/root/ops/vm-health.conf`, old copy `.before-8b0df85`): `PROD_TAG=8b0df85`, `ROLLBACK_TAG=65d7d0b`,
`ROLLBACK_BACKUP=01a1cb31-1edc-7650-974f-06d9b635919c`.

Verified: `orbit`, `orbit-agent`, nginx active, no restarts; readiness (all checks) and `/health/live` 200;
`/orbit-build.json` revision `8b0df85c`; schema 92; live quick-check ok, 0 foreign-key rows; web `/` and its main JS
asset 200; without a session the new routes answer 401 (`GET`/`POST .../drive-files`, `POST .../drive-files/resolve`,
`PUT`/`DELETE .../drive-files/{block}`, `GET .../pages/drive-file-uses/{node}`; a POST/PUT body without its required
fields gives 400 first); an unknown route 404; `deploy/verify-production.sh` passes against `http://127.0.0.1:8000`;
sidecar `/health` `{"ok":true,"mock":false}` and `/opt/orbit-agent/src` equal to `apps/agent/src` of `8b0df85c`;
agent queue empty; the only log warning is the `rmcp` line. `vm-health` OK except nginx (35 lines, restarts), docker
(`ifd-s3`) and git-owner. **Not** verified on production: a signed-in Insert from Drive (the branch's e2e run covers
the picker, inline image and video, PDF card, locked card, unavailable file and Donna's insert), the phone app, a Donna
Drive-file tool call.

Rollback (no migration: the old binary runs on schema 92, no restore needed). To go back to `65d7d0b`:

```sh
sudo systemctl stop orbit-agent orbit
sudo install -m 755 /usr/local/bin/orbit-65d7d0b /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-8b0df85 /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# Data, only if needed (work after 15:19 UTC is lost): /usr/local/bin/orbit-65d7d0b backup restore 01a1cb31-1edc-7650-974f-06d9b635919c (as orbit, with /etc/orbit/orbit.env)
```

Pages that got Drive file blocks after the switch keep them in their content; the old web app does not know the
`driveFile` block. After this rollback, set `PROD_TAG=65d7d0b`, `ROLLBACK_TAG=c198b30`,
`ROLLBACK_BACKUP=01a1cb00-c6ae-7195-8014-497bb566df86` in `/root/ops/vm-health.conf`.

### November 11, 2026 upgrade: the Ask bar needs a ChatGPT connection (schema 92, no migration)

`main` from `2bd86666` to `65d7d0b6`: cherry-pick of `d30f4de8` (branch `agent-profiles`, on `6673a3f5`). The Ask bar
and its panel show only for a member whose ChatGPT is connected (as well as the agent on and a member role); losing the
connection closes an open panel (`useAskDonna`, `useAskDonna.test.tsx`). `aboutOrbit()` and the Ask bar line of the
`2026-11-11` what's-new entry say so (the only conflict: main's merged entry kept, that line replaced; still 9 lines, 5
entries). No migration, no new `ORBIT_*` setting, no new dependency; sidecar `package.json`/`package-lock.json`/
`tsconfig.json` unchanged (only `src`).

Checks before the switch (after `bun install --frozen-lockfile` and `npm ci` in `apps/agent`): web `bun test
src/features/askDonna` (10 pass) and `tsc --noEmit`; agent `tsc` and `node --test test/*.test.ts` (68 pass, 11
skipped, 0 fail). Rust tests not run (no Rust change). `orbit-deploy image` built `orbit:65d7d0b` (image
`sha256:428b6f9741b81d9e3101863629a9db3f1b8e4461b291f25314a91e9b332b195f`, finished 14:25 UTC, after the webhook-bots
`cargo test` released `orbit-build`); `deploy/smoke.sh` passed; `orbit config check` passed with the extracted binary.

At the switch: stopped `orbit-agent` and `orbit` at 14:26:27 UTC, created snapshot
`01a1cb00-c6ae-7195-8014-497bb566df86` with the old CLI (`/usr/local/bin/orbit-c198b30`; the verified download is
`/var/backups/orbit/downloads/01a1cb00-c6ae-7195-8014-497bb566df86`) and verified it, `deploy/backup-rehearsal.sh`
with the old binary passed (verify), ran `migrate run` of the new binary on a copy in
`/var/lib/orbit/rehearsal-65d7d0b-*` (0 applied, schema 92, quick-check ok; deleted afterwards), saved the sidecar data
as `/var/backups/orbit-agent/orbit-agent-before-65d7d0b.tgz`, kept the old sidecar code as
`/opt/orbit-agent.before-65d7d0b`, installed the binary (`/usr/local/bin/orbit-65d7d0b` too) and the new sidecar `src`,
and started `orbit` (14:27:01, ready), then `orbit-agent` (14:27:05). `main` is **not** pushed. `vm-health`
(`/root/ops/vm-health.conf`, old copy `.before-65d7d0b`): `PROD_TAG=65d7d0b`, `ROLLBACK_TAG=c198b30`,
`ROLLBACK_BACKUP=01a1cb00-c6ae-7195-8014-497bb566df86`.

Verified: `orbit`, `orbit-agent`, nginx active, no restarts; readiness (all checks) and `/health/live` 200;
`/orbit-build.json` revision `65d7d0b6`; schema 92; live quick-check ok, 0 foreign-key rows; web `/` and its main JS
asset 200; `deploy/verify-production.sh` passes against `http://127.0.0.1:8000` (`X-Forwarded-Proto: https` via a
`CURL_HOME` `.curlrc`); sidecar `/health` `{"ok":true,"mock":false}` and `/opt/orbit-agent/src` equal to
`apps/agent/src` of `65d7d0b6`; the only log warning is the `rmcp` "allowed origin without a port" line. `vm-health` OK
except nginx (21 lines, restarts), docker (`ifd-s3`) and git-owner. Signed-in browser check (temporary v7 session rows,
socat TLS hop, Playwright; both sessions logged out, revoked): a member without ChatGPT sees no Ask bar on
`/profile/agentic` and `/tasks`; a member with ChatGPT sees it on both. **Not** verified: the desktop app, a question
through the bar after the switch.

Rollback (no migration: the old binary runs on schema 92, no restore needed). To go back to `c198b30`:

```sh
sudo systemctl stop orbit-agent orbit
sudo install -m 755 /usr/local/bin/orbit-c198b30 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-65d7d0b /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# Data, only if needed (work after 14:26 UTC is lost): /usr/local/bin/orbit-c198b30 backup restore 01a1cb00-c6ae-7195-8014-497bb566df86 (as orbit, with /etc/orbit/orbit.env)
```

After this rollback, set `PROD_TAG=c198b30`, `ROLLBACK_TAG=7ae0434`,
`ROLLBACK_BACKUP=01a1c9f2-7f91-7301-9967-9efb75b713de` in `/root/ops/vm-health.conf`.

### November 11, 2026 upgrade: step 6, agent profiles (schema 92)

`main` from `7dda9885` to `c198b304`: merge commit of branch `agent-profiles` (`6673a3f5`, on `5687ce3c`). Owners and
admins create more agents next to Donna (Settings → Agentic → Agents: name, avatar, instructions, a subset of the
default agent's tools, model and thinking level, channels; at most 20 per workspace, none made by this deploy); people
@mention one in its channels or DM it, it works as the driver with the same approvals; Orbit limits a custom agent's
session to the routes of its tools (scope layer, fail closed) and the sidecar offers only those tools. Answer level
(Account settings → Agentic: Short, Normal, Thorough) for every agent. Routines can run with any agent; deleting an
agent pauses its routines. Guests see no agent. New API `/api/v1/workspaces/{w}/agents[/{id}]` and
`/api/v1/agent/preferences`. Migration `0092_agent_profiles` (SQLite and Postgres): new tables `agent_profiles`,
`agent_profile_channels`, `agent_preferences` (additive). No new `ORBIT_*` setting, no new dependency; sidecar
`package.json`/`package-lock.json`/`tsconfig.json` unchanged (only `src`), so server and sidecar switched together.
`aboutOrbit()` describes agents and the answer level.

Merge (conflicts with step 5's approval fix `c00bbe05`): what's new: the branch's 3 lines (dated `2026-11-14`; its
`2026-11-13` copy of the step 5 lines dropped) go on top of the `2026-11-11` entry (now 9 lines; 5 entries).
`scheduled/store.rs`: both new helper groups kept. So that custom agents keep the driver's approval for
`schedule_routine` and `send_later`: their route map (`profiles::route_need`) opens `POST /agent-tools/scheduled` for
the three scheduling tools and only `GET` on `/routines` and `/scheduled-messages`; the handler checks the exact tool
against the agent's tools (as `/agent-tools/drive` does); a routine's preview and creation use the session's agent
(its channels checked before the driver is asked; the agent is part of the approved input). The `agent_profiles` test
now makes an agent's own routine through that approval path (and checks a refused tool, a refused channel and the 403
on `POST /routines`).

Checks before the switch (after `bun install --frozen-lockfile` and `npm ci` in `apps/agent`; the full suites passed on
the branch; the known 2 TaskList web failures, the clippy lint in `collab/sanitize.rs` and the `markdown_merge` timing
flake were not rerun): agent `tsc` and `node --test test/*.test.ts` (68 pass, 11 skipped, 0 fail; the what's-new test
included); web `bun run build` (fresh `dist`); orbit-server `agent_api` (17), `agent_drive` (4), `agent_profiles` (6)
and `scheduled_api` (11) all pass, and the `agent::scope`, `agent::profiles`, `scheduled` unit tests (12) pass, SQLite,
under `orbit-build` (waited for a build of the `agent-profiles` worktree). Postgres was not run. `orbit-deploy image`
built `orbit:c198b30` (08:54-09:24 UTC); `deploy/smoke.sh` passed; `orbit config check` passed with the extracted
binary. A migration rehearsal on a hot copy (`sqlite3 .backup`, before the stop) gave the same result as the one below.

At the switch: stopped `orbit-agent` and `orbit` at 09:31:14 UTC, created snapshot
`01a1c9f2-7f91-7301-9967-9efb75b713de` with the old CLI (`/usr/local/bin/orbit-7ae0434`; the verified download is
`/var/backups/orbit/downloads/01a1c9f2-7f91-7301-9967-9efb75b713de`) and verified it, `deploy/backup-rehearsal.sh`
with the old binary passed (verify), ran `migrate run` of the new binary on a copy in
`/var/lib/orbit/rehearsal-c198b30-switch-*` (1 applied, schema 91 → 92, quick-check ok, 0 foreign-key rows, the row
count of every old table unchanged: 7 users, 71 tasks, 185 agent inputs, 808 chat messages, 63 notifications, 0
routines; the 3 new tables empty; deleted afterwards), saved the sidecar data as
`/var/backups/orbit-agent/orbit-agent-before-c198b30.tgz`, kept the old sidecar code as
`/opt/orbit-agent.before-c198b30`, installed the binary (`/usr/local/bin/orbit-c198b30` too) and the new sidecar `src`,
and started `orbit` (09:31:37, ready 09:31:38), then `orbit-agent` (09:31:38). `main` is **not** pushed. `vm-health`
(`/root/ops/vm-health.conf`, old copy `.before-c198b30`): `PROD_TAG=c198b30`, `ROLLBACK_TAG=7ae0434`,
`ROLLBACK_BACKUP=01a1c9f2-7f91-7301-9967-9efb75b713de`.

Verified: `orbit`, `orbit-agent`, nginx active, no restarts; readiness (all checks, `scheduler` included) and
`/health/live` 200; `/orbit-build.json` revision `c198b304`; schema 92; live quick-check ok, 0 foreign-key rows; web `/`
and its main JS asset 200; without a session `GET`/`POST .../agents`, `GET /api/v1/agent/preferences`, `GET
.../routines`, `GET .../scheduled-messages` and `POST .../agent-tools/scheduled` answer 401, `GET .../agents/x` 405;
`deploy/verify-production.sh` passes against `http://127.0.0.1:8000` (`ORBIT_VERIFY_ORIGIN`, `X-Forwarded-Proto: https`
via a `CURL_HOME` `.curlrc`); sidecar `/health` `{"ok":true,"mock":false}` and `/opt/orbit-agent/src` equal to
`apps/agent/src` of `c198b304`; the agent queue was empty after the start (0 unacked inputs, no thread running). The
only log warning is the `rmcp` "allowed origin without a port" line of every start. `vm-health` OK except nginx (19
lines, upstream refused during the 03:25 and 09:31 restarts), docker (another project's `ifd-s3` container) and
git-owner (root-owned files in `.git`). **Not** verified: a signed-in browser check (Settings → Agentic → Agents,
Account settings → Answer level), the phone app, a real message to Donna or to a custom agent after the switch (none
arrived during the checks), a routine of a custom agent.

Rollback (the old binary refuses schema 92: restore the snapshot together with the binary; work after 09:31 UTC is
lost). To go back to `7ae0434`:

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-c198b30 backup verify 01a1c9f2-7f91-7301-9967-9efb75b713de && /usr/local/bin/orbit-c198b30 backup restore 01a1c9f2-7f91-7301-9967-9efb75b713de'
sudo install -m 755 /usr/local/bin/orbit-7ae0434 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-c198b30 /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-c198b30.tgz
```

After this rollback, set `PROD_TAG=7ae0434`, `ROLLBACK_TAG=5b63556`,
`ROLLBACK_BACKUP=01a1c973-0106-7325-9644-8dc368cb7b28` in `/root/ops/vm-health.conf`.

### November 11, 2026 upgrade: step 5, routines, reminders and send later (schema 91)

`main` from `a050b765` to `7ae0434f`: merge commit of branch `donna-routines` (`5687ce3c`, `c00bbe05`, on `cd052333`);
the merge resolves the what's-new conflict: the branch's 3 lines (dated `2026-11-13`; its `2026-11-12` copy of the
followers line dropped) go on top of the `2026-11-11` entry (now 6 lines; 6 entries). Routines: a member asks Donna in
chat for a recurring job (cron in a time zone), run as the creator with their permissions and posted in the chosen
channel or DM; Settings → Agentic → Routines lists, pauses, resumes, edits and deletes them (25 per person per
workspace, at most every 15 minutes). Reminders: Donna DMs the person at the time. Send later: the arrow next to Send
in the composer; Scheduled messages edits or cancels. Guests can use none of it. Donna's `schedule_routine` and
`send_later` need the driver's yes (agent sessions create only through `/agent-tools/scheduled`); `remind_me` needs
none. `aboutOrbit()` describes all of it. One server loop (`apps/server/src/scheduled/runner.rs`, every 10 s) runs due
work. New API `/api/v1/workspaces/{w}/routines[/{id}]` and `/scheduled-messages[/{id}]`. Migration
`0091_routines_and_scheduled_messages`: creates `agent_routines` and `scheduled_messages`, `ALTER TABLE agent_inputs
ADD COLUMN routine_id` (SQLite and Postgres; additive). New Rust dependency `chrono-tz` 0.10. No new `ORBIT_*` setting;
sidecar `package.json`/`package-lock.json`/`tsconfig.json` unchanged (only `src`, with the new `routine` field on agent
inputs, so server and sidecar switched together).

Checks before the switch (after `bun install --frozen-lockfile` and `npm ci` in `apps/agent`; the full suites passed on
the branch; the 2 TaskList web test failures, the clippy lint in `collab/sanitize.rs` and the fmt diff in
`crates/platform/tests/http_security.rs` also exist on main): agent `tsc` and `node --test test/*.test.ts` (64 pass,
11 skipped, 0 fail; the what's-new test included); web `bun run build` (fresh `dist`); orbit-server `agent_api` (17
pass) and `scheduled_api` (11 pass), SQLite, under `orbit-build` (after the step 6 `agent-profiles` build released
it). Postgres was not run. `orbit-deploy image` built `orbit:7ae0434` with `chrono-tz` (06:25-07:05 UTC);
`deploy/smoke.sh` passed; `orbit config check` passed with the extracted binary. A migration rehearsal on a hot copy
(`sqlite3 .backup`, before the stop) gave the same result as the one below.

At the switch: stopped `orbit-agent` and `orbit` at 07:11:59 UTC, created snapshot
`01a1c973-0106-7325-9644-8dc368cb7b28` with the old CLI (`/usr/local/bin/orbit-5b63556`; stored in S3, the verified
download is `/var/backups/orbit/downloads/01a1c973-0106-7325-9644-8dc368cb7b28`) and verified it,
`deploy/backup-rehearsal.sh` with the old binary passed (verify), ran `migrate run` of the new binary on a copy in
`/var/lib/orbit/rehearsal-7ae0434-switch-*` (1 applied, schema 90 → 91, quick-check ok, 0 foreign-key rows, the row
count of every old table unchanged: 7 users, 71 tasks, 180 agent inputs, 797 chat messages, 63 notifications; the 2 new
tables empty; deleted afterwards), saved the sidecar data as `/var/backups/orbit-agent/orbit-agent-before-7ae0434.tgz`,
kept the old sidecar code as `/opt/orbit-agent.before-7ae0434`, installed the binary (`/usr/local/bin/orbit-7ae0434`
too) and the new sidecar `src`, and started `orbit` (07:12:15, ready), then `orbit-agent` (07:12:16). `main` is
**not** pushed. `vm-health` (`/root/ops/vm-health.conf`, old copy `.before-7ae0434`): `PROD_TAG=7ae0434`,
`ROLLBACK_TAG=5b63556`, `ROLLBACK_BACKUP=01a1c973-0106-7325-9644-8dc368cb7b28`.

Verified: `orbit`, `orbit-agent`, nginx, `orbit-oidc-gateway` and `orbit-oidc-tunnel` active, no restarts; readiness
(all checks, `scheduler` included) and `/health/live` 200; `/orbit-build.json` revision `7ae0434f`; schema 91; live
quick-check ok, 0 foreign-key rows; web `/` and its main JS asset 200; without a session `GET .../routines` and
`GET .../scheduled-messages` answer 401, `POST .../agent-tools/scheduled` 403, `GET .../routines/x` 405 (PATCH/DELETE
only), an unknown route 404; `deploy/verify-production.sh` passes against `http://127.0.0.1:8000` with
`X-Forwarded-Proto: https` (sent via a `CURL_HOME` `.curlrc`); sidecar `/health` `{"ok":true,"mock":false}` and
`/opt/orbit-agent/src` equal to `apps/agent/src` of `7ae0434f`. The scheduler loop has no start log line: it runs as a
background service (one that stops ends the server) and logged no warning after the start; the only log warning is the
`rmcp` "allowed origin without a port" line that each start also logged before. `vm-health` OK except disk 81% and one
nginx line (07:12:15, upstream refused during the start). **Not** verified: a signed-in browser check (Settings →
Agentic → Routines, Send later and Scheduled messages in the composer), the phone app, a real routine run, reminder or
scheduled send, Donna's approval card for `schedule_routine` / `send_later`.

Rollback (the old binary refuses schema 91: restore the snapshot together with the binary; work after 07:11 UTC is
lost). To go back to `5b63556`:

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-7ae0434 backup verify 01a1c973-0106-7325-9644-8dc368cb7b28 && /usr/local/bin/orbit-7ae0434 backup restore 01a1c973-0106-7325-9644-8dc368cb7b28'
sudo install -m 755 /usr/local/bin/orbit-5b63556 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-7ae0434 /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-7ae0434.tgz
```

After this rollback, set `PROD_TAG=5b63556`, `ROLLBACK_TAG=6988352`,
`ROLLBACK_BACKUP=01a1c8a3-1238-7233-8587-e9164866681c` in `/root/ops/vm-health.conf`.

### November 11, 2026 upgrade: step 4, task followers (schema 90)

`main` from `ebb79e0a` to `5b63556e`: merge commit `5807a3ed` of branch `task-followers` (one commit, `cd052333`, on
`69883527`), then `5b63556e` moved its what's-new line from a `2026-11-12` entry into the `2026-11-11` entry (now 3
lines, on top; 6 entries). The creator, assignees, commenters and @mentioned people of a task follow it; followers get
an Activity notification and a push for each new comment and status change (not their own); the Followers field of a
task (web and phone) has Follow / Unfollow; Account settings → Notifications has "Comments and status changes on tasks I
follow". New API `/api/v1/workspaces/{workspace_id}/tasks/{task_id}/followers` (and `/followers/me`). Donna has a
`follow_task` tool, and `aboutOrbit()` describes followers. Migration `0090_task_followers`: creates `task_followers`,
backfills it, and **rebuilds** the SQLite `notifications` table (Postgres: its own file). No new `ORBIT_*` setting;
sidecar `package.json`/`package-lock.json`/`tsconfig.json` unchanged (only `src`).

Checks before the switch (after `bun install --frozen-lockfile` and `npm ci` in `apps/agent`; the full suites passed on
the branch; the 2 TaskList bulk-toolbar web test failures and one clippy lint in `collab/sanitize.rs` also exist on
main): agent `tsc` and `node --test test/prompt.test.ts` (5 pass, the what's-new test included); web `bun run build`
(fresh `dist`); orbit-server `task_followers_api` (5 pass), `notifications_api` (3 pass), `push_api` (6 pass), SQLite,
under `orbit-build`. Postgres was not run. `orbit-deploy image` built `orbit:5b63556` (about 40 min);
`deploy/smoke.sh` passed; `orbit config check` passed with the extracted binary. A migration rehearsal on a hot copy
(schema 89 → 90, before the stop) gave the same result as the one below.

At the switch: stopped `orbit-agent` and `orbit` at 03:24:52 UTC, created snapshot
`01a1c8a3-1238-7233-8587-e9164866681c` with the old CLI (`/usr/local/bin/orbit-6988352`; stored in S3, the verified
download is `/var/backups/orbit/downloads/01a1c8a3-1238-7233-8587-e9164866681c`) and verified it,
`deploy/backup-rehearsal.sh` with the old binary passed (verify), ran `migrate run` of the new binary on a copy in
`/var/lib/orbit/rehearsal-5b63556-switch-*` (1 applied, schema 90, quick-check ok, 0 foreign-key rows, counts
unchanged: 7 users, 71 tasks, 180 agent inputs, 63 notifications with the same count for each kind; 97 follower rows on
71 tasks; deleted afterwards), saved the sidecar data as `/var/backups/orbit-agent/orbit-agent-before-5b63556.tgz`,
kept the old sidecar code as `/opt/orbit-agent.before-5b63556`, installed the binary (`/usr/local/bin/orbit-5b63556`
too) and the new sidecar `src`, and started `orbit` (03:25:16, ready), then `orbit-agent` (03:25:19). `main` is
**not** pushed. `vm-health` (`/root/ops/vm-health.conf`, old copy `.before-5b63556`): `PROD_TAG=5b63556`,
`ROLLBACK_TAG=6988352`, `ROLLBACK_BACKUP=01a1c8a3-1238-7233-8587-e9164866681c`.

Verified: `orbit`, `orbit-agent`, nginx, `orbit-oidc-gateway` and `orbit-oidc-tunnel` active, no restarts; readiness
(all checks) and `/health/live` 200; `/orbit-build.json` revision `5b63556e`; schema 90; live quick-check ok, 0
foreign-key rows, 97 `task_followers` rows, 63 notifications; web `/` and its main JS asset 200; `GET
/api/v1/workspaces/{w}/tasks/{t}/followers` without a session answers 401 `authentication_required` (an unknown route
404); `deploy/verify-production.sh` passes against `http://127.0.0.1:8000` with `X-Forwarded-Proto: https` (sent via a
`CURL_HOME` `.curlrc`); sidecar `/health` `{"ok":true,"mock":false}` and `/opt/orbit-agent/src` equal to
`apps/agent/src` of `5b63556e`; no log warnings; `vm-health` OK except the nginx lines of the switch window (37, all
03:24-03:25, upstream refused while stopped) and the known `git-owner` warning. **Not** verified: a signed-in browser
check (Followers field, Follow / Unfollow, a follower notification and push), the phone app, Donna's `follow_task`.

Rollback (the old binary refuses schema 90: restore the snapshot together with the binary; work after 03:24 UTC is
lost). To go back to `6988352`:

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-5b63556 backup verify 01a1c8a3-1238-7233-8587-e9164866681c && /usr/local/bin/orbit-5b63556 backup restore 01a1c8a3-1238-7233-8587-e9164866681c'
sudo install -m 755 /usr/local/bin/orbit-6988352 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-5b63556 /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-5b63556.tgz
```

After this rollback, set `PROD_TAG=6988352`, `ROLLBACK_TAG=2d72c1c`,
`ROLLBACK_BACKUP=01a1c7f3-40b8-77e1-a4b1-a766211bdfcb` in `/root/ops/vm-health.conf`.

### November 11, 2026 upgrade: step 3, the Ask Donna bar (schema 89)

`main` fast-forwarded from `531b0d70` to `69883527` (branch `ask-donna-bar`, one commit): a small "Ask Donna…" box at
the bottom center of every page but Chat (web and desktop app, not for guests); Enter sends to the person's DM with
Donna and opens a panel on the right (Open in Chat, ×, files). Each send from the box or the panel carries
`page_context` (area and the open task, Docs page or Drive folder), stored on the agent input only and shown to the
model as data (`apps/agent/src/page.ts`). Migration `0089_agent_page_context`: `ALTER TABLE agent_inputs ADD COLUMN
page_context TEXT` (SQLite and Postgres). `aboutOrbit()` has the new "Ask bar" section and a "How do I… use you" line;
"what's new" has a new `2026-11-11` entry (2 lines, on top; 6 entries). No new `ORBIT_*` setting; sidecar
`package.json`/`package-lock.json`/`tsconfig.json` unchanged (only `src`).

Checks before the switch (after `bun install --frozen-lockfile` and `npm ci` in `apps/agent`; the full suites passed on
the branch, and the 2 TaskList bulk-toolbar web test failures also fail on main): agent `tsc` and `node --test
test/page.test.ts` (4 pass); web `bun run build` (fresh `dist`); orbit-server `agent_api` (16 pass, SQLite). Postgres
was not run. `orbit-deploy image` built `orbit:6988352` (34 min); `deploy/smoke.sh` passed; `orbit config check`
passed with the extracted binary. A migration rehearsal on a hot copy gave the same result as the one below.

At the switch: stopped `orbit-agent` and `orbit` at 00:12:50 UTC, created snapshot
`01a1c7f3-40b8-77e1-a4b1-a766211bdfcb` with the old CLI (`/usr/local/bin/orbit-2d72c1c`; stored in S3, the verified
download is `/var/backups/orbit/downloads/01a1c7f3-40b8-77e1-a4b1-a766211bdfcb`) and verified it,
`deploy/backup-rehearsal.sh` with the old binary passed (verify), ran `migrate run` of the new binary on a copy in
`/var/lib/orbit/rehearsal-6988352-switch-*` (1 applied, schema 89, quick-check ok, 0 foreign-key rows, counts
unchanged: 7 users, 7 memberships, 751 chat messages, 67 tasks, 157 agent inputs; deleted afterwards), saved the
sidecar data as `/var/backups/orbit-agent/orbit-agent-before-6988352.tgz`, kept the old sidecar code as
`/opt/orbit-agent.before-6988352`, installed the binary (`/usr/local/bin/orbit-6988352` too) and the new sidecar `src`,
and started `orbit` (00:13:32, ready), then `orbit-agent` (00:13:42). `main` is **not** pushed. `vm-health`
(`/root/ops/vm-health.conf`, old copy `.before-6988352`): `PROD_TAG=6988352`, `ROLLBACK_TAG=2d72c1c`,
`ROLLBACK_BACKUP=01a1c7f3-40b8-77e1-a4b1-a766211bdfcb`.

Verified: `orbit`, `orbit-agent`, nginx, `orbit-oidc-gateway` and `orbit-oidc-tunnel` active, no restarts; readiness
(all checks) and `/health/live` 200; `/orbit-build.json` revision `69883527`; schema 89; live quick-check and
foreign-key checks pass; web `/` 200; `deploy/verify-production.sh` passes against `http://127.0.0.1:8000` with
`X-Forwarded-Proto: https`; sidecar `/health` `{"ok":true,"mock":false}` and `/opt/orbit-agent/src` equal to
`apps/agent/src` of `69883527`; no log warnings; `vm-health` OK except the nginx lines of the switch window (21, all
00:13, upstream refused while stopped). Signed-in browser check as the owner ddusi@wellrox.com (temporary session
row, 30 min, revoked by `POST /api/v1/auth/logout` (204), then 401; Playwright through a local TLS hop): `/tasks` shows
the "Ask Donna…" box, `/chat` has none; Enter on Tasks sent `page_context {"page":"tasks","item":null}` (200), the
agent input stored it, the panel opened, and Donna answered "You're on the **Tasks** page." The test question and
answer stay in the owner's DM with Donna. Console: 3 × 400 for `/sw.js` (known artifact of the added header) and 4 ×
404 (not investigated). **Not** verified: an item context (open task, Docs page, Drive folder), the desktop app, and
a `member` role account.

Rollback (the old binary refuses schema 89: restore the snapshot together with the binary; work after 00:12 UTC is
lost). To go back to `2d72c1c`:

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-6988352 backup verify 01a1c7f3-40b8-77e1-a4b1-a766211bdfcb && /usr/local/bin/orbit-6988352 backup restore 01a1c7f3-40b8-77e1-a4b1-a766211bdfcb'
sudo install -m 755 /usr/local/bin/orbit-2d72c1c /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-6988352 /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-6988352.tgz
```

After this rollback, set `PROD_TAG=2d72c1c`, `ROLLBACK_TAG=6bfa8a7`,
`ROLLBACK_BACKUP=01a1c722-ed80-71dc-81f7-0cd456bf1feb` in `/root/ops/vm-health.conf`.

### November 10, 2026 upgrade: step 2, chat-only guest role (schema 88)

`main` from `6d92dbb2` to `2d72c1c7`: the three commits of branch `guest-role` (based on `a249fefa`) cherry-picked on
top, the same linear result as a rebase and fast-forward (the tree differs from the branch only by `6d92dbb2`'s
workflow file): `44be5af5` (port of upstream `5821bc44`, #86: role Guest; owners and admins invite a guest to chosen
private channels; a guest sees only those channels, direct messages with people there and their account settings; no
public channels, Home, Activity, tasks, Docs, Drive, apps, API tokens or Donna; migration `0088_guests`: SQLite
rebuilds `workspace_invitations` and `memberships` to add `guest` to their CHECK, Postgres only changes the
constraints; both add `workspace_invitation_conversations`), `1adbadb1` (agent test reads every chunk), `2d72c1c7`
(lessons). The branch already had the line on top of the `2026-11-10` "what's new" entry (Guests; two older lines merged
to keep 4) and the `aboutOrbit()` "Guests" section. No new `ORBIT_*` setting; sidecar `package.json`/`package-lock.json`
unchanged (only `src`). After the deploy, `e1403955` (test only): the two agent prompt tests stream the system prompt
at 4000 tokens/s; at the default 400 it is longer than their 15 s wait (they failed on the branch too).

Checks before the switch (after `bun install --frozen-lockfile` and `npm ci` in `apps/agent`): agent `tsc` and
`node --test` (60 pass, 0 fail, 11 skipped, with `e1403955`); web `tsc -b` and `bun test` of the 4 changed test files
(21 pass); mobile `tsc` and `bun test` of `guest`, `chat`, `account-api` (37 pass); Rust (SQLite): orbit-domain
`policies` (15), orbit-platform `database` (50) and `backup` (23), orbit-server `guest_access` (1) and `app_smoke` (21).
(`guest_access` and `app_smoke` first failed with `StaticAssets(RevisionMismatch)`: this checkout's `apps/web/dist` was
an old build; a fresh web build fixed it.) Postgres was not run. `orbit-deploy image` built `orbit:2d72c1c` (37 min);
`deploy/smoke.sh` passed; `orbit config check` passed with the extracted binary. A migration rehearsal on a hot copy
gave the same result as the one below.

At the switch: stopped `orbit-agent` and `orbit` at 20:25:17 UTC, created snapshot
`01a1c722-ed80-71dc-81f7-0cd456bf1feb` with the old CLI (`/usr/local/bin/orbit-6bfa8a7`; stored in S3, the verified
download is `/var/backups/orbit/downloads/01a1c722-ed80-71dc-81f7-0cd456bf1feb`) and verified it,
`deploy/backup-rehearsal.sh` with the old binary passed (verify), ran `migrate run` of the new binary on a copy in
`/var/lib/orbit/rehearsal-2d72c1c-switch-*` (1 applied, schema 88, quick-check ok, 0 foreign-key rows, counts
unchanged: 7 users, 1 workspace, 7 memberships (6 member, 1 owner), 7 invitations, 66 task assignees, 62 chat members,
74 thread members, 3 projects, 67 tasks, 53 pages, 745 chat messages, 154 agent inputs; owner check and the 5
`memberships` triggers present; deleted afterwards), saved the sidecar data as
`/var/backups/orbit-agent/orbit-agent-before-2d72c1c.tgz`, kept the old sidecar code as
`/opt/orbit-agent.before-2d72c1c`, installed the binary (`/usr/local/bin/orbit-2d72c1c` too) and the new sidecar
`src`, and started `orbit` (20:25:57, ready), then `orbit-agent` (20:26:02). `main` is **not** pushed. `vm-health`
(`/root/ops/vm-health.conf`, old copy `.before-2d72c1c`): `PROD_TAG=2d72c1c`, `ROLLBACK_TAG=6bfa8a7`,
`ROLLBACK_BACKUP=01a1c722-ed80-71dc-81f7-0cd456bf1feb`.

Verified: `orbit`, `orbit-agent`, nginx, `orbit-oidc-gateway` and `orbit-oidc-tunnel` active, no restarts; readiness
(all checks) and `/health/live` 200; `/orbit-build.json` revision `2d72c1c7`; schema 88; live quick-check and
foreign-key checks pass; web `/` 200; `deploy/verify-production.sh` passes against `http://127.0.0.1:8000` with
`X-Forwarded-Proto: https`; sidecar `/health` `{"ok":true,"mock":false}` and `/opt/orbit-agent/src` equal to
`apps/agent/src` of `2d72c1c7` (the `2026-11-10` entry starts with the Guests line); no log warnings; `vm-health` OK
except the nginx lines of the switch window (65, all 20:25, upstream refused while stopped). Signed-in API check as the
owner ddusi@wellrox.com (temporary session row, 30 min, revoked by `POST /api/v1/auth/logout`, then 401):
`/workspaces` gives Wellrox (owner), chat conversations 12 (6 public, 1 private, 5 DMs), projects 2 (the third row is
deleted), members 7 (6 member, 1 owner). **Not** verified: a signed-in check as a `member` role account, a real guest
invitation and guest sign-in on production, and a Donna answer about what's new (the deployed file was checked).

Rollback (the old binary refuses schema 88: restore the snapshot together with the binary; work after 20:25 UTC is
lost, also any guest invitation made after the deploy). To go back to `6bfa8a7`:

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-2d72c1c backup verify 01a1c722-ed80-71dc-81f7-0cd456bf1feb && /usr/local/bin/orbit-2d72c1c backup restore 01a1c722-ed80-71dc-81f7-0cd456bf1feb'
sudo install -m 755 /usr/local/bin/orbit-6bfa8a7 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-2d72c1c /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-2d72c1c.tgz
```

After this rollback, set `PROD_TAG=6bfa8a7`, `ROLLBACK_TAG=e3ba2f8`,
`ROLLBACK_BACKUP=01a1c629-5ad6-7167-98d6-3431a1608760` in `/root/ops/vm-health.conf`.

### November 10, 2026 upgrade: step 1c, Connect remote MCP servers with OAuth (schema 87)

`main` fast-forwarded to `0fea68d2` from `423d9dc7` (branch `mcp-oauth-connect`): `9b041c7c` (Agent tools: credential
"Sign in with OAuth (Connect)" for remote MCP servers, e.g. TikTok Social; one shared sign-in per integration, tokens
sealed with the app key and renewed, "Connected as …", grant end date, owner/admin email 14 days before, Reconnect,
Disconnect; callback `GET /api/v1/agent-integrations/oauth/callback`; migration `0087_agent_integration_connect`, SQLite
and Postgres: 6 nullable columns on `agent_integrations` and the table `agent_integration_oauth_states`) and `0fea68d2`
(also try the path-appended RFC 8414 metadata URL), then `6bfa8a7e`: a line on top of the `2026-11-10` "what's new"
entry (OAuth Connect; two older lines of that entry merged to keep 4). `aboutOrbit()` was already updated on the branch
(Agent tools line and "How do I… let you use a service like TikTok"). No new `ORBIT_*` setting; sidecar
`package.json`/`package-lock.json` unchanged (only `src`). `ORBIT_PUBLIC_ORIGIN` is `https://orbit.wellrox.com` (the
OAuth redirect URI base); `https://business-api.tiktok.com/` answers HTTP 301 to the `orbit` user.

Checks before the switch (after `bun install --frozen-lockfile` and `npm ci` in `apps/agent`): web `tsc -b` and `bun test
src/features/agentIntegrations` (4 pass, 0 fail); agent `tsc` and `node --test` (60 pass, 0 fail, 11 skipped). Rust tests
were not run again on `main` (same commits as the branch). `orbit-deploy image` built `orbit:6bfa8a7` (31 min);
`deploy/smoke.sh` passed; `orbit config check` passed with the extracted binary.

At the switch: stopped `orbit-agent` and `orbit` at 15:52:41 UTC, created snapshot
`01a1c629-5ad6-7167-98d6-3431a1608760` with the old CLI (`/usr/local/bin/orbit-e3ba2f8`) and verified it, ran
`migrate run` of the new binary on a copy in `/var/lib/orbit/rehearsal-6bfa8a7-*` (1 applied, schema 87, quick-check ok,
0 foreign-key rows, 6 new `oauth_*` columns and the new table; deleted afterwards), saved the sidecar data as
`/var/backups/orbit-agent/orbit-agent-before-6bfa8a7.tgz`, kept the old sidecar code as
`/opt/orbit-agent.before-6bfa8a7`, installed the binary (`/usr/local/bin/orbit-6bfa8a7` too) and the new sidecar `src`,
and started `orbit` (15:53:14, ready), then `orbit-agent` (15:53:20). Pushed `main`. `vm-health`
(`/root/ops/vm-health.conf`, old copy `.before-6bfa8a7`): `PROD_TAG=6bfa8a7`, `ROLLBACK_TAG=e3ba2f8`,
`ROLLBACK_BACKUP=01a1c629-5ad6-7167-98d6-3431a1608760`.

Verified: `orbit`, `orbit-agent`, nginx, `orbit-oidc-gateway` and `orbit-oidc-tunnel` active, no restarts; readiness
(all checks) and `/health/live` 200; `/orbit-build.json` revision `6bfa8a7e`; schema 87; live quick-check and
foreign-key checks pass; web `/` 200; `/api/v1/agent-integrations/oauth/callback?state=x&code=y` answers 303 to
`/settings/agentic/workspace?agent_tool_connect=expired` (an unknown path under it is 404); sidecar `/health`
`{"ok":true,"mock":false}` and `/opt/orbit-agent/src` equal to `apps/agent/src` of `6bfa8a7e`; no log warnings except
rmcp's known "allowed origin without a port"; `vm-health` OK except the nginx lines of the switch window (58, all
15:52–15:53, upstream refused while stopped) and `.git` file owners. **Not** verified: a real TikTok Connect (needs an
admin's sign-in in the browser) and a Donna answer through it.

Rollback (the old binary refuses schema 87: restore the snapshot together with the binary; work after 15:52 UTC is lost,
also any OAuth connection made after the deploy). To go back to `e3ba2f8`:

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-6bfa8a7 backup verify 01a1c629-5ad6-7167-98d6-3431a1608760 && /usr/local/bin/orbit-6bfa8a7 backup restore 01a1c629-5ad6-7167-98d6-3431a1608760'
sudo install -m 755 /usr/local/bin/orbit-e3ba2f8 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-6bfa8a7 /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-6bfa8a7.tgz
```

After this rollback, set `PROD_TAG=e3ba2f8`, `ROLLBACK_TAG=8151a3c`,
`ROLLBACK_BACKUP=01a1c5a4-5127-7421-90cd-a66f0ccfe0cf` in `/root/ops/vm-health.conf`.

### November 10, 2026 upgrade: step 1b, phone chat, Drive click and desktop 0.1.3 (schema 86)

`main` fast-forwarded to `9fb6d301` from `9e690d92` (branch `mobile-chat-fix`): `c6db2c64` (phone chat: cards fit the
screen, Markdown tables wrap and scroll sideways, day chip with a fade, repeated tool lines grouped with a count; no
grey box behind the W in the phone top bar; desktop "Sign in to app" popup fix; desktop 0.1.3) and `9fb6d301` (Drive: a
click opens, only the checkbox or Ctrl/Shift selects), then `e3ba2f86`: lines added to the `2026-11-10` "what's new"
entry (Drive click; phone chat and desktop 0.1.3). `aboutOrbit()` was already updated on the branch. No migration, no
new `ORBIT_*` setting, sidecar `package.json`/`package-lock.json` unchanged (only `src` changed).

Checks before the switch (after `bun install --frozen-lockfile` at the root and `npm ci` in `apps/agent`: the main
checkout had no `@json-render` packages): web `tsc --noEmit` and `bun test src/features/drive src/features/chat
src/lib/markdown.test.tsx src/app/shell` (263 pass, 0 fail); mobile `tsc --noEmit` and `bun test src` (153 pass, 0
fail); agent `tsc` and `node --test` (60 pass, 0 fail, 11 skipped); desktop `cargo test --workspace` (61 pass, 1
ignored). `orbit-deploy image` built `orbit:e3ba2f8`; `deploy/smoke.sh` passed; `orbit config check` passed with the
extracted binary.

At the switch: stopped `orbit-agent` and `orbit` at 13:27:22 UTC, created snapshot
`01a1c5a4-5127-7421-90cd-a66f0ccfe0cf` with the old CLI (`/usr/local/bin/orbit-8151a3c`) and verified it, ran
`migrate run` of the new binary on a copy in `/var/lib/orbit/rehearsal-e3ba2f8-*` (0 applied, schema 86, quick-check
ok, 0 foreign-key rows; deleted afterwards), saved the sidecar data as
`/var/backups/orbit-agent/orbit-agent-before-e3ba2f8.tgz`, kept the old sidecar code as
`/opt/orbit-agent.before-e3ba2f8`, installed the binary (`/usr/local/bin/orbit-e3ba2f8` too) and the new sidecar `src`,
and started `orbit` (ready 13:27:57), then `orbit-agent`. Pushed `main` and tag `desktop-v0.1.3`. `vm-health`
(`/root/ops/vm-health.conf`, old copy `.before-e3ba2f8`): `PROD_TAG=e3ba2f8`, `ROLLBACK_TAG=8151a3c`,
`ROLLBACK_BACKUP=01a1c5a4-5127-7421-90cd-a66f0ccfe0cf`.

Verified: `orbit`, `orbit-agent`, nginx, `orbit-oidc-gateway` and `orbit-oidc-tunnel` active, no restarts; readiness
(all checks); `/orbit-build.json` revision `e3ba2f86`; schema 86; live quick-check and foreign-key checks pass; web `/`
and `/logo.svg` 200; sidecar `/health` `{"ok":true,"mock":false}` and `/opt/orbit-agent/src` equal to `apps/agent/src`
of `e3ba2f86`; no warnings in the logs; `vm-health` OK except the nginx lines of the switch window, the build-session
test instance and `.git` file owners. Browser on production at 390 px (temporary UUID v7 session row for
ddusi@wellrox.com, revoked by `POST /api/v1/auth/logout` (204); headless Chromium through `socat` on 127.0.0.1:8443):
Donna's DM with a Markdown table (the table scrolls sideways in its box, the page stays 390 px wide), the phone top bar
W with no box, Drive: a click on a folder opens it and a click on a file opens the preview, no selection. Console: the
known service worker 400 and one 404 (resource not identified). Screenshots:
`/root/.local/share/com.jean.desktop/pasted-images/orbit-1b-prod-*.png`.
Desktop downloads: tag `desktop-v0.1.3` (on `e3ba2f86`) started Desktop run
[42824960498](https://github.com/DaveDushi/orbit/actions/runs/42824960498) (test, windows and manifest passed, macos
skipped); `deploy/desktop-downloads.sh 42824960498` put `Orbit-0.1.3-windows-x64-setup.exe` (sha256 `e4da3e1c…`),
`Orbit-0.1.3-windows-x64.zip` (`9d5e96aa…`) and `latest.json` (version 0.1.3, Windows only) in
`/srv/orbit-downloads/desktop/` and removed the 0.1.2 files. Through the local nginx: `latest.json` 200
`application/json`, `no-cache`; both files download with the sha256 of `latest.json`. **Not** verified: the installer
and the "Sign in to app" popup on a real Windows PC.

Rollback (same schema 86: the old binary reads it, no restore needed). To go back to `8151a3c`:

```sh
sudo systemctl stop orbit-agent orbit
sudo install -m 755 /usr/local/bin/orbit-8151a3c /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-e3ba2f8 /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# Data back to 13:27 UTC, only if needed (work after it is lost), with both services stopped:
# sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-8151a3c backup verify 01a1c5a4-5127-7421-90cd-a66f0ccfe0cf && /usr/local/bin/orbit-8151a3c backup restore 01a1c5a4-5127-7421-90cd-a66f0ccfe0cf'
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-e3ba2f8.tgz
```

After this rollback, set `PROD_TAG=8151a3c`, `ROLLBACK_TAG=49954a2`,
`ROLLBACK_BACKUP=01a1c509-3a31-7732-8323-b52e2f8836cf` in `/root/ops/vm-health.conf`. The desktop 0.1.3 downloads can
stay (the server does not use them).

### November 10, 2026 upgrade: release step 1, upstream fixes and the phone settings menu (schema 86)

`main` fast-forwarded to `8151a3c7` from `50396704` (branch `badge-drive-tab`): `4dfc6657` (Drive replaces Mail in the
phone dock), `ec04712d` (desktop 0.1.2: unread number on the Windows taskbar overlay), nine coollabsio/orbit commits
cherry-picked with `-x` (`fc850b10` next free project key, `1ef45408` comment trim and no blank page on comment send
(without upstream's math sanitizer), `30766fcf` bulk changes in each task's activity, `5cbc496e` phone settings menu
(merged with our classic-UI sub-navigation, which a phone menu replaces), `c53ea121` GitHub closed issues and merged
PRs to Done (by hand, without upstream's trial-limit test), `ee9cecf5` ring spinner (our W loader in
`OrbitUnavailable.tsx` stays; mobile notes get `loading`), `fd5c3c1c` and `507ad808` board virtualization (no "Show
more"), `71e714ab` plain background), and `8151a3c7`: `ChangesAssociations=yes` in the Windows installer, two tests
that upstream left broken by its own changes, `aboutOrbit()` and the `2026-11-10` "what's new" entry. Not taken: the
import group, the mobile/Expo group, version bumps, `5821bc44` (guest role, a later step). No migration, no new
`ORBIT_*` setting, sidecar `package-lock.json` unchanged (only `src/prompt.ts` and `src/whats-new.md` changed).

Checks before the switch: web `tsc --noEmit` and `bun test src/features/tasks src/features/settings src/app/shell
src/components/ui` (512 pass; 2 fail in `TaskList.test.tsx`, the same 2 fail on `50396704`); mobile `tsc --noEmit` and
`bun test src` (152 pass; iOS bundle not built: no Metro server runs); agent `tsc` and `node --test` (59 pass, 0 fail,
11 skipped); server `tasks_api`, `integrations_api`, `chat_api` on SQLite and Postgres (141 pass each) and
`--lib sanitize` (15 pass). Browser e2e against a local seeded server (screenshots
`/root/.local/share/com.jean.desktop/pasted-images/orbit-step1-*.png`). `orbit-deploy image` built `orbit:8151a3c`
before the merge and reused it after the fast-forward; `deploy/smoke.sh` passed; `orbit config check` passed with the
extracted binary.

At the switch: stopped `orbit-agent` and `orbit` at 10:37:58 UTC, created snapshot
`01a1c509-3a31-7732-8323-b52e2f8836cf` (S3 bucket; the verified copy is in
`/var/backups/orbit/downloads/01a1c509-3a31-7732-8323-b52e2f8836cf`) with the old CLI (`/usr/local/bin/orbit-49954a2`)
and verified it, ran `migrate run` of the new binary on a copy in `/var/lib/orbit/rehearsal-8151a3c-*` (0 applied,
schema current, quick-check ok, 0 foreign-key rows; deleted afterwards), saved the sidecar data as
`/var/backups/orbit-agent/orbit-agent-before-8151a3c.tgz`, kept the old sidecar code as
`/opt/orbit-agent.before-8151a3c`, installed the binary (`/usr/local/bin/orbit-8151a3c` too) and the new sidecar `src`,
and started `orbit` (ready 10:38:40), then `orbit-agent`. `vm-health` (`/root/ops/vm-health.conf`, old copy
`.before-8151a3c`): `PROD_TAG=8151a3c`, `ROLLBACK_TAG=49954a2`, `ROLLBACK_BACKUP=01a1c509-3a31-7732-8323-b52e2f8836cf`.

Verified: `orbit`, `orbit-agent`, nginx, `orbit-oidc-gateway` and `orbit-oidc-tunnel` active, no restarts; readiness
(all checks); `/orbit-build.json` revision `8151a3c7`; schema 86; live quick-check and foreign-key checks pass; web `/`
and `/logo.svg` 200; sidecar `/health` `{"ok":true,"mock":false}` and `/opt/orbit-agent/src` equal to `apps/agent/src`
of `8151a3c7`. Logs: only the rmcp "allowed origin without a port" warning and Node's SQLite ExperimentalWarning, both
also with `49954a2`. **Not** verified: a signed-in browser check on production.
Desktop downloads: tag `desktop-v0.1.2` (on `22920080`) started Desktop run
[42801038457](https://github.com/DaveDushi/orbit/actions/runs/42801038457) (test, windows and manifest passed, macos
skipped); `deploy/desktop-downloads.sh 42801038457` put `Orbit-0.1.2-windows-x64-setup.exe` (sha256 `06facd7c…`),
`Orbit-0.1.2-windows-x64.zip` (`55491790…`) and `latest.json` (version 0.1.2, Windows only) in
`/srv/orbit-downloads/desktop/` and removed the 0.1.1 files. Through the local nginx: `latest.json` 200
`application/json`, `no-cache`; both files download with the sha256 of `latest.json`. The public URLs answer 307 to the
exe.dev login without an exe.dev session. **Not** verified: the installer on a real Windows PC.

Rollback (same schema 86: the old binary reads it, no restore needed). To go back to `49954a2`:

```sh
sudo systemctl stop orbit-agent orbit
sudo install -m 755 /usr/local/bin/orbit-49954a2 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-8151a3c /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# Data back to 10:37 UTC, only if needed (work after it is lost), with both services stopped:
# sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-49954a2 backup verify 01a1c509-3a31-7732-8323-b52e2f8836cf && /usr/local/bin/orbit-49954a2 backup restore 01a1c509-3a31-7732-8323-b52e2f8836cf'
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-8151a3c.tgz
```

After this rollback, set `PROD_TAG=49954a2`, `ROLLBACK_TAG=f668385`,
`ROLLBACK_BACKUP=01a1c171-6545-7502-b5ce-5a5090ab8bdd` in `/root/ops/vm-health.conf`.

### November 9, 2026 upgrade: the "W" logo and the Windows desktop app downloads (schema 86)

`main` fast-forwarded to `49954a2f` from `3a7ce95f`: the `desktop-rune` branch (the desktop app: the web UI in a
native window with tray, notifications, badge, Windows installer; parts 1-4 and `70bf91f9`, `c8753f1a`) and
`49954a2f`: the logo is a pixel "W" (Wellrox fork) from one source, `apps/web/public/logo.svg`;
`scripts/logo-icons.sh` makes the favicons, PWA/Apple icons, the desktop tray PNG and `orbit.ico`. The Desktop workflow
builds macOS only on request, and `latest.json` needs only the Windows job; desktop 0.1.1;
`deploy/desktop-downloads.sh`; "Download desktop app" in the account menu (Windows browsers, not inside the app).
`aboutOrbit()` names the W logo and the download link; the 4th line of the `2026-11-09` "what's new" entry names them.
No migration, no new `ORBIT_*` setting, sidecar `package-lock.json` unchanged (only `src/prompt.ts` and
`src/whats-new.md` changed; the new `src` was copied over a copy of the old code).

Checks before the switch: desktop `cargo test --workspace` (57 pass, with
`the_windows_only_file_of_the_desktop_workflow_reads`); web `tsc --noEmit`, `UserMenu.test.tsx` and `favicon.test.ts`
(4 pass); mobile `tsc --noEmit` and `bun test src` (151 pass; iOS bundle not built: no Metro server runs); agent
`tsc` and `node --test` (59 pass, 0 fail, 11 skipped). `orbit-deploy image` built `orbit:49954a2` before the merge and
reused it after the fast-forward; `deploy/smoke.sh` passed; `orbit config check` passed with the extracted binary.

At the switch: stopped `orbit-agent` and `orbit` at 17:53:16 UTC, created snapshot
`01a1c171-6545-7502-b5ce-5a5090ab8bdd` with the old CLI (`/usr/local/bin/orbit-f668385`) and verified it, ran
`migrate run` of the new binary on a copy in `/var/lib/orbit/rehearsal-49954a2-*` (0 applied, quick-check ok, 0
foreign-key rows; deleted afterwards), saved the sidecar data as
`/var/backups/orbit-agent/orbit-agent-before-49954a2.tgz`, kept the old sidecar code as
`/opt/orbit-agent.before-49954a2`, installed the binary (`/usr/local/bin/orbit-49954a2` too) and the new sidecar
`src`, added the nginx `/downloads/desktop/` location (`nginx -t` ok, reload), and started `orbit` (ready 17:53:42),
then `orbit-agent`. Pushed `main`. `vm-health` (`/root/ops/vm-health.conf`): `PROD_TAG=49954a2`,
`ROLLBACK_TAG=f668385`, `ROLLBACK_BACKUP=01a1c171-6545-7502-b5ce-5a5090ab8bdd`.

Verified: `orbit`, `orbit-agent`, nginx, `orbit-oidc-gateway` and `orbit-oidc-tunnel` active, no restarts; readiness
(all checks); `/orbit-build.json` revision `49954a2f`; live quick-check and foreign-key checks pass; web `/` 200;
`/logo.svg` and `/favicon.svg` are the W; sidecar `/health` `{"ok":true,"mock":false}` and `/opt/orbit-agent/src` equal
to `apps/agent/src` of `49954a2f`; no warnings in the logs. Browser on production (temporary UUID v7 session row for
ddusi@wellrox.com, revoked by `POST /api/v1/auth/logout` (204); headless Chromium through `socat` on 127.0.0.1:8443):
the phone top bar shows the W, the account menu (Windows user agent) shows "Download desktop app", which opens
`/downloads/desktop/`. Only console error: the known service worker 400. The sign-in page has no logo (none since the
Coolify-style restyle `2fef22c3`). Screenshots in `/root/.local/share/com.jean.desktop/pasted-images/`:
`desktop-w-icons.png`, `desktop-w-icons-actual-size.png`, `web-w-signin.png`, `web-w-phone-topbar.png`,
`web-w-phone-topbar-zoom.png`, `web-w-phone-sidebar.png`, `web-w-account-menu.png`, `web-w-downloads-page.png`.
Desktop downloads: tag `desktop-v0.1.1` started Desktop run
[42669076371](https://github.com/DaveDushi/orbit/actions/runs/42669076371) (test, windows and manifest passed, macos
skipped); `deploy/desktop-downloads.sh 42669076371` put `Orbit-0.1.1-windows-x64-setup.exe` (sha256 `6a39bfd9…`),
`Orbit-0.1.1-windows-x64.zip` (`57ec2ae5…`) and `latest.json` (version 0.1.1, Windows only) in
`/srv/orbit-downloads/desktop/`. Through the local nginx: `latest.json` 200 `application/json`, `no-cache`; both files
download with the sha256 of `latest.json`. The public URL answers 307 to the exe.dev login without an exe.dev session,
like every Orbit page. **Not** verified: a download through the public URL with an exe.dev session, and the installer on
a real Windows PC (the workflow installed and uninstalled it on its Windows runner).

Rollback (same schema 86: the old binary reads it, no restore needed). To go back to `f668385`:

```sh
sudo systemctl stop orbit-agent orbit
sudo install -m 755 /usr/local/bin/orbit-f668385 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-49954a2 /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# Data back to 17:53 UTC, only if needed (work after it is lost), with both services stopped:
# sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-f668385 backup verify 01a1c171-6545-7502-b5ce-5a5090ab8bdd && /usr/local/bin/orbit-f668385 backup restore 01a1c171-6545-7502-b5ce-5a5090ab8bdd'
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-49954a2.tgz
```

After this rollback, set `PROD_TAG=f668385`, `ROLLBACK_TAG=4eb9a16`,
`ROLLBACK_BACKUP=01a1c082-e2b9-7515-a956-d7c5cc81b6bf` in `/root/ops/vm-health.conf`. The nginx downloads location
can stay (the old binary does not use `/downloads/`).

### November 9, 2026 upgrade: Donna's media sandbox, the version and what is new, ecommerce app icons (schema 86)

`main` fast-forwarded to `f6683850` from `accd0e4d`. The `donna-media` branch had `a6b7e3f8` and `917aa551` (Donna
looks at and edits images, videos and scanned PDFs from Drive in a sandbox, `deploy/donna-sandbox/`), and merged
`donna-knowledge` (`f34bf62c`, Donna knows the deployed version and what is new; merge `21671c50`, conflict in
`apps/agent/src/tools.ts`: both the `orbit-about` section with the sandbox flag and the new `orbit-version` section
kept) and `app-icons` (`7caba27b`, 16 ecommerce icons in the app icon picker; merge `0bcdc06e`). `f6683850` adds the
icons to `aboutOrbit()` and the "what's new" entry: the `2026-11-09` entry of `apps/agent/src/whats-new.md` now has 4
lines (media sandbox; time, date and version; cards; ecommerce icons and the phone top bar). No migration, no new
`ORBIT_*` setting, `package-lock.json` of the sidecar unchanged.

Checks before the switch: agent `tsc` and `node --test` (59 pass, 0 fail, 11 skipped); `cargo test -p orbit-server
--lib apps::` (6 pass, with `the_icons_equal_the_web_list_and_include_the_default`); web `tsc --noEmit`.
`orbit-deploy image` built `orbit:f668385` before the merge (label = the full commit) and reused it after the
fast-forward; `deploy/smoke.sh` passed; `orbit config check` passed with the extracted binary.

At the switch: stopped `orbit-agent` and `orbit` at 13:32:45 UTC, created snapshot
`01a1c082-e2b9-7515-a956-d7c5cc81b6bf` with the old CLI (`/usr/local/bin/orbit-4eb9a16`) and verified it, ran
`migrate run` of the new binary on a copy in `/var/lib/orbit/rehearsal-f668385-*` (0 applied, schema 86, quick-check
ok, 0 foreign-key rows, counts unchanged: 6 users, 1 workspace, 3 projects, 65 tasks, 47 pages, 649 chat messages,
128 agent inputs; deleted afterwards), saved the sidecar data as
`/var/backups/orbit-agent/orbit-agent-before-f668385.tgz`, kept the old sidecar code as
`/opt/orbit-agent.before-f668385`, installed the binary and the new sidecar code, and started `orbit` (ready
13:33:11), then `orbit-agent`. The old executable is `/usr/local/bin/orbit-4eb9a16`; the new one is also kept as
`/usr/local/bin/orbit-f668385`. Pushed `main`. Then `sudo deploy/donna-sandbox/setup.sh` (installed bubblewrap,
ffmpeg, imagemagick, ghostscript, poppler-utils and libimage-exiftool-perl; needrestart deferred the restarts of
docker, jean-server, dbus and systemd-logind, not done) and `systemctl restart orbit-agent` (13:33:41).
`vm-health` (`/root/ops/vm-health.conf`): `PROD_TAG=f668385`, `ROLLBACK_TAG=4eb9a16`,
`ROLLBACK_BACKUP=01a1c082-e2b9-7515-a956-d7c5cc81b6bf`.

Verified: `orbit`, `orbit-agent`, nginx, `orbit-oidc-gateway` and `orbit-oidc-tunnel` active, no restarts; readiness
(all checks); `/orbit-build.json` revision `f6683850`; schema 86; live quick-check and foreign-key checks pass; web `/`
200; sidecar `/health` `{"ok":true,"mock":false}` and `/opt/orbit-agent/src` equal to `apps/agent/src` of
`f6683850`; `/run/orbit-donna/sandbox.sock` (root:orbit-agent, 0660), `orbit-donna.socket` active, `orbit-agent` has
`AGENT_SANDBOX_SOCKET`; no log errors (only the known rmcp origin warning and the node SQLite warning).
**Real-model check on production:** the same way as the 4eb9a16 check (a temporary UUID v7 session row for
ddusi@wellrox.com, 2 h expiry, revoked by `POST /api/v1/auth/logout` (204) after the check; headless Chromium through
`socat` on 127.0.0.1:8443). In the Donna DM:
- "look at the Drive image wellrox-pushup-challenge-story-1.png …" (message `01a1c084-d5ae-740a-92ff-d20710555e79`):
  "A blush-pink Wellrox push-up challenge giveaway graphic with the logo at the top. It advertises a full 4-band
  premium fabric set for one random winner, with entry closing August 23 at 12 PM ET. The bottom displays
  challenge.wellrox.com, plus eligibility and entry details." with the status "Looking at 1 image(s)"; the sandbox made
  one `/work` image in `/var/lib/orbit-donna/work`.
- "which Orbit version runs here, and what is new?" (message `01a1c085-34f7-7149-a278-570ea8dc8a69`): "Orbit runs
  f668385, deployed November 9, 2026", then the 4 lines of the new entry.

Only console error: the known service worker 400. Screenshots:
`/root/.local/share/com.jean.desktop/pasted-images/donna-media-prod-image.png` and `donna-media-prod-version.png`.
**Not** verified: a video or scanned PDF on production, a media edit saved to Drive, the icon picker in the browser,
and the phone app.

Rollback (same schema 86: the old binary reads it, no restore needed). To go back to `4eb9a16`:

```sh
sudo systemctl stop orbit-agent orbit
sudo install -m 755 /usr/local/bin/orbit-4eb9a16 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-f668385 /opt/orbit-agent
sudo deploy/donna-sandbox/setup.sh --uninstall   # from a checkout that has it; removes the sandbox and its /work data
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# Data back to 13:32 UTC, only if needed (work after it is lost), with both services stopped:
# sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-4eb9a16 backup verify 01a1c082-e2b9-7515-a956-d7c5cc81b6bf && /usr/local/bin/orbit-4eb9a16 backup restore 01a1c082-e2b9-7515-a956-d7c5cc81b6bf'
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-f668385.tgz
```

After this rollback, set `PROD_TAG=4eb9a16`, `ROLLBACK_TAG=b1ad159`,
`ROLLBACK_BACKUP=01a1bfde-6379-76a0-921b-f55b526c059a` in `/root/ops/vm-health.conf`. To turn off only the sandbox
and keep f668385: `sudo deploy/donna-sandbox/setup.sh --uninstall && sudo systemctl restart orbit-agent` (Donna then
has no media tools).

### November 9, 2026 upgrade: the phone top bar stays visible on the chat list (schema 86, web only)

`main` merged `chat-topbar` (`13b5ba4e`, based on `39f7fdf0`) with merge commit `4eb9a16e` (`main` had `b1ad1597` and
the docs commit `19812b2f`). Web only: `ChatPage.tsx` pulls the page up into the top safe area only on a conversation
path (`isChatConversationPath`), and `ChatSidebar.tsx` drops its own top inset padding. No migration, no new
`ORBIT_*` setting, no change in `apps/agent` (`/opt/orbit-agent/src` equal to `apps/agent/src`, so the sidecar code
was not replaced), `aboutOrbit()` unchanged (bug fix).

Checks before the switch: `orbit-deploy image` built `orbit:4eb9a16` in the background (label = the full merge
commit); `deploy/smoke.sh` passed; `orbit config check` passed with the extracted binary.

At the switch: stopped `orbit-agent` and `orbit` at 10:33:05 UTC, created snapshot
`01a1bfde-6379-76a0-921b-f55b526c059a` with the old CLI (`/usr/local/bin/orbit-b1ad159`) and verified it, ran
`migrate run` of the new binary on a copy in `/var/lib/orbit/rehearsal-4eb9a16-*` (0 applied, schema 86, quick-check
ok, 0 foreign-key rows, counts unchanged: 6 users, 1 workspace, 3 projects, 65 tasks, 47 pages, 637 chat messages,
122 agent inputs; deleted afterwards), saved the sidecar data as
`/var/backups/orbit-agent/orbit-agent-before-4eb9a16.tgz`, installed the binary, and started `orbit` (ready 10:33:27),
then `orbit-agent`. The old executable is `/usr/local/bin/orbit-b1ad159`; the new one is also kept as
`/usr/local/bin/orbit-4eb9a16`. `vm-health` (`/root/ops/vm-health.conf`): `PROD_TAG=4eb9a16`,
`ROLLBACK_TAG=b1ad159`, `ROLLBACK_BACKUP=01a1bfde-6379-76a0-921b-f55b526c059a`. Pushed `main`.

Verified: `orbit`, `orbit-agent`, nginx, `orbit-oidc-gateway` and `orbit-oidc-tunnel` active, no restarts; readiness
(all checks); `/orbit-build.json` revision `4eb9a16`; schema 86; live quick-check and foreign-key checks pass; web `/`
200; sidecar `/health` `{"ok":true,"mock":false}`; no log errors (only the known rmcp origin warning and the node
SQLite warning). **Phone web check on production:** no saved login, so a temporary UUID v7 session row for
ddusi@wellrox.com (2 h expiry, inserted live with `busy_timeout`), revoked by `POST /api/v1/auth/logout` (204,
`revoked_at` set) after the check; headless Chromium 390x844 through `socat` on 127.0.0.1:8443 with
`Emulation.setSafeAreaInsetsOverride` top 47 px. `/chat`: the top bar (Wellrox, bell, avatar) is at 47-108 px, the
chat page starts at 108 px. `/chat/unreads`: no top bar, the chat page fills 0-844 px. Only console error: the known
service worker 400. Screenshots: `/root/.local/share/com.jean.desktop/pasted-images/chat-topbar-prod.png` and
`chat-topbar-prod-conversation.png`. **Not** verified: the phone app.

Rollback (same schema 86: the old binary reads it, no restore needed). To go back to `b1ad159`:

```sh
sudo systemctl stop orbit-agent orbit
sudo install -m 755 /usr/local/bin/orbit-b1ad159 /usr/local/bin/orbit
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# Data back to 10:33 UTC, only if needed (work after it is lost), with both services stopped:
# sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-b1ad159 backup verify 01a1bfde-6379-76a0-921b-f55b526c059a && /usr/local/bin/orbit-b1ad159 backup restore 01a1bfde-6379-76a0-921b-f55b526c059a'
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-4eb9a16.tgz
```

Then set `PROD_TAG=b1ad159`, `ROLLBACK_TAG=9f790ab`, `ROLLBACK_BACKUP=01a1bfc0-de4a-7367-95f9-ba6f131f295d` in
`/root/ops/vm-health.conf`.

### November 9, 2026 upgrade: Donna knows the sender's current time (schema 86, no migration)

`main` merged `donna-time` (`3a36a9c7`, based on `9f790ab3`) with merge commit `b1ad1597` (`main` had the docs commit
`39f7fdf0`). `frame()` starts each new input with the sender's local time (`[Mon 2026-11-09 12:01, Asia/Jerusalem]`,
from `users.timezone`; UTC if none); the internal people endpoint (`apps/server/src/agent/routes.rs`) now returns
`timezone`. The system prompt has no time in it, only the fixed `TIME_RULE` (`apps/agent/src/prompt.ts`), so its cache
stays. No migration, no new `ORBIT_*` setting, `package-lock.json` unchanged (the new sidecar's `npm ci --omit=dev`
gave the same `node_modules/.package-lock.json`).

Checks before the switch: agent `tsc` and `node --test` (49 pass, with "the system prompt is the same at any time":
the prompt at 2026-11-09 08:03Z and 2027-03-01 23:30Z is identical); `agent_api` (14) on SQLite and Postgres.
`orbit-deploy image` built `orbit:b1ad159` (label = the full merge commit; the first try died with the 10-minute tool
timeout, the background run finished); `deploy/smoke.sh` passed; `orbit config check` passed with the extracted binary.

At the switch: stopped `orbit-agent` and `orbit` at 10:00:50 UTC, created snapshot
`01a1bfc0-de4a-7367-95f9-ba6f131f295d` with the old CLI and verified it, ran `migrate run` of the new binary on a copy
in `/var/lib/orbit/rehearsal-b1ad159-switch-*` (0 applied, schema 86, quick-check ok, 0 foreign-key rows, counts
unchanged: 6 users, 1 workspace, 3 projects, 65 tasks, 47 pages, 635 chat messages, 121 agent inputs; deleted
afterwards), saved the sidecar data as `/var/backups/orbit-agent/orbit-agent-before-b1ad159.tgz`, kept the old sidecar
code as `/opt/orbit-agent.before-b1ad159`, installed the binary and the new sidecar code, and started `orbit` (ready
10:01:18), then `orbit-agent`. The old executable is `/usr/local/bin/orbit-9f790ab`; the new one is also kept as
`/usr/local/bin/orbit-b1ad159`. `vm-health` (`/root/ops/vm-health.conf`): `PROD_TAG=b1ad159`, `ROLLBACK_TAG=9f790ab`,
`ROLLBACK_BACKUP=01a1bfc0-de4a-7367-95f9-ba6f131f295d`. Pushed `main`.

Verified: `orbit`, `orbit-agent`, nginx, `orbit-oidc-gateway` and `orbit-oidc-tunnel` active, no restarts; readiness
(all checks); `/orbit-build.json` revision `b1ad159`; schema 86; live quick-check and foreign-key checks pass; web `/`
200; sidecar `/health` `{"ok":true,"mock":false}` and `/opt/orbit-agent/src` equal to `apps/agent/src` of `b1ad1597`;
no log errors. **Real-model date on production:** the same way as the 9f790ab check (a temporary UUID v7 session row
for ddusi@wellrox.com inserted while Orbit was stopped, 2 h expiry, revoked by `POST /api/v1/auth/logout` (204) after
the check; headless Chromium through `socat` on 127.0.0.1:8443). In the Donna DM (question message
`01a1bfc1-d7b7-762e-8f17-220235d6bed9`, 10:01 UTC = 12:01 in Jerusalem) "what is today's date and weekday, and what
date is next Friday?" gave "Today is Monday, November 9, 2026; next Friday is November 13, 2026 (Asia/Jerusalem)."
(correct). Only console error: the known service worker 400. Screenshot:
`/root/.local/share/com.jean.desktop/pasted-images/donna-time-prod.png`. **Not** verified: the phone app.

Rollback (same schema 86: the old binary reads it, no restore needed). To go back to `9f790ab`:

```sh
sudo systemctl stop orbit-agent orbit
sudo install -m 755 /usr/local/bin/orbit-9f790ab /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-b1ad159 /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# Data back to 10:00 UTC, only if needed (work after it is lost), with both services stopped:
# sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-9f790ab backup verify 01a1bfc0-de4a-7367-95f9-ba6f131f295d && /usr/local/bin/orbit-9f790ab backup restore 01a1bfc0-de4a-7367-95f9-ba6f131f295d'
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-b1ad159.tgz
```

Then set `PROD_TAG=9f790ab`, `ROLLBACK_TAG=0e1cbcc`, `ROLLBACK_BACKUP=01a1bf84-ed4d-762a-a81e-1338a01f24c6` in
`/root/ops/vm-health.conf`.

### November 9, 2026 upgrade: Donna's interactive cards (schema 86)

`main` fast-forwarded from `e202fd80` (production `0e1cbcc9`) to `9f790ab3` (`a8f75065`: Donna answers with
interactive cards, `show_ui` and json-render; `9f790ab3`: live cards through SpecStream, card clicks without a chat
message, native phone cards). Migrations `0085_agent_card_actions` (SQLite rebuilds `agent_inputs` for kind `action`)
and `0086_agent_hidden_actions` (SQLite rebuilds `agent_inputs` again without the `chat_messages` foreign key; Postgres
drops `agent_inputs_message_id_fkey`). No new `ORBIT_*` setting. The sidecar `package-lock.json` changed: the new
sidecar has its own `npm ci --omit=dev` (55 modules, new `@json-render` and `zod`; this VM has no `npm` next to
`/opt/orbit-node`, so it ran `/root/.local/node/lib/node_modules/npm/bin/npm-cli.js` with `/opt/orbit-node/bin/node`).
`aboutOrbit()` in `apps/agent/src/prompt.ts` names the live cards, the clicks without a chat message, "Sent" and the
phone cards.

Checks before the switch: agent `tsc` and `node --test` (47 pass); SQLite and Postgres: `agent_api` (14), `app_smoke`
(21), `openapi_contract` (19), orbit-platform `database` (23) and `backup` (49). (`app_smoke` first failed with
`StaticAssets(RevisionMismatch)` because the worktree's `apps/web/dist` was an old build; a fresh web build fixed it,
the image builds its own.) `orbit-deploy image` built `orbit:9f790ab` before the merge (label = the full commit) and
reused it after the fast-forward; `deploy/smoke.sh` passed twice; `orbit config check` passed with the extracted
binary. An early migration rehearsal on a hot copy gave the same result as the one below.

At the switch: stopped `orbit-agent` and `orbit` at 08:55:22 UTC, created snapshot
`01a1bf84-ed4d-762a-a81e-1338a01f24c6` with the old CLI and verified it, ran `migrate run` of the new binary on a copy
in `/var/lib/orbit/rehearsal-9f790ab-switch-*` (2 applied, schema 86, quick-check ok, 0 foreign-key rows, counts
unchanged: 6 users, 1 workspace, 3 projects, 65 tasks, 47 pages, 622 chat messages, 0 API tokens, 112 agent inputs
(0 pending), 537 Drive nodes; deleted afterwards), saved the sidecar data as
`/var/backups/orbit-agent/orbit-agent-before-9f790ab.tgz`, kept the old sidecar code as
`/opt/orbit-agent.before-9f790ab`, installed the binary and the new sidecar code, and started `orbit` (ready
08:55:28), then `orbit-agent`. The old executable is `/usr/local/bin/orbit-0e1cbcc`; the new one is also kept as
`/usr/local/bin/orbit-9f790ab`. `vm-health` (`/root/ops/vm-health.conf`): `PROD_TAG=9f790ab`, `ROLLBACK_TAG=0e1cbcc`,
`ROLLBACK_BACKUP=01a1bf84-ed4d-762a-a81e-1338a01f24c6`. Pushed `main`. Stopped the gen-UI test servers on
127.0.0.1:18741/18742 and 18761/18762 (`/tmp/orbit-genui-e2e`, `/tmp/orbit-genui-v2` and their sidecars).

Verified: `orbit`, `orbit-agent`, nginx, `orbit-oidc-gateway` and `orbit-oidc-tunnel` active, no restarts; readiness
(all checks); `/orbit-build.json` revision `9f790ab`; schema 86; live quick-check and foreign-key checks pass; web `/`
200; sidecar `/health` `{"ok":true,"mock":false}` and `/opt/orbit-agent/src` equal to `apps/agent/src` of `9f790ab3`;
no log errors (only rmcp's known "allowed origin without a port"; nginx "connection refused" only during the 6 s
restart). **Real-model card on production:** a temporary session row for ddusi@wellrox.com (inserted while Orbit was
stopped, 2 h expiry, revoked by `POST /api/v1/auth/logout` after the check; a session id must be a UUID v7), headless
Chromium through a local TLS hop (`socat` on 127.0.0.1:8443 to nginx, `X-Forwarded-Proto: https` added by the
browser context). In the Donna DM (question message `01a1bf86-6aa9-753a-866b-264bcc542729`, 08:56:59) a short question gave a card "Production card check"
that built live (`aria-busy`), with buttons "Works" and "Broken"; the click on "Works" showed "Sent", made one
`agent_inputs` row of kind `action` with its own id (acked) and **no** chat message of the user, and Donna answered
"Confirmed—cards and button replies work on production." "Sent" stays after a reload. No page errors (the one console
400 is the service worker script, which does not get the test's added header; `/sw.js` is 200 through HTTPS).
Screenshots: `/root/.local/share/com.jean.desktop/pasted-images/donna-gen-ui-prod-{1-card,2-sent,3-reply}.png`.
**Not** verified: the phone app against production.

Rollback (the old binary cannot read schema 86: restore the snapshot together with the binary; work after 08:55 UTC
is lost, also chat messages). To go back to `0e1cbcc`:

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-0e1cbcc backup verify 01a1bf84-ed4d-762a-a81e-1338a01f24c6 && /usr/local/bin/orbit-0e1cbcc backup restore 01a1bf84-ed4d-762a-a81e-1338a01f24c6'
sudo install -m 755 /usr/local/bin/orbit-0e1cbcc /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-9f790ab /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-9f790ab.tgz
```

Then set `PROD_TAG=0e1cbcc`, `ROLLBACK_TAG=025fffe`, `ROLLBACK_BACKUP=01a1b779-fdac-7724-b377-016a1385d2e4` in
`/root/ops/vm-health.conf`.

### November 7, 2026 upgrade: wellrox-cloud import fix, Donna's Drive tools (schema 84), and the wellrox-cloud import

`main` from `015d4de7` (production `025fffe9`) to the merge commit `0e1cbcc9`: `import-guard-fix` (`e7a04eb6`: the
import takes the Worker keys `blobs/<22-character id>`; the attachment cleanup and mover list only
`blobs/{workspace_id}/…`, before they could delete those objects; the import no longer logs the dump) and `donna-drive`
(`aff5c091`, `5b14027d`, `f6a6f2bd`: Donna's Drive tools, trash and public links only after the driver's
server-checked yes). Its migration `0076_agent_drive_approvals` was renumbered to `0084` (SQLite and Postgres). No new
`ORBIT_*` setting. The sidecar `package-lock.json` did not change, so the new sidecar reuses the old `node_modules`.

Checks before the switch: agent `tsc` and `node --test` (41 pass); SQLite and Postgres: `drive_import` (4), `agent_drive`
(3), `agent_integrations` (17), `app_smoke` (20; `embedded_web_build_is_sent_precompressed…` fails locally because the
working tree's `apps/web/dist` has no `.br` files, not in the image), orbit-platform `object_storage`, `database`, `backup`,
and the `drive_import` unit tests; `clippy -p orbit-server --all-targets -D warnings`. `orbit-deploy image` built
`orbit:0e1cbcc` (image `sha256:2e90ceec366db1ef25e9cd1f41bc5a395d5fe960bf335ef7d111dd0e32d705fe`, label = the full
commit), `deploy/smoke.sh` passed, and `orbit config check` passed with the extracted binary.

At the switch: stopped `orbit-agent` and `orbit` at 19:26:27 UTC, created snapshot `01a1b779-fdac-7724-b377-016a1385d2e4`
with the old CLI and verified it, ran `migrate run` of the new binary on a copy in `/var/lib/orbit/rehearsal-0e1cbcc-*`
(1 applied, schema 84, quick-check and foreign-key checks pass, counts unchanged; deleted afterwards), saved the sidecar
data as `/var/backups/orbit-agent/orbit-agent-before-0e1cbcc.tgz`, kept the old sidecar code as
`/opt/orbit-agent.before-0e1cbcc`, installed the binary and the new sidecar code, and started `orbit` (ready 19:26:53),
then `orbit-agent`. The old executable is `/usr/local/bin/orbit-025fffe`; the new one is also kept as
`/usr/local/bin/orbit-0e1cbcc`. `vm-health`: `PROD_TAG=0e1cbcc`, `ROLLBACK_TAG=025fffe`. Pushed `main`.

The wellrox-cloud import (workspace `01a13079-d065-729b-917b-e30d8d9aa51c`, Drive bucket `wellrox-files`, no prefix):
stopped both services at 19:27:14, created and verified snapshot `01a1b77a-b5b8-7087-8de4-b1805e8941bb`, gave `orbit` a
private copy of the dump. Dry run (stderr empty, so no dump text; deleted): 60 folders below `/wellrox-cloud/` (the
Worker's `root` becomes `/wellrox-cloud/` itself: 61 folder nodes in the dump), 473 files, 8,833,106,462 bytes, 0 in the
trash, 197 left out (purged in the Worker), 0 missing objects, 0 refused keys, 0 name conflicts, 0 size differences.
The real run (19:28:18–19:28:47) gave the same report and made its own snapshot `01a1b77b-af48-73a5-9de3-6c2b07f0ebb7`.
Started `orbit` and `orbit-agent` at 19:29:05. The dump and its copy are deleted.

Verified: `orbit`, `orbit-agent`, nginx, `orbit-oidc-gateway` and `orbit-oidc-tunnel` active, no restarts; readiness (all
checks); `/orbit-build.json` revision `0e1cbcc`; schema 84; live quick-check and foreign-key checks pass; counts unchanged
except Drive (6 users, 1 workspace, 3 projects, 63 tasks, 43 pages, 596 chat messages, 0 API tokens; Drive nodes 3 → 537:
534 `wellrox_cloud` nodes, 61 folders and 473 files under `/wellrox-cloud/`, 8,833,106,462 bytes, 0 trashed; 473 Drive
blobs, 462 of them `blobs/<id>`); sidecar `/health` `{"ok":true,"mock":false}` and `/opt/orbit-agent/src` equal to
`apps/agent/src` of `0e1cbcc9`; no log warnings after the import except rmcp's known "allowed origin without a port".
On a copy of the imported database served by the new binary in a network namespace: `/drive` lists `wellrox-cloud`, the
folder opens (breadcrumb, `Rachel Uploads`), no page errors, and the download API gave `wellrox-files` URLs for 4 files;
outside the namespace a 16-byte ranged GET of each returned 206 with the right total size and file signature (MP4, MOV
under `Rachel Uploads/`, JPEG, PNG). The copy server stops itself after ~15 s without network (attachment reconcile
cannot reach S3). **Not** verified: a Donna Drive answer (needs a person's mention on production), any signed-in page
on production itself. Nothing in the R2 bucket was deleted or changed.

Rollback (the old binary cannot read schema 84: restore a snapshot together with the binary; work after the snapshot is
lost). To undo only the import, restore `01a1b77a-b5b8-7087-8de4-b1805e8941bb` with the new binary (schema 84, before
the import). A restore changes no R2 object. `025fffe` lists all of `blobs/` only in the installation bucket
(`wellrox-orbit-backups`), so it does not touch the Worker's `blobs/<id>` in the workspace bucket `wellrox-files`; never
make `wellrox-files` the installation bucket with it. To go back to `025fffe` fully:

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-025fffe backup verify 01a1b779-fdac-7724-b377-016a1385d2e4 && /usr/local/bin/orbit-025fffe backup restore 01a1b779-fdac-7724-b377-016a1385d2e4'
sudo install -m 755 /usr/local/bin/orbit-025fffe /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-0e1cbcc /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-0e1cbcc.tgz
```

### November 6, 2026 upgrade: upstream port, Drive sidebar and Donna's knowledge (schema 83)

`main` from `35780dc` to the merge commit `025fffe9` (`drive-sidebar`). Shipped: the upstream port (API tokens with one
`scopes` column, all-projects tokens and MCP grants, a public REST API for tokens, chat incoming webhooks with Block Kit
and buttons, MCP `search_messages` and sub-issue tools, OAuth consent access per resource, Agentic moved to workspace
settings, 413 for big bodies and other fixes), the Drive folder tree in the app sidebar, and Donna's knowledge of this
release (`apps/agent/src/{agent,prompt,tools}.ts`). Migrations 0076–0083. No new `ORBIT_*` setting. The sidecar
`package-lock.json` did not change, so the new sidecar copy reuses the `node_modules` of the old one (no npm on this VM).

Checks before the switch: web `tsc` and 62 Drive/shell tests on the merged head. `orbit-deploy image` built
`orbit:025fffe` (image `sha256:d55b6176c56083450d0cea32f24c6019f9a347ec67f146ea8cff3c6dd00a2ac0`, label
`org.orbit.clean-revision` = the full commit, orbit-server compile 26 min; a first attempt was canceled with its
session), `deploy/smoke.sh` passed, and `orbit config check` passed with the extracted binary.

At the switch: stopped `orbit-agent` and `orbit` at 14:36:39 UTC, created snapshot `01a1b14a-509c-741d-9ba9-30cabd4eb5d1`
with the old CLI and verified it, ran `migrate run` of the new binary on a copy in `/var/lib/orbit/rehearsal-025fffe-*`
(8 applied, schema 83, quick-check and foreign-key checks pass, counts unchanged, `api_tokens.scopes` present, its own
pre-migration backup made; deleted afterwards), saved the sidecar data as
`/var/backups/orbit-agent/orbit-agent-before-025fffe.tgz`, kept the old sidecar code as `/opt/orbit-agent.before-025fffe`,
installed the binary and the new sidecar code, and started `orbit` (ready at 14:37:23; at 0076 it made the local
pre-migration backup `01a1b14a-f1a0-71c3-accd-51847ec7ecf2` itself), then `orbit-agent`. The old executable is
`/usr/local/bin/orbit-35780dc`; the new one is also kept as `/usr/local/bin/orbit-025fffe`. `vm-health`:
`PROD_TAG=025fffe`, `ROLLBACK_TAG=35780dc`.

Verified: `orbit`, `orbit-agent`, nginx, `orbit-oidc-gateway` and `orbit-oidc-tunnel` active, no restarts; readiness (all
checks); `/orbit-build.json` revision `025fffe`; schema 83; live quick-check and foreign-key checks pass; counts unchanged
(6 users, 1 workspace, 3 projects, 63 tasks, 43 pages, 591 chat messages, 3 Drive nodes, 0 API tokens); sidecar
`/health` `{"ok":true,"mock":false}` and `/opt/orbit-agent/src` equal to `apps/agent/src` of `025fffe9`; the only new log
warning is rmcp's "allowed origin without a port" (also there with `35780dc`). Signed in on a copy of the migrated
production database served by the new binary on loopback, in a network namespace with no outside network: `/drive`
shows the "Drive folders" tree (My Drive, Trash) in the app sidebar, no page errors. **Not** verified: a Donna answer
(a run on production needs a person's mention in chat; a run from a copy can rotate that person's ChatGPT token), and
any signed-in page on production itself.

Rollback (the old binary cannot read schema 83: restore the snapshot together with the binary; work after 14:36 UTC is
lost):

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-35780dc backup verify 01a1b14a-509c-741d-9ba9-30cabd4eb5d1 && /usr/local/bin/orbit-35780dc backup restore 01a1b14a-509c-741d-9ba9-30cabd4eb5d1'
sudo install -m 755 /usr/local/bin/orbit-35780dc /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo mv /opt/orbit-agent.before-025fffe /opt/orbit-agent
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-025fffe.tgz
```

### November 5, 2026 upgrade: Orbit Drive v1 (schema 75)

Fast-forward of `main` from `43120b5a` to `35780dc`, the merge of `main` into `drive-core` (`53101b0f`, M1–M8 and the
Quality & Security fix). Shipped: one Drive per workspace (`/drive`: tree, list/grid, uploads straight to the bucket
with presigned URLs and multipart resume, browser ZIP, trash, search), browser-made thumbnails and the viewer, public
links `/s/<token>` (hashed token, expiry, password, revoke), MCP tools under `drive:read` / `drive:write`, the
`orbit drive import-wellrox-cloud` CLI (not run), and a CSP whose `connect-src`/`img-src`/`media-src` add only the
configured https bucket origins. Migration 0075 (`drive_nodes`, `drive_blobs`, `drive_media`, `drive_shares`); new
hourly schedule `drive.maintenance`; new permissions `drive.purge`, `drive.shares.manage`. No new `ORBIT_*` setting, no
change in `apps/agent`, so the sidecar code was not updated.

Checks on the merged head (`orbit-build`): `rate_limit` (15), `drive_shares_api` (4), platform `backup` (23) and
`database` (44; one run had a flaky `page_search_migration_backfills_and_tracks_pages`, which passed alone and in two
more full runs), `app_smoke` (20, schema 75), `openapi_contract` (18), web build and `tsc`. The branch itself passed all
Drive tests on SQLite and Postgres, the full web suite and the browser e2e runs before the merge.
`orbit-deploy image` built `orbit:35780dc` (label `org.orbit.clean-revision` = the full commit), `deploy/smoke.sh`
passed, and `orbit config check` passed with the extracted binary.

At the switch: stopped `orbit-agent` and `orbit` at 23:39:12 UTC, created snapshot `01a1ae14-acf6-7515-b660-0b2286f942fc`
with the old CLI and verified it, ran `migrate run` of the new binary on a copy in `/var/lib/orbit/rehearsal-35780dc-*`
(1 applied, schema 75, the four Drive tables, quick-check and foreign-key checks pass, counts unchanged; deleted
afterwards), saved the sidecar data as `/var/backups/orbit-agent/orbit-agent-before-35780dc.tgz`, installed the binary
and started `orbit` (ready at 23:40:09; it applied 0075 itself), then `orbit-agent`. The old executable is
`/usr/local/bin/orbit-7a176ff` (equal to the installed one before the swap); the new one is also kept as
`/usr/local/bin/orbit-35780dc`. `vm-health`: `PROD_TAG=35780dc`, `ROLLBACK_TAG=7a176ff`.

Verified: `orbit`, `orbit-agent`, nginx, `orbit-oidc-gateway` and `orbit-oidc-tunnel` active, no restarts; readiness (all
checks); `/orbit-build.json` revision `35780dc`; `/login`, `/drive` and `/s/<token>` answer 200 with the SPA; schema 75;
live quick-check and foreign-key checks pass; counts unchanged (6 users, 1 workspace, 3 projects, 63 tasks, 43 pages, 581
chat messages); `drive.maintenance` scheduled; the Drive API answers 401 without a session and an unknown public link
answers 404 `drive_share_not_found`; the CSP adds only the configured bucket origin; no warning or error in the Orbit log
since the start (nginx errors only during the stop window). The Orbit nginx location already has `access_log off`, so
`/s/` and `/api/v1/public/drive/` tokens are not logged on this VM. **Not** done or verified on production: any
signed-in Drive flow, R2 CORS on `wellrox-files`, `wellrox-files` as the workspace's Drive bucket (no credentials in this
session), the MP4 spot-check, and the wellrox-cloud import (needs a user decision).

**Before members use Drive:** production has no workspace bucket; its installation bucket is `wellrox-orbit-backups`.
Until `wellrox-files` is connected in Settings → File storage (with R2 CORS GET/PUT/HEAD from the Orbit origin), Drive
in the Wellrox workspace picks the backups bucket, where browser uploads fail (no CORS).

Rollback (the old binary cannot read schema 75: restore the snapshot together with the binary; Drive data made after
23:39 UTC is lost, and so is all other work after it):

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-7a176ff backup verify 01a1ae14-acf6-7515-b660-0b2286f942fc && /usr/local/bin/orbit-7a176ff backup restore 01a1ae14-acf6-7515-b660-0b2286f942fc'
sudo install -m 755 /usr/local/bin/orbit-7a176ff /usr/local/bin/orbit
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# Sidecar data, only if needed: sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-35780dc.tgz
```

### November 5, 2026 upgrade: delete account with no workspace (upstream #39)

Fast-forward of `main` from `a9502be1` to `7a176ff` (`delete-account-no-workspace`, port of upstream `0c2492d1`).
Shipped: someone with no workspace opens Account settings from the onboarding, in a workspace-less app frame
(`NoWorkspaceShell`, replaces `OperatorShell`), and can delete their account there; mobile's "No workspaces" screen has
a Delete account button. The dev seed (`just dev`, `just seed`) now opens registration and sets Mailpit when no mail
server is set; production refuses `--seed`, so this does not reach production. Upstream's provider sign-in parts are not
ported. No migration (schema stays 74), no new `ORBIT_*` setting, no change in `apps/agent`, so the sidecar code was not
updated.

Checks on the merged head (`orbit-build`): web `tsc` and all web tests (1541); mobile `tsc`, `bun test src` and the iOS
export (on the branch, same tree); server `migrate_run_seed` tests on SQLite and Postgres; a browser e2e on an isolated
instance (leave the only workspace, Account settings from the onboarding, delete the account, sign-in refused).
`orbit-deploy image` built `orbit:7a176ff` in 1480 s (label `org.orbit.clean-revision` = the full commit; a first
attempt was killed with its shell after 10 minutes), `deploy/smoke.sh` passed (seed/reset blocked), and `orbit config
check` passed with the extracted binary. Production unit and `/etc/orbit/orbit.env`: `ORBIT_ENV=production`, `serve`
only, no seed, Mailpit or registration setting.

At the switch: stopped `orbit-agent` and `orbit` at 19:17:15 UTC, created snapshot `01a1ad24-da16-70b1-91f3-f10cbe80b330`
with the old CLI and verified it, ran `migrate run` of the new binary on a copy in `/var/lib/orbit/rehearsal-7a176ff-*`
(0 applied, schema 74, quick-check and foreign-key checks pass, counts unchanged; deleted afterwards), saved the sidecar
data as `/var/backups/orbit-agent/orbit-agent-before-7a176ff.tgz`, installed the binary and started `orbit` (ready at
19:18:21), then `orbit-agent`. The old executable is `/usr/local/bin/orbit-75bd825` (equal to the installed and the
running one before the swap); the new one is also kept as `/usr/local/bin/orbit-7a176ff`. `vm-health`:
`PROD_TAG=7a176ff`, `ROLLBACK_TAG=75bd825`.

Verified: `orbit`, `orbit-agent`, nginx, `orbit-oidc-gateway` and `orbit-oidc-tunnel` active, no restarts; readiness (all
checks); `/orbit-build.json` revision `7a176ff`; `/login` answers 200 with the SPA; schema 74; live quick-check and
foreign-key checks pass; counts unchanged (6 users, 1 workspace, 3 projects, 63 tasks, 43 pages, 579 chat messages);
registration still closed and no mail server set; no warning or error in the logs since the start. **Not** verified on
production: any signed-in page, and the no-workspace flow itself (production has no user without a workspace).

Rollback (the schema did not change, so the old binary reads the current database; restore the snapshot only if data is
damaged):

```sh
sudo systemctl stop orbit-agent orbit
sudo install -m 755 /usr/local/bin/orbit-75bd825 /usr/local/bin/orbit
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# Only if data must go back to 19:17 UTC (work after it is lost), with both services stopped:
# sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-75bd825 backup verify 01a1ad24-da16-70b1-91f3-f10cbe80b330 && /usr/local/bin/orbit-75bd825 backup restore 01a1ad24-da16-70b1-91f3-f10cbe80b330'
# sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-7a176ff.tgz
```

### November 5, 2026 upgrade: dependency advisories, precompressed web assets and lazy routes

Fast-forward of `main` from `92cf3aa6` to `75bd825` (merge of `main` into `dependency-advisories`, which holds
`asset-compression` and `dep-advisories`). Shipped: rustls 0.23.46, rustls-webpki 0.103.16, webpki-roots 1.0.11, tokio
1.53.3, webauthn-rs 0.6.2 (removes crypto-bigint 0.5.5; passkey sign-in uses `finish_discoverable_authentication_v2`),
calamine 0.38, pulldown-cmark 0.13.6, zod 4.6.7 and expo 58.0.23 (web and mobile tooling); the server sends the web
build's precompressed `.br` / `.gz` files, and the signed-in pages load on first use. `cargo audit` leaves only
RUSTSEC-2023-0071 (rsa, no fixed version; Orbit only generates an RSA key and verifies passkey signatures, not
affected). No migration (schema stays 74), no new `ORBIT_*` setting, no change in `apps/agent`, so the sidecar code was
not updated.

Checks on the branch (`orbit-build`): workspace build and `cargo clippy --workspace --all-targets -- -D warnings`;
`bun install --frozen-lockfile` and the web build; SQLite: platform `object_storage` (11), server `storage_api` (6),
`workspace_storage_api` (3), `page_files_api`, `two_factor_api` (11, with the new passkey register and sign-in test),
`email_api`, `oauth_api`, `notion_client`, `app_smoke`, `markdown_import`, `export_markdown` and the file parser unit
tests; Postgres: `object_storage`, `page_files_api`, `storage_api`, `workspace_storage_api`, `two_factor_api`.
`orbit-deploy image` built `orbit:75bd825` (label `org.orbit.clean-revision` = the full commit), `deploy/smoke.sh`
passed, a check of the image sent `content-encoding: br` and `gzip` for the entry JS and CSS (decoded bytes equal the
plain file), and `orbit config check` passed with the extracted binary.

At the switch: stopped `orbit-agent` and `orbit` at 14:40:52 UTC, created snapshot `01a1ac27-d108-727a-a1da-89e1c97c9322`
with the old CLI (to the S3 bucket) and verified it, ran `migrate run` of the new binary on a copy in
`/var/lib/orbit/rehearsal-75bd825-*` (0 applied, schema 74, quick-check and foreign-key checks pass, counts unchanged;
deleted afterwards), saved the sidecar data as `/var/backups/orbit-agent/orbit-agent-before-75bd825.tgz`, installed the
binary and started `orbit` (ready at 14:41:40), then `orbit-agent`. The old executable is `/usr/local/bin/orbit-f3b830e`
(equal to the installed one before the swap); the new one is also kept as `/usr/local/bin/orbit-75bd825`. `vm-health`:
`PROD_TAG=75bd825`, `ROLLBACK_TAG=f3b830e`.

Verified: `orbit`, `orbit-agent`, nginx, `orbit-oidc-gateway` and `orbit-oidc-tunnel` active, no restarts; readiness (all
checks); `/orbit-build.json` revision `75bd825`; schema 74; live quick-check and foreign-key checks pass; counts unchanged
(6 users, 1 workspace, 3 projects, 63 tasks, 40 pages, 548 chat messages, 0 passkeys, 0 workspace buckets); no warning or
error in the logs since the start. Through nginx: the entry JS is sent as br (75 kB) and gzip (88 kB) of 285 kB, both
decode to the plain file; `passkeys_enabled` is true and a passkey sign-in starts (rpId `orbit.wellrox.com`). Outbound
TLS of the new binary: `backup verify` of the new snapshot from R2, run on a database copy with an empty local backup
folder. SMTP is not set up (`email_enabled` false), so no test email. `vm-health`: all OK except the nginx errors of the
switch and `.git` files owned by root. **Not** verified on production: any signed-in page, a save in Admin → Storage, and
a passkey added and used by a person (production has no passkeys yet).

Rollback (the schema did not change, so the old binary reads the current database; restore the snapshot only if data is
damaged):

```sh
sudo systemctl stop orbit-agent orbit
sudo install -m 755 /usr/local/bin/orbit-f3b830e /usr/local/bin/orbit
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# Only if data must go back to 14:40 UTC (work after it is lost), with both services stopped:
# sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-f3b830e backup verify 01a1ac27-d108-727a-a1da-89e1c97c9322 && /usr/local/bin/orbit-f3b830e backup restore 01a1ac27-d108-727a-a1da-89e1c97c9322'
# sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-75bd825.tgz
```

### November 5, 2026 upgrade: overlapping storage buckets and 12 fixes

Fast-forward of `main` from `165c6686` to `f3b830e` (`storage-bucket-overlap`): Admin → Storage refuses a bucket that
overlaps a workspace bucket (also a removed one that still drains), a workspace bucket is refused when it overlaps the
installation's bucket or another workspace's bucket, and the mover never deletes an object whose source and destination are
one physical object (the cause of the files lost on November 5 at 10:28 UTC). Also shipped, since `5b64346`: 11 upstream
fixes (`d7cb2f13` to `165c6686`) and the November 4 record. No migration (schema stays 74), no new `ORBIT_*` setting, no
change in `apps/agent`, so the sidecar code was not updated.

Before the switch (read only): `instance_settings` has the bucket `wellrox-orbit-backups` (R2, prefix empty, attachments
off, backups on); `workspace_storage` has no row, so no workspace bucket is active or draining.

Checks on the branch: platform `object_storage` (11) and server `storage_api` (6) and `workspace_storage_api` (3) on SQLite
and Postgres; `cargo clippy -p orbit-platform -p orbit-server --all-targets`; `cargo fmt --check`. `orbit-deploy image`
built `orbit:f3b830e` (label `org.orbit.clean-revision` = the full commit), `deploy/smoke.sh` passed, and
`orbit config check` passed with the extracted binary.

At the switch: stopped `orbit-agent` and `orbit` at 11:17:42 UTC, created snapshot `01a1ab6d-cdb9-7496-8eb9-94c66da02a08`
with the old CLI (it went to the S3 bucket, as backups are in S3; the CLI cannot list S3 while the server runs) and verified
it, ran `migrate run` of the new binary on a copy in `/var/lib/orbit/rehearsal-f3b830e-*` (0 applied, schema 74,
quick-check and foreign-key checks pass, counts unchanged; deleted afterwards), saved the sidecar data as
`/var/backups/orbit-agent/orbit-agent-before-f3b830e.tgz`, installed the binary and started `orbit` at 11:18:42 UTC (ready at
11:18:43), then `orbit-agent`. The old executable is `/usr/local/bin/orbit-5b64346` (equal to the installed one before the
swap); the new one is also kept as `/usr/local/bin/orbit-f3b830e`. `vm-health`: `PROD_TAG=f3b830e`, `ROLLBACK_TAG=5b64346`.

Verified: `orbit`, `orbit-agent`, nginx, `orbit-oidc-gateway` and `orbit-oidc-tunnel` active, no restarts; readiness (all
checks); `/orbit-build.json` revision `f3b830e`; schema 74; live quick-check and foreign-key checks pass; counts unchanged
(6 users, 1 workspace, 3 projects, 63 tasks, 40 pages, 548 chat messages, 0 workspace buckets); sidecar `mock: false`; no
warning or error in the logs since the start. `vm-health`: all OK except the nginx errors of the 1-minute switch, the age of
the newest local snapshot (the new one is in S3) and `.git` files owned by root from other sessions.
**Not** verified on production: any signed-in page and a save in Admin → Storage.

Rollback (the schema did not change, so the old binary reads the current database; restore the snapshot only if data is
damaged):

```sh
sudo systemctl stop orbit-agent orbit
sudo install -m 755 /usr/local/bin/orbit-5b64346 /usr/local/bin/orbit
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
# Only if data must go back to 11:17 UTC (work after it is lost), with both services stopped:
# sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-5b64346 backup verify 01a1ab6d-cdb9-7496-8eb9-94c66da02a08 && /usr/local/bin/orbit-5b64346 backup restore 01a1ab6d-cdb9-7496-8eb9-94c66da02a08'
# sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-f3b830e.tgz
```

### November 4, 2026 upgrade: upstream port, Suggest a change, the app bar at once and files in agent chats

Merged into `main` (`01ba182`): `upstream-port-1103` (`059ffa6`, 22 commits, fast-forward), `improvement-requests` (`4063977`;
merge `bf80423`), `app-bar-immediate` (`2288998`, as it is; merge `71abb2e`) and `agent-chat-files` (`d60c3b0`; merge `1d26e4a`),
then `5b64346` (rustfmt import order, already wrong on the port branch). Conflicts: the schema version (73 vs 74) in `backup.rs`
and the tests, resolved to 74 with migration 73 before 74 in `migrate.rs`; `Topbar.tsx` kept the port's new phone bar (the
other branch only added a breadcrumb label that the port removed); `.ai/lessons.md` kept both sides without the obsolete
`.last-run.json` line; the OpenAPI document and `packages/api` were generated again from the merged code (208 paths, 267
operations; new: `GET /api/v1/workspaces/{workspace_id}/chat/files/{file_id}/content`). Migrations 73
(`0073_improvement_requests.sql`, additive; the feature is off until an admin turns it on) and 74
(`0074_confirm_setup_owner_email.sql`: sets `email_verified_at` of the setup owner, 1 user here). No new `ORBIT_*` setting;
`/etc/orbit/orbit.env`, `/etc/orbit/agent.env`, nginx, the OIDC gateway and the tunnel did not change. `apps/agent` changed
(6 source files), so the sidecar code was updated.

Checks on merged `main`: on SQLite and Postgres, platform `backup` (23) and `database` (44), and server `openapi_contract` (17),
`app_smoke` (18), `improvements_api` (5), `chat_api` (29, 1 ignored), `agent_api` (13), `storage_api` (4) and
`backup_items_api` (8); `cargo fmt --check`; `cargo clippy --workspace --all-targets -D warnings`; web `tsc -b`, oxlint
(warnings only) and 288 tests (the changed test files and the apps, shell and chat folders); `packages/api` (10); sidecar
typecheck and 40 tests.

Build: the first `orbit-deploy image` (11:15 UTC) stalled at `MemoryHigh=3G` of `orbit-builds.slice` (the release compile of
`orbit-server` needs about 2.8 GB; rustc spent its time in memory reclaim) and was cancelled at 13:13 UTC. With the user's
approval, `MemoryHigh` was set to 4G for the build only (`systemctl set-property --runtime`); the second build (13:48 UTC) was
stopped by a VM restart at about 13:59 UTC (the VM came back with 16 GB). The third build, as the transient unit
`orbit-deploy-5b64346.service`, ran 14:00:18 to 14:02:40 UTC (the compile from the second build was in the cache mount), and
`MemoryHigh` went back to 3G after it. `deploy/smoke.sh` passed; the extracted binary has revision `5b64346`, both migrations
and the new route, and `orbit config check` passed with it.

At the switch: stopped `orbit-agent` and `orbit` at 14:06:34 UTC, created and verified snapshot
`01a1a6e2-0eb8-7703-b833-e9d3ae786020` (`/var/backups/orbit/snapshots/01a1a6e2-0eb8-7703-b833-e9d3ae786020`) with the old CLI,
rehearsed the migrations on a copy in `/var/lib/orbit/rehearsal-5b64346-tcR8Ko` (2 applied, schema 74, quick-check and
foreign-key checks pass, counts unchanged; deleted afterwards), saved the sidecar data as
`/var/backups/orbit-agent/orbit-agent-before-5b64346.tgz`, moved the old sidecar code to `/opt/orbit-agent.before-5b64346` and
the new one (`npm ci --omit=dev`, same 53 modules) to `/opt/orbit-agent`, installed the binary at 14:06:52 UTC and started
`orbit` (ready 14:06:59 UTC), then `orbit-agent`. The old executable is `/usr/local/bin/orbit-effa521` (checked equal to the
installed one before the swap); the new one is also kept as `/usr/local/bin/orbit-5b64346`.

Verified: `orbit`, `orbit-agent`, nginx, `orbit-oidc-gateway` and `orbit-oidc-tunnel` active, no restarts; readiness (all
checks); `/orbit-build.json` revision `5b64346`; schema 74 (73 and 74 applied); live quick-check and foreign-key checks pass;
counts unchanged (6 users, 1 workspace, 3 projects, 63 tasks, 32 pages, 28 page files, 43 blobs, 508 chat messages, 80 agent
inputs, 1 integration, 2 apps); 0 improvement requests and the feature off; no new warning or error in the logs; the sidecar
runs from the new `/opt/orbit-agent` with `mock: false`. Signed out, the new chat file route answers 401 (an unknown path 404
`route_not_found`). Chromium (desktop 1440×900 and a phone identity 390×844) through nginx: the signed-out pages answer 200
with no horizontal overflow, the app and Agentic pages go to `/login`, https://orbit.wellrox.com/ goes to the exe.dev login,
and the two CSP inline-style errors of earlier versions are gone. On an isolated development instance of the same image
(seed data, deleted afterwards): an unknown chat file answers 404 `chat_not_found`; an embedded app that answers after 20 s
shows its bar after 0.4 s on desktop and phone; Admin → Feedback & fixes opens. Screenshots: `/root/deploy-1104/shots/`.
**Not** verified on production: any signed-in page, a real agent run with a chat file, and real WebKit on a phone.

Rollback (an older binary cannot read a version 74 database; preserve the current database first; work after the snapshot is
lost):

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-effa521 backup verify 01a1a6e2-0eb8-7703-b833-e9d3ae786020 && /usr/local/bin/orbit-effa521 backup restore 01a1a6e2-0eb8-7703-b833-e9d3ae786020'
sudo install -m 755 /usr/local/bin/orbit-effa521 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo cp -a /opt/orbit-agent.before-5b64346 /opt/orbit-agent
sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-5b64346.tgz
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
```

### November 3, 2026 upgrade: the app sign-in card and lighter Docs PDFs

Merged `app-signin-card` (`dbceae5`, fast-forward) and `web-perf` (`b2d9d90`) into `main`: merge commit `effa521`. Contents: a
"Sign in to <app>" card when an embedded app does not load (frame blocked, or the app's sign-in needs Orbit login); `PdfPages`
frees the bitmaps of PDF pages off screen; the realtime flush timer runs only while a refresh waits (report: `.ai/web-perf.md`).
Server code changed too (`apps/server/src/apps/{mod,oidc}.rs`, `crates/platform/src/http/security.rs`): the security layer keeps
a response's own `Referrer-Policy` (the framed `login_required` page sends `no-referrer`); other pages still get
`strict-origin-when-cross-origin`. No migration (schema stays 72), no new setting, no `apps/agent` change. `/etc/orbit/orbit.env`,
`/etc/orbit/agent.env`, nginx, the OIDC gateway and the tunnel did not change; the sidecar was only restarted.

Before the merge, 347 tracked files and some folders in this checkout belonged to root (earlier git runs as root), so git as
exedev could not update them. They now belong to exedev again (list: `/root/deploy-signin-perf/root-owned.txt`); the merge ran as
exedev. `apps/web/dist`, `target` and `apps/server/backups` were not changed.

Checks on merged `main` before the build: web typecheck, oxlint (warnings only), web unit tests (1533 pass), and
`cargo test -p orbit-server --test apps_api` on SQLite (11). The branches were tested before (`apps_api` on SQLite and Postgres,
`scripts/apps-demo/e2e-card.mjs` in Chromium, `useWorkspaceEvents` tests). `orbit:effa521` was built with `orbit-deploy image`
(10:09:58 to 10:24:18 UTC, 14 minutes, warm cache) and `deploy/smoke.sh` passed. The extracted binary has revision `effa521`;
`orbit config check` with it passed.

At the switch: stopped `orbit-agent` and `orbit` at 10:25:06 UTC, created and verified snapshot
`01a1a0f0-ecdf-7172-9d3c-15331f8851e5` (`/var/backups/orbit/snapshots/01a1a0f0-ecdf-7172-9d3c-15331f8851e5`) with the old CLI,
saved the sidecar data as `/var/backups/orbit-agent/orbit-agent-before-effa521.tgz` (the sidecar code did not change, so no code
copy), installed the binary at 10:25:14 UTC and started `orbit` (ready 10:25:16 UTC), then `orbit-agent`. The old executable is
`/usr/local/bin/orbit-b051881` (checked equal to the installed and running one before the swap); the new one is also kept as
`/usr/local/bin/orbit-effa521`.

Verified: `orbit`, `orbit-agent`, nginx, `orbit-oidc-gateway` and `orbit-oidc-tunnel` active, no restarts; readiness (all checks);
`/orbit-build.json` revision `effa521`; sidecar `mock: false`; schema 72; live quick-check and foreign-key checks pass; counts
unchanged (6 users, 1 workspace, 3 projects, 62 tasks, 32 pages, 28 page files, 42 blobs, 474 chat messages, 72 agent inputs, 1
integration, 2 apps); no new warning or error in the logs; the served bundle has the card (`app-sign-in-card`); `/login` answers
`Referrer-Policy: strict-origin-when-cross-origin`, COOP `same-origin-allow-popups` and `X-Frame-Options: DENY`; the OIDC gateway
answers `/oauth/jwks` 200 and `/login` 404 on loopback. Chromium (desktop 1440×900 and a phone identity 390×844) through nginx:
`/login`, `/recovery`, `/register`, `/setup`, `/accept-invitation` and `/confirm-email` answer 200 with no horizontal overflow;
`/apps/product-development` and `/agentic` go to `/login`; https://orbit.wellrox.com/ goes to the exe.dev login. Screenshots:
`/root/deploy-signin-perf/shots/`. Each page logs two CSP inline-style errors; `orbit:b051881` logs the same two, so they are not
new. **Not** verified on production: any signed-in page (the card on a real app, a PDF page), and real WebKit on a phone.

Rollback (no schema change, so the old binary can read the database; the snapshot is needed only if data is damaged):

```sh
sudo systemctl stop orbit-agent orbit
sudo install -m 755 /usr/local/bin/orbit-b051881 /usr/local/bin/orbit
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
```

### November 2, 2026 upgrade: upstream port, the Agentic page and the app bar X button

Merged `agent-settings-cleanup` (`11b1e7d`, fast-forward) and `app-bar-close-button` (`e1e862a`) into `main`: merge commit
`b051881`. Contents: the Agentic page (`/agentic`, one menu item with You, Workspace and Admin sections; old agent addresses
redirect), hand-ported upstream `coollabsio/orbit` commits `536c5ac2` (self-hosted security hardening), `c77aea9c`, `a60784da`
(scoped MCP), `2a431ab1` (account deletion, email change, workspace exports) and `5cb47ec0`, and the X that closes an app's bar.
Migrations 70 (`0070_account_lifecycle.sql`), 71 (`0071_workspace_exports.sql`) and 72 (`0072_mcp_permissions.sql`), all
additive. No new `ORBIT_*` setting; `/etc/orbit/orbit.env`, `/etc/orbit/agent.env` and nginx did not change. `apps/agent`
changed (messages and prompt name Agentic and link to `/agentic`), so the sidecar code was updated.

Behavior changes the user accepted: an app's sign-in tabs escape the frame sandbox (Orbit now sends
`Cross-Origin-Opener-Policy: same-origin-allow-popups`, and no `X-Frame-Options` on the app frame wrapper, which has its own
CSP); admins cannot change their own email on the Admin route (Profile, with the password); MCP task writes need the new
`tasks:write` token scope, and the REST `write` flag gives no MCP access (production had 0 API tokens and 0 OAuth grants). New
HTTP limits: request headers within 30 seconds, 60-second request timeout and a 2 MiB JSON body cap outside uploads, exports,
downloads, admin routes, page file saves and agent tool calls. Keep a proxy's upstream keep-alive under 30 seconds (nginx here
uses no upstream keep-alive).

Checks on merged `main` before the build: web typecheck, oxlint, web tests for apps, settings, agent, admin and shell (174).
The branch was tested before (server tests on SQLite and Postgres, `scripts/apps-demo/e2e.mjs` 27 checks in Chromium).
`orbit:b051881` was built with `orbit-deploy image` (13:41:25 to 14:24:29 UTC, 43 minutes) and `deploy/smoke.sh` passed.

At the switch: stopped `orbit-agent` and `orbit` at 14:24:59 UTC, created and verified snapshot
`01a19ca6-3215-7153-8920-36185614b8ab` (`/var/backups/orbit/snapshots/01a19ca6-3215-7153-8920-36185614b8ab`) with the old CLI,
rehearsed migrations 70 to 72 on a copy in `/var/lib/orbit/rehearsal-b051881-GV96hC` (3 applied, schema 72, quick-check and
foreign-key checks pass, counts unchanged; deleted afterwards), saved the sidecar data as
`/var/backups/orbit-agent/orbit-agent-before-b051881.tgz` and the old sidecar code as `/opt/orbit-agent.before-b051881`, installed
the new sidecar (`npm ci --omit=dev`, same 53 modules) and the binary, and started `orbit` (ready 14:25:41 UTC), then
`orbit-agent`. The old executable is `/usr/local/bin/orbit-f005d4b` (checked equal to the installed one before the swap); the new
one is also kept as `/usr/local/bin/orbit-b051881`.

Verified: `orbit`, `orbit-agent`, nginx, `orbit-oidc-gateway` and `orbit-oidc-tunnel` active, no restarts; readiness (all
checks); `/orbit-build.json` revision `b051881`; sidecar `mock: false`; schema 72; live quick-check and foreign-key checks pass;
counts unchanged (6 users, 1 workspace, 3 projects, 62 tasks, 31 pages, 28 page files, 42 blobs, 429 chat messages, 54 agent
inputs, 1 integration); no new warning or error in the logs (the existing "allowed origin without a port" warning stays);
`/agentic`, `/settings/agent-tools` and `/apps/product-development` answer 200 and, signed out, go to `/login`; the served
bundle has the X button ("Close bar") and the `/agentic/workspace` links; the app shell answers COOP
`same-origin-allow-popups` with `X-Frame-Options: DENY`; https://orbit.wellrox.com/ answers 307 to the exe.dev login; the OIDC
gateway answers `/oauth/jwks` 200 and `/login` 404 on loopback. The frame wrapper headers (no `X-Frame-Options`, own CSP) were
checked with the deployed binary on an isolated instance, because production needs a signed-in session for it.
**Not** verified on production: any signed-in page (the Agentic sections, the redirect targets, the X button on the Products
app, a sign-in popup), the tunnel's public host, and a real agent run.

Rollback (an older binary cannot read a version 72 database; preserve the current database first; work after the snapshot is
lost):

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-f005d4b backup verify 01a19ca6-3215-7153-8920-36185614b8ab && /usr/local/bin/orbit-f005d4b backup restore 01a19ca6-3215-7153-8920-36185614b8ab'
sudo install -m 755 /usr/local/bin/orbit-f005d4b /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo cp -a /opt/orbit-agent.before-b051881 /opt/orbit-agent
sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-b051881.tgz
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
```

### November 1, 2026 tools: deploy lock and BuildKit cache (no deploy)

Fast-forwarded `main` to `dba6934`: deploy tools and docs only, so no image build and no binary switch. Production stays on
`f005d4b`. Installed `/usr/local/bin/orbit-deploy` and the lock `/var/lib/orbit-deploys/deploy.lock` (root, mode 0666), and the
new `/etc/orbit-buildkit.toml` (6 GB cache limit, explicit GC rules). The earlier config is
`/etc/orbit-buildkit.toml.before-deploy-lock`. Restarted `orbit-buildkit` under the build lock at 16:16 UTC.

Verified: `buildx inspect` shows 4 GC rules at 6 GiB; `orbit-buildkit-state` is kept (832 MB before and after); the `orbit-limited`
builder answers; as root and as exedev, a second `begin` is refused (exit 75), and the lock is released after `end`; cleanup
dry-run only; `orbit`, `orbit-agent`, nginx and `orbit-oidc-gateway` are active with revision `f005d4b` and readiness (all checks).
**Not** verified: a real build with the new rules or a warm build, a real `orbit-deploy image` (mock tests only), a reboot, and
GC under low disk. The real cleanup was not run.

Rollback of the config (only when no build runs) and of the tool (only when no deploy runs):

```sh
sudo install -m 0644 /etc/orbit-buildkit.toml.before-deploy-lock /etc/orbit-buildkit.toml
sudo flock -n /var/lib/orbit-builds/build.lock systemctl restart orbit-buildkit.service
flock -n /var/lib/orbit-deploys/deploy.lock sudo rm /usr/local/bin/orbit-deploy
```

### November 1, 2026 upgrade: Orbit Apps on phones, the app bar and a clearer Settings → Apps

Fast-forwarded `main` to `orbit-apps` (`f005d4b`, 12 commits) at 15:34:35 UTC: the app page's phone layout, the bar that hides on
the app's ready signal (`orbit:app-ready`, see `docs/apps.md`), and the Settings → Apps redesign. Web, docs and test scripts only:
no migration (schema stays 69), no new setting, no server or `apps/agent` change. `/etc/orbit/orbit.env`, `/etc/orbit/agent.env`,
nginx, the OIDC gateway, the tunnel and the sidecar code did not change, and the sidecar was only restarted around the switch.

Checks before the merge, on `f005d4b` with `main` `e136805` merged: web typecheck, oxlint on the changed areas, tests for apps,
shell and auth (101), and the browser tests in `scripts/apps-demo/` against an isolated instance: `e2e-settings.mjs` (66),
`e2e.mjs` (25) and `e2e-mobile.mjs` (118). Only Chromium, on SQLite. The phone runs use a phone identity, not real WebKit. The
server test suite and the full workspace suite were not run, because no server code changed.

`orbit:f005d4b` was built from a clean `git archive` of `main` with
`orbit-build /usr/bin/docker buildx build --builder=orbit-limited --load` (15:34:35 to 16:13:41 UTC, 39 minutes), and
`deploy/smoke.sh` passed. The extracted binary has the revision and the new web code.

At the switch: stopped `orbit-agent` and `orbit`, created and verified snapshot `01a197e3-de77-754a-8b09-b4339a5d48d6`
(`/var/backups/orbit/snapshots/01a197e3-de77-754a-8b09-b4339a5d48d6`) with the old CLI, installed the binary at 16:14:15 UTC,
and started `orbit`, then `orbit-agent`; ready at 16:14:17 UTC. The old executable is `/usr/local/bin/orbit-6a817b9` (checked equal
to the running one before the swap), and the new one is also kept as `/usr/local/bin/orbit-f005d4b`.

Verified: `orbit`, `orbit-agent`, nginx, `orbit-oidc-gateway` and `orbit-oidc-tunnel` are active; readiness 200 (all checks) on
loopback; revision `f005d4b`; the served JS has the new code; counts unchanged (6 users, 1 workspace, 3 projects, 62 tasks, 31
pages, 42 blobs, 427 chat messages, 54 agent inputs, 1 integration), quick-check and foreign-key checks pass; the
`product-development` row of `workspace_apps` (client ID, redirect URI, roles, open mode, `updated_at`) is byte-equal before and
after; the apps route answers 401 without a session and a foreign origin write 403; https://orbit.wellrox.com/ answers 307 to the
exe.dev login; through the tunnel `/oauth/jwks` 200 with one key of public fields and `/login` 404. A signed-in test on production and
a test on a real iPhone were **not** done from this VM: they need a person's login and device. The bar on the Products app stays
until its owner adds the ready snippet from `docs/apps.md`.

Rollback (no schema change, so the old binary can read the database; the snapshot is needed only if data is damaged):

```sh
sudo systemctl stop orbit-agent orbit
sudo install -m 755 /usr/local/bin/orbit-6a817b9 /usr/local/bin/orbit
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
```

### November 1, 2026 upgrade: resize the file panel on Docs pages

Fast-forwarded `main` to `docs-file-panel-resize` (`6a817b9`, 3 commits): a divider between a page and its open file, and a
full screen button in the panel header. Web only: no migration (schema stays 69), no new setting, no server or `apps/agent`
change. `/etc/orbit/orbit.env`, `/etc/orbit/agent.env`, nginx and the sidecar code did not change, and the sidecar was only
restarted around the switch.

Checks before the merge, on `6a817b9`: `FileSplit.test.ts` (7), web typecheck, oxlint on the changed files; the browser tests
in `apps/web/e2e/file-panel.spec.ts` ran against an isolated instance built from `f41e235`, which differs only in that test
file: the two older tests once and the divider test 5 times, all pass. One earlier run on `f41e235` failed because the test
read the panel width during its 150ms transition (`duration-150` gives `transition: all`); the test now polls, with the same
limits. Only Chromium, on SQLite. The server test suite was not run, because no server code changed.

`orbit:6a817b9` was built from a clean `git archive` of `main` with
`orbit-build /usr/bin/docker buildx build --builder=orbit-limited --load` (about 35 minutes), and `deploy/smoke.sh` passed.

At the switch: stopped `orbit-agent` and `orbit`, created and verified snapshot `01a19791-b271-735b-a6b0-52db18317043`
(`/var/backups/orbit/snapshots/01a19791-b271-735b-a6b0-52db18317043`) with the old CLI, installed the binary, and started `orbit`
directly after the swap, then `orbit-agent`. The old executable is `/usr/local/bin/orbit-857ce8e` (checked equal to the running
one before the swap), and the new one is also kept as `/usr/local/bin/orbit-6a817b9`.

Verified: `orbit`, `orbit-agent`, nginx and `orbit-oidc-gateway` are active with no restarts; readiness 200 (all checks); the
installed binary's hash equals the one from the image; revision `6a817b9` is in the binary and in the served
`/assets/index-BSfbUl0T.js`, with "Exit full screen", "Resize file panel" and `orbit:file_panel_share`; sidecar `mock: false`;
`/api/v1/auth/me` answers 401 with no session, with HSTS; canonical login 400, foreign 403; https://orbit.wellrox.com/ answers
307 to the exe.dev login. No errors in the log. `deploy/verify-production.sh` passes against `http://127.0.0.1:8000` with the
`X-Forwarded-Proto: https` header. On the public origin it stops at the auth check with 307, because the exe.dev gate answers
before Orbit; this does not depend on the Orbit binary.

**Not** verified: the signed-in divider and button on production (it needs a person's login; the browser tests ran only on the
test instance), and browsers other than Chromium. Known cosmetic point: during a drag the panel edge follows the divider with
a small lag (at normal speed a median of 11px and a maximum of 34px, 0 at rest), from the same transition.

Rollback (no migration, so the old binary reads this database; the snapshot is not needed):

```sh
sudo systemctl stop orbit-agent orbit
sudo install -m 755 /usr/local/bin/orbit-857ce8e /usr/local/bin/orbit
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
```

### October 31, 2026 upgrade: open and edit files on Docs pages

Fast-forwarded `main` to `docs-file-preview` (`857ce8e`, 4 commits). No new setting; `/etc/orbit/orbit.env`,
`/etc/orbit/agent.env` and nginx did not change. New crates: calamine, csv, pdf-extract, rustix; new frontend package:
pdfjs-dist. The server now starts itself as a limited child process through `current_exe` to parse files
(`parse-file`, `patch-xlsx`), so **restart `orbit` directly after any binary swap**.

Order: backup `01a192d9-627d-7790-84d0-ad361e2bc82e` (verified) while stopped; Orbit started again. Rehearsal before the
merge: the first try with the test instance's binary stopped with "migration 68 is not known" and changed nothing, because
that binary is older than `main`. `orbit:857ce8e` was then built from a clean `git archive` of the commit with
`orbit-build /usr/bin/docker buildx build --builder=orbit-limited --load`, and migration 69 was rehearsed with it. After the
merge, the image was built again from `main` (a full compile, about 35 minutes with one job), and `deploy/smoke.sh` passed.

At the switch: stopped `orbit-agent` and `orbit`, created and verified the final snapshot
`01a19321-ed03-722e-86e5-cb29bc6227b1` (`/var/backups/orbit/snapshots/01a19321-ed03-722e-86e5-cb29bc6227b1`; use this one for
rollback), saved the sidecar data as `/var/backups/orbit-agent/orbit-agent-before-857ce8e.tgz` and the old sidecar code as
`/opt/orbit-agent.before-857ce8e`, and rehearsed migration 69 again on a copy of it. The rehearsal copies are deleted.
Applied it live: schema 69. Counts before and after: 6 users, 1 workspace, 3 projects, 61 tasks, 31 pages, 28 page files,
43 blobs, 396 chat messages, 40 agent inputs, 31 `page_search` rows, with the same hash of all `page_search` rows. The old
executable is `/usr/local/bin/orbit-ab45847`, and the new one is also kept as `/usr/local/bin/orbit-857ce8e`.

Verified: `orbit`, `orbit-agent`, nginx and `orbit-oidc-gateway` are active with no restarts; readiness (all checks);
revision `857ce8e`; sidecar `mock: false`; live quick-check and foreign-key checks pass; 28 page files at version 1 and 0 file
versions; FTS5 `integrity-check` passes on a copy of the final snapshot; a `page_search` query returns rows. The page file
list, file, content and versions routes answer 401 without a session, and restore answers 403 from a foreign origin and 415
with no session and no body (an unknown route answers 404). Frontend assets and the PDF worker asset answer 200, and the CSP
is unchanged (`default-src 'self'` covers the same-origin worker). Canonical login 400, foreign 403;
https://orbit.wellrox.com/ answers 307 to the exe.dev login. No warnings in the logs. The sidecar's tests pass on this tree.
The `parse-file` child ran as user `orbit` with the service's sandbox settings on a read-only copy of every stored file of
these types, and all parsed: 3 `.xlsx`, 5 `.csv`, 9 `.pdf`, 6 `.docx`. Only output shape was inspected.

**Not** verified: the signed-in file panel in a browser, PDF rendering, saving a cell or restoring a version on a real file
(`patch-xlsx`), any real `.xlsm` file (production has none), and an agent run with the file tools on a real model.
The server test suite was not rerun on `main`, because the tree equals the tested branch.

Rollback (an older binary cannot read a version 69 database; preserve the current database first; work after the snapshot
is lost):

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-ab45847 backup verify 01a19321-ed03-722e-86e5-cb29bc6227b1 && /usr/local/bin/orbit-ab45847 backup restore 01a19321-ed03-722e-86e5-cb29bc6227b1'
sudo install -m 755 /usr/local/bin/orbit-ab45847 /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo cp -a /opt/orbit-agent.before-857ce8e /opt/orbit-agent
sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-857ce8e.tgz
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
```

### October 30, 2026 upgrade: OAuth client credentials and GraphQL reads for agent tools

Merged `agent-oauth-client-credentials` (`9677f60`) into `main` (merge `ab45847`; only this file differs from the tested
branch). Tests on this code are in the branch's report: `agent_integrations` (17), `agent_api` (13), `app_smoke` (18),
platform `backup` (21) and `database` (43) on SQLite and Postgres; the agent unit tests (27), Clippy, web typecheck and
lint, and the browser e2e on SQLite. After the last small fix (two joined `if` statements), the server tests were rerun
on SQLite and `agent_integrations` on Postgres; `agent_api`, `app_smoke`, `backup` and `database` on Postgres were not
rerun, and nothing was rerun on the merge, because its code is the same. The full workspace suite was not run. The image
`orbit:ab45847` built with the limited builder and `deploy/smoke.sh` passed.

Stopped `orbit-agent` and `orbit`. Counts before: schema 67, 6 users, 1 workspace, 3 projects, 61 tasks, 31 pages, 43
attachment blobs, 390 chat messages, 40 agent inputs, 0 integrations. Created and verified snapshot
`01a18d0e-d881-7468-8557-5724f1fea800` (`/var/backups/orbit/snapshots/01a18d0e-d881-7468-8557-5724f1fea800`) with the old
CLI. Saved the sidecar data as `/var/backups/orbit-agent/orbit-agent-before-ab45847.tgz`. Sidecar code did not change. The
old executable is `/usr/local/bin/orbit-6808e1b`, and the new one is also kept as `/usr/local/bin/orbit-ab45847`. Rehearsed
migration 68 (`0068_agent_integration_oauth.sql`, five new columns on `agent_integrations`) on a copy in
`/var/lib/orbit/rehearsal-ab45847-AfA1Rk` (deleted afterwards): quick-check and foreign-key checks pass, counts unchanged.
Applied it live: schema version 68, quick-check and foreign-key checks pass, all counts unchanged. `/etc/orbit/orbit.env`,
`/etc/orbit/agent.env` and nginx did not change. No new setting is needed.

Verified: `orbit`, `orbit-agent`, nginx and `orbit-oidc-gateway` are active; readiness (all checks); sidecar listens; no
error in the `orbit` log; `/settings/agent-tools` answers 200 and the agent-integrations API answers 401 with no session;
https://orbit.wellrox.com/ answers 307 to the exe.dev login. Workspace settings → Agent tools was checked signed in, in a
browser on loopback, against a copy of the migrated production database served by the new binary (a session was added and
the push subscriptions removed in the copy only; the copy was deleted): the HTTP form has the OAuth choice, and a
`.myshopify.com` base URL fills in the token URL, header, test path and GraphQL path, with no page errors. The form was not
saved. The live signed-in page, a real Shopify call and an agent run with a tool were **not** tested: they need a person's
login and credentials. No credential was added: production has 0 integrations.

Rollback (an older binary cannot read a version 68 database; preserve the current database first; work after the snapshot
is lost):

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-6808e1b backup verify 01a18d0e-d881-7468-8557-5724f1fea800 && /usr/local/bin/orbit-6808e1b backup restore 01a18d0e-d881-7468-8557-5724f1fea800'
sudo install -m 755 /usr/local/bin/orbit-6808e1b /usr/local/bin/orbit
sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-ab45847.tgz
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
```

### October 30, 2026 upgrade: Orbit Apps and Sign in with Orbit

Merged `orbit-apps` (`34a1eb5`) into `main` (merge `6808e1b`; only this file differs from the tested branch). Tests on this
code are in the branch's report. The full workspace suite was not run. Platform `database` was run once in full, with one
schema check failing; that check was fixed and rerun alone. The real Cloudflare sign-in, Safari and the iOS bundle were not checked. The image `orbit:6808e1b` built with the
limited builder and `deploy/smoke.sh` passed. An earlier build stopped when the VM rebooted at about 11:01 UTC, which also
grew the disk to 40 GB. A backup from 10:49 UTC (`01a18c6e-41ff-7553-ad0f-89f91fef3183`) is older than the one below: do not
use it for rollback.

Stopped `orbit-agent` and `orbit`. Counts before: schema 66, 6 users, 1 workspace, 3 projects, 61 tasks, 31 pages, 43
attachment blobs, 381 chat messages, 40 agent inputs, 0 integrations. Created and verified snapshot
`01a18c97-844c-7482-998d-618f250d27be` (`/var/backups/orbit/snapshots/01a18c97-844c-7482-998d-618f250d27be`) with the old
CLI. Saved the sidecar data as `/var/backups/orbit-agent/orbit-agent-before-6808e1b.tgz`. Sidecar code did not change, so
no new code copy was made. The old executable is `/usr/local/bin/orbit-aedbc85`, and the new one is also kept as
`/usr/local/bin/orbit-6808e1b`. Rehearsed migration 67 (`0067_workspace_apps.sql`, four new tables) on a copy in
`/var/lib/orbit/rehearsal-6808e1b-*` (deleted afterwards): quick-check and foreign-key checks pass. Applied it live: schema
version 67, quick-check and foreign-key checks pass, all counts unchanged. `/etc/orbit/orbit.env`, `/etc/orbit/agent.env` and
the main nginx did not change.

Verified: `orbit`, `orbit-agent` and nginx are active; readiness (all checks); revision `6808e1b`; sidecar listens;
discovery 200 with issuer `https://orbit.wellrox.com`; `/oauth/jwks` 200 with no keys yet (the first app with sign-in makes
the key); the apps routes and userinfo answer 401 without a session or token; a foreign origin write 403; MCP metadata 200;
the global CSP has `frame-src 'self' https://www.youtube-nocookie.com` and `frame-ancestors 'none'`;
https://orbit.wellrox.com/ answers 307 to the exe.dev login. The signed-in Apps page and a real sign-in were **not** tested
on production from this VM: they need a person's login.

Gateway: installed per `docs/apps.md`, tunnel section, step 2. Local checks on `127.0.0.1:8400`: `/oauth/jwks` 200,
`POST /oauth/token` 401 with a wrong client, and 404 for `/login`, `/`, `/api/v1/auth/me`, `/oauth/authorize`, traversal,
encoded paths and `GET /oauth/token`. `cloudflared` is on hold: not installed, no tunnel. The Products app is not added yet.

Rollback (an older binary cannot read a version 67 database; preserve the current database first; work after the snapshot
is lost):

```sh
sudo systemctl disable --now orbit-oidc-gateway
sudo systemctl stop orbit-agent orbit
sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-aedbc85 backup verify 01a18c97-844c-7482-998d-618f250d27be && /usr/local/bin/orbit-aedbc85 backup restore 01a18c97-844c-7482-998d-618f250d27be'
sudo install -m 755 /usr/local/bin/orbit-aedbc85 /usr/local/bin/orbit
sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-6808e1b.tgz
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
```

### October 30, 2026 upgrade: agent model and thinking level

Merged the `agent-model-setting` branch (`c65ffc4`, which contains `main` up to `c4ca69c`) into `main` (merge
`aedbc85`); the merged tree equals the tested branch. Tests on this code: sidecar typecheck and 24 tests; web
typecheck, lint and build; mobile typecheck and 135 tests; `agent_api` (13), `app_smoke` (18) and `backup` (21) on
SQLite and Postgres; platform `database` (43) on Postgres. Platform `database` on SQLite is intermittent:
`page_search_migration_backfills_and_tracks_pages` fails with "malformed inverted index for FTS5 table
main.page_search" (the `PRAGMA integrity_check` false positive in `.ai/lessons.md`). `c4ca69c` shows the same failure
(run 7 of a series stopped at the first failure; another session measured 3 of 15 full runs on both trees), so it does
not come from this change; the test is not changed yet. The iOS bundle was not checked (no Metro server). The image
`orbit:aedbc85` built with the limited builder and `deploy/smoke.sh` passed.

Rehearsed migration 66 (`0066_agent_model_settings.sql`, one new table) on a copy in
`/var/lib/orbit/rehearsal-aedbc85-T3zWkq` (deleted afterwards). Stopped `orbit-agent` and `orbit`. Counts before:
schema 65, 6 users, 1 workspace, 2 projects, 15 tasks, 18 pages, 15 attachment blobs, 370 chat messages, 35 agent
inputs, 3 ChatGPT connections, 0 agent memories, 0 integrations, 0 team instructions. Created and verified snapshot
`01a18c0f-83d8-72b8-b885-2b44c623a630` (`/var/backups/orbit/snapshots/01a18c0f-83d8-72b8-b885-2b44c623a630`) with the old CLI. Saved the sidecar data as
`/var/backups/orbit-agent/orbit-agent-before-aedbc85.tgz` and the old sidecar code as
`/opt/orbit-agent.before-aedbc85`; the old executable is `/usr/local/bin/orbit-29c694c` and the new one is also kept
as `/usr/local/bin/orbit-aedbc85`. Applied migration 66 live: schema version 66, quick-check and foreign-key checks
pass, all counts unchanged. Installed the new sidecar `src`; `package.json`, `package-lock.json` and `tsconfig.json`
did not change. `/etc/orbit/orbit.env`, `/etc/orbit/agent.env` and nginx did not change.

Incident during this deploy: the sidecar did not start for about 30 seconds after the switch (09:06 to 09:07 UTC)
because `/opt/orbit-agent/node_modules` was empty. At 07:32 UTC, `rm -rf` on a path with a trailing slash went through a
worktree symlink to that directory; the running sidecar did not show it until it restarted. Restored from
`/opt/orbit-agent.before-29c694c` (same lockfile, also identical to the `before-51c3035` copy), in `/opt/orbit-agent` and
in `/opt/orbit-agent.before-aedbc85`. No input was queued in that time.

Verified: `orbit`, `orbit-agent` and nginx are active; readiness (all checks); revision `aedbc85`; sidecar health
`mock: false`; sidecar `/models` lists 7 models with default `gpt-6.1-sol` and answers 401 without the secret; the
internal inputs route gives `model: null, thinking: null` with the secret and 401 without it;
`/api/v1/admin/agent/model` answers 401 without a session and 403 for a foreign origin; `agent_model_settings` is empty,
so runs keep `gpt-6.1-sol` with no thinking level; https://orbit.wellrox.com/ answers 307 to the exe.dev login. Admin → AI
agent was checked signed in, in a browser on loopback, against a copy of the migrated production database served by the
new binary with the mock sidecar (a session was added and the push subscriptions removed in the copy only; the copy was
deleted): the page shows "Default (GPT-6.1 Sol)" and thinking "Default", and a mock run answered `Model: gpt-6.1-sol,
thinking: default`. A real agent run on production and the live signed-in page were **not** tested from this VM: they
need a person's login and ChatGPT account. Stripe was not configured.

Rollback (an older binary cannot read a version 66 database; preserve the current database first; work after the
snapshot is lost):

```sh
sudo systemctl stop orbit-agent orbit
sudo -u orbit bash -c 'set -a; source /etc/orbit/orbit.env; set +a; /usr/local/bin/orbit-29c694c backup verify 01a18c0f-83d8-72b8-b885-2b44c623a630 && /usr/local/bin/orbit-29c694c backup restore 01a18c0f-83d8-72b8-b885-2b44c623a630'
sudo install -m 755 /usr/local/bin/orbit-29c694c /usr/local/bin/orbit
sudo rm -rf /opt/orbit-agent && sudo cp -a /opt/orbit-agent.before-aedbc85 /opt/orbit-agent
sudo rm -rf /var/lib/orbit-agent && sudo tar -C /var/lib -xzf /var/backups/orbit-agent/orbit-agent-before-aedbc85.tgz
sudo systemctl start orbit && sleep 4 && sudo systemctl start orbit-agent
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8000/health/ready
```

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
