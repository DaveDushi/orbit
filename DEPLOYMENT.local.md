# This VM's Orbit deployment

Private team trial; upstream explicitly labels Orbit alpha, not production-ready.

- Canonical URL: https://orbit.wellrox.com/
- The exe.dev hostname `wellrox-orbit.exe.xyz` still reaches nginx, but browser writes must originate from the canonical URL.
- Source: DaveDushi/orbit, local merge commit 6af5e606b79b8f6ff6201dda1e64fc42434edbb5
  (fork commit bdaaecf plus these deployment notes).
- Binary extracted from the image built locally on this VM from that commit
  (`orbit:6af5e60`); the fork commit has no published upstream image.
- Image ID: sha256:5c7fce9295c0c31f62414b36c6c468aff5963f582fbd19be8a0c07c0d4cfe472
- Executable: /usr/local/bin/orbit; unprivileged system user: orbit.
- Config: /etc/orbit/orbit.env (production mode); registration remains closed in instance settings.
- Environment-only startup: /etc/systemd/system/orbit.service.d/environment.conf.
- Legacy TOML is retained at /etc/orbit/orbit.toml for the old binary and rollback only.
- SQLite, attachments, encryption key: /var/lib/orbit/ (mode 700).
- Local backups: /var/backups/orbit/ (mode 700).
- systemd: orbit.service and nginx.service, enabled at boot.
- nginx listens on 8000; Orbit listens only on loopback port 8080.
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

## Health and logs

```sh
sudo systemctl status orbit nginx
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
