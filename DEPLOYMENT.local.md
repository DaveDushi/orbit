# This VM's Orbit deployment

Private team trial; upstream explicitly labels Orbit alpha, not production-ready.

- Canonical URL: https://orbit.wellrox.com/
- The exe.dev hostname `wellrox-orbit.exe.xyz` still reaches nginx, but browser writes must originate from the canonical URL.
- Source: DaveDushi/orbit, `main` at 9f790ab389e784588fc81b7df750694bdeb03077
  (Donna's interactive cards: live cards, hidden card clicks, phone cards, schema 86, November 9, 2026).
- Binary extracted from the image built locally on this VM from that commit
  (`orbit:9f790ab`); fork commits have no published upstream image.
- Image ID: sha256:533b28f8bff4a4c11dd31a8024c2acac46c26d678ce1796c808a81f6c8aef7cc
- Team agent sidecar code (`/opt/orbit-agent`): commit 9f790ab.
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
