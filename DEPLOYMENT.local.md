# This VM's Orbit deployment

Private team trial; upstream explicitly labels Orbit alpha, not production-ready.

- Canonical URL: https://orbit.wellrox.com/
- The original exe.dev hostname still reaches nginx, but browser writes must originate from the canonical URL.
- Source: DaveDushi/orbit, commit 599976c4c12a895da726439a2daca4f535742733.
- Binary extracted from the upstream coollabsio/orbit image for the identical full commit.
- Image digest: sha256:60fc44c7cb20fd0592d4c9f8b17a621416a40e57b4e781842b991c3f0218e097
- Executable: /usr/local/bin/orbit; unprivileged system user: orbit.
- Config: /etc/orbit/orbit.toml (production mode, closed registration).
- SQLite, attachments, encryption key: /var/lib/orbit/ (mode 700).
- Local backups: /var/backups/orbit/ (mode 700).
- systemd: orbit.service and nginx.service, enabled at boot.
- nginx listens on 8000; Orbit listens only on loopback port 8080.
- exe.dev terminates TLS and authenticates private web access; nginx forwards its HTTPS transport header. Orbit trusts only loopback.
- nginx access logging is disabled for this app to avoid logging invitation/setup tokens.

## Custom-domain configuration

Orbit's `/etc/orbit/orbit.toml` sets `[http].public_origin` to
`https://orbit.wellrox.com` (no trailing slash). This controls the browser
origin policy as well as generated application links. Changing DNS alone does
not update it: a mismatched origin makes sign-in fail with
`The request origin is not allowed.`

nginx's `/etc/nginx/conf.d/orbit.conf` names both the custom domain and original
exe.dev hostname. After changing the domain, validate the application config
and nginx config, restart `orbit.service`, and reload `nginx.service`.
Keep production origin checks and the existing trusted-proxy policy enabled.
The previous application config is saved locally at
`/etc/orbit/orbit.toml.before-custom-domain`.

Verified through the local nginx proxy with the HTTPS forwarding header:
readiness is healthy; a login POST with the custom origin reaches request-body
validation (400 for an empty body), while an unrelated origin still receives
403 `origin_forbidden`. Both services are active. This verifies origin handling,
not a successful authenticated login through the public domain.

## First login and team access

The one-time setup link is stored in /home/exedev/orbit-setup-link.txt (mode 600).
Open it to create the root account, then create your workspace and project.
Do not share this setup link with teammates.

Two access layers apply:
1. Grant teammates exe.dev **Web** access (`share add victor-vs-isolani team`, or individual emails). Do not grant Root access just to use Orbit.
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
decrypt saved secrets. A local mode-600 copy is /var/backups/orbit/app.key.

Manual CLI backup (the CLI requires the live app stopped):

```sh
sudo systemctl stop orbit
sudo -u orbit /usr/local/bin/orbit --config /etc/orbit/orbit.toml backup create
sudo systemctl start orbit
sudo -u orbit /usr/local/bin/orbit --config /etc/orbit/orbit.toml backup list
sudo -u orbit /usr/local/bin/orbit --config /etc/orbit/orbit.toml backup verify <id>
```

If creating a backup fails, still start Orbit again after investigating.
Before upgrades, create and verify a backup and retain the current binary.
Never run two Orbit processes against the same SQLite database. Restore only
with Orbit stopped; see docs/operations.md and .ai/DEVELOPMENT.md.

## Expired setup link

While setup is incomplete:

```sh
sudo systemctl stop orbit
sudo -u orbit /usr/local/bin/orbit --config /etc/orbit/orbit.toml setup-token rotate > /home/exedev/orbit-setup-link.txt
chmod 600 /home/exedev/orbit-setup-link.txt
sudo systemctl start orbit
```

## Verification performed

Upstream production image smoke suite passed: non-root execution, HTTPS
boundary, embedded frontend, setup required, no seeded login, production
seed/reset rejection, and restart readiness. Deployed service readiness,
frontend assets, security headers, origin rejection and local backup verification
passed. Private public-URL browser access could not be tested from this VM;
the owner should complete that check when opening the setup link. Multi-user
collaboration, SMTP and off-host recovery still require configuration/testing.
