# Deploy Orbit

## Shared build VM operator setup

This is opt-in Linux/systemd VM infrastructure, not a requirement for ordinary developers or CI.
It isolates heavy Orbit builds from Jean and other interactive services. The shared
`orbit-builds.slice` has `MemoryHigh=3G` (soft pressure), `MemoryMax=4G` (aggregate hard ceiling),
and low CPU/IO weights. All guarded commands share one host-wide flock across users and worktrees.
Do not put Jean in this slice or restart it to recover build memory.

From the repository root, as an operator with sudo:

```sh
sudo install -m 0755 scripts/build-limited.sh /usr/local/bin/orbit-build
sudo install -m 0755 scripts/docker-build-guard.sh /usr/local/bin/docker
sudo install -m 0644 deploy/orbit-builds.slice /etc/systemd/system/orbit-builds.slice
sudo install -d -o root -g root -m 0755 /var/lib/orbit-builds
sudo touch /var/lib/orbit-builds/build.lock
sudo chown root:root /var/lib/orbit-builds/build.lock
sudo chmod 0666 /var/lib/orbit-builds/build.lock
sudo systemctl daemon-reload
sudo systemctl start orbit-builds.slice
```

Never remove or replace the lock while any guard is running. Its root-owned parent directory prevents
unprivileged replacement; mode 0666 allows both root and exedev to open the same inode.
The helper requires Bash, util-linux `flock`, systemd supporting `--expand-environment=no`, and
passwordless `sudo -E systemd-run` for non-root callers. The command drops back to its original numeric
UID/GID before execution, retaining cwd, stdio, environment, and exit status. It forwards normal
termination signals while retaining the lock until the child exits. These are cooperative resource
controls for trusted VM users, not a security sandbox. Do not grant untrusted users unrestricted sudo.
Missing setup or scope creation failures fail closed; they never silently run an unguarded build.

Merge (do not overwrite unrelated existing configuration) the following into **both**
`/root/.cargo/config.toml` and `/home/exedev/.cargo/config.toml`:

```toml
[build]
jobs = 1
rustc-wrapper = "/usr/local/bin/orbit-build"
```

Keep `/usr/local/bin` ahead of `/usr/bin` in both users' `PATH`. The installed Cargo wrapper catches
raw Cargo compiler invocations, including probes, but wrap whole native build/check/test recipes with
`orbit-build` as well so Cargo, linkers, and test processes share the same scope/lock. The reentry
marker is set only inside that scope, so nested Cargo/rustc wrappers do not deadlock. Do not set
`ORBIT_DEV_BUILD_GUARD_ACTIVE` yourself or override the jobs/wrapper configuration.
With raw Cargo, the small Cargo parent remains outside the slice and individual compiler invocations
are serialized; `orbit-build cargo test …` also caps the test executables and the whole Cargo process.

### Isolated Docker worker

Limiting the Docker client does **not** limit daemon-side compilers. Use a dedicated, explicitly
managed BuildKit container and a Buildx **remote** builder. Do not rely on the docker-container driver's
`cgroup-parent` option on a host using Docker's systemd cgroup driver.

```sh
sudo install -m 0644 deploy/buildkitd.toml /etc/orbit-buildkit.toml
sudo install -m 0644 deploy/orbit-buildkit.service /etc/systemd/system/orbit-buildkit.service
sudo /usr/bin/docker volume create orbit-buildkit-state
sudo /usr/bin/docker create --name orbit-buildkit --restart=no --privileged \
  --cgroup-parent=orbit-builds.slice --memory=4g --memory-swap=4g \
  -v orbit-buildkit-state:/var/lib/buildkit \
  -v /etc/orbit-buildkit.toml:/etc/buildkit/buildkitd.toml:ro \
  moby/buildkit:v0.26.2
sudo systemctl daemon-reload
sudo systemctl enable --now orbit-buildkit.service
sudo -H /usr/bin/docker buildx create --name orbit-limited --driver remote \
  docker-container://orbit-buildkit
sudo -H -u exedev /usr/bin/docker buildx create --name orbit-limited --driver remote \
  docker-container://orbit-buildkit
sudo -H /usr/bin/docker buildx inspect orbit-limited --bootstrap
sudo -H -u exedev /usr/bin/docker buildx inspect orbit-limited --bootstrap
```

Creation commands are one-time steps: inspect existing containers/builders instead of recreating
them blindly. The privileged worker is dedicated to trusted local builds; expose no BuildKit TCP port.
Systemd owns restart behavior (`--restart=no` on the container). `--memory-swap=4g` equals its 4 GiB
memory limit, disabling additional swap allowance. BuildKit schedules one OCI build operation at a
time. Its GC settings reserve 512 MB, target at most 2 GB cache, and aim for 2 GB free disk; these are
GC targets, **not hard disk quotas**. The first build has a cold, independent cache and may take longer.

After changing BuildKit config, restart `orbit-buildkit.service`. Container memory/cgroup/image changes
require stopping the service and deliberately recreating the container with the same state volume.
Do not remove `orbit-buildkit-state` unless intentionally discarding the build cache.

### Run and verify

```sh
orbit-build cargo test --workspace
just limited cargo build --release --workspace
docker build --build-arg ORBIT_BUILD_REVISION="$(git rev-parse HEAD)" -t orbit:local .
docker buildx build --builder=orbit-limited --load -t orbit:local .
# Explicit equivalent, without relying on the installed Docker PATH wrapper:
orbit-build /usr/bin/docker buildx build --builder=orbit-limited --load -t orbit:local .

systemctl show orbit-builds.slice -p ControlGroup -p MemoryHigh -p MemoryMax -p MemoryCurrent
sudo /usr/bin/docker inspect orbit-buildkit \
  --format '{{.HostConfig.CgroupParent}} {{.HostConfig.Memory}} {{.HostConfig.MemorySwap}}'
pid=$(sudo /usr/bin/docker inspect orbit-buildkit --format '{{.State.Pid}}')
sudo cat "/proc/$pid/cgroup"
bash scripts/test-build-guards.sh
```

The worker's actual cgroup must be below `/orbit.slice/orbit-builds.slice/` (normally a
`docker-…scope` child), not just the Docker client. Inspect a running native compiler's
`/proc/<pid>/cgroup` too. Verify both root and exedev, and confirm simultaneous guarded commands
serialize. The Docker PATH wrapper routes `docker build` and `docker buildx build` through the
same lock and forces `orbit-limited`; conflicting explicit `--builder` values are rejected.
`docker build` adds `--load` unless explicit `--output`/`-o`, `--push`, or `--load` options are present;
`docker buildx build` retains your export options unchanged. Other Docker commands pass through unchanged.
The unprivileged mock suite requires Bash and Python 3; it does not access Docker or systemd.

Do not bypass the wrapper with `/usr/bin/docker build`, alternate builders, Docker global-option
forms such as `docker --context … build`, or Compose builds. For Compose, first build/tag the image
using the limited builder, then run `docker compose up` **without** `--build`. Wrapping Compose alone
does not guarantee that its daemon-side worker is isolated.

## Docker with HTTPS

Requirements: Docker Compose, a DNS hostname pointing to this host, and inbound TCP ports 80/443 for Caddy certificate issuance. The example uses a dedicated `172.30.0.0/24` Docker network. Change both its proxy address and the trusted-proxy setting if that subnet conflicts with your host.

```sh
cp deploy/.env.example deploy/.env
# Edit deploy/.env: set ORBIT_DOMAIN (without https://) and ORBIT_APP_KEY (openssl rand -base64 32).
docker compose --env-file deploy/.env -f deploy/compose.yaml up -d --build
```

Orbit reads its configuration only from `ORBIT_*` environment variables. The image sets production mode, the bind address, the data paths and `ORBIT_TRUSTED_PROXIES=0.0.0.0/0,::/0`. A container needs only `ORBIT_PUBLIC_ORIGIN` and `ORBIT_APP_KEY`. The image trusts forwarding headers from every address, so only the reverse proxy may reach port 8080. If anything else can reach it, set `ORBIT_TRUSTED_PROXIES` to the proxy network. To use Postgres, also set `ORBIT_DATABASE_URL`. Keep a copy of `ORBIT_APP_KEY` outside the server: without it, Orbit cannot read the secrets it saved. For all variables, see [Configuration](../docs/operations.md#configuration).

The application port is not published. Only Caddy can reach it through the deployment network. Caddy forwards WebSocket upgrades automatically. The application runs as UID 65532 with a read-only root filesystem and separate named volumes for SQLite/attachments and backups.

On first startup, retrieve the one-time setup URL locally:

```sh
docker compose --env-file deploy/.env -f deploy/compose.yaml logs orbit
```

Treat that URL as a secret. Open it at your HTTPS domain and create the root account (the only account that can open Admin). Orbit then asks for your first workspace and project. Production has no seeded login or automatic setup bypass. Never copy the development database into production. The `seed`, `migrate run --seed`, and reset commands reject production mode. Dev seeding remains idempotent and available through `just dev`.

For Tailscale-only hosting, use an existing Tailscale HTTPS proxy instead of the Caddy service. Configure the actual HTTPS public origin and trust only the exact proxy address. Do not set production to development mode to work around HTTPS/origin checks.

## Single executable

The image contains a statically linked Linux executable with all frontend assets embedded. It needs no Node, Bun, Rust installation, or separate web directory at runtime.

```sh
docker build -t orbit:local .
container=$(docker create orbit:local)
docker cp "$container:/orbit" ./orbit
docker rm -v "$container"
chmod +x orbit
./orbit --help
```

Set the [`ORBIT_*` environment variables](../docs/operations.md#configuration) for the environment, the origin, the trusted proxies and the writable storage paths, then run:

```sh
(umask 077 && openssl rand -base64 32 > /etc/orbit/app-key)  # once; keep a copy of this file off the server
export ORBIT_ENV=production \
  ORBIT_PUBLIC_ORIGIN=https://orbit.example.com \
  ORBIT_TRUSTED_PROXIES=127.0.0.1/32,::1/128 \
  ORBIT_APP_KEY_FILE=/etc/orbit/app-key \
  ORBIT_DATABASE_PATH=/var/lib/orbit/orbit.sqlite \
  ORBIT_ATTACHMENTS_PATH=/var/lib/orbit/attachments \
  ORBIT_BACKUPS_PATH=/var/backups/orbit
./orbit config check
./orbit serve
```

Run under an unprivileged service account and an HTTPS reverse proxy. The binary architecture matches the Docker build platform. The latest commit on `main` publishes one image manifest for `linux/amd64` and `linux/arm64` to `ghcr.io/<owner>/<repository>:<seven-character-commit-sha>`. A newer commit cancels an older image build. Each image is built and smoke-tested on a native runner; the image workflow does not use QEMU. The workflow does not create GitHub Releases or downloadable binaries. Local builds do not publish anything.

## Backups and upgrades

Backups are created from **Admin → Backups** while Orbit runs, or automatically (hourly or daily) when the root user turns that on in **Admin → Storage**; with backups in S3 they are uploaded to the bucket. Snapshot retention keeps up to 24 snapshots from the last day, plus seven daily and four weekly buckets when enough backups exist. A backup volume on the same machine is not disaster recovery: copy it off-host, protect access, and regularly test restore into separate storage.

On SQLite the running server owns the database file, so the CLI `backup create` works only while the app is stopped. On Postgres (`ORBIT_DATABASE_URL`) it also works while servers run. `backup list` works at any time.

```sh
docker compose --env-file deploy/.env -f deploy/compose.yaml stop orbit
docker compose --env-file deploy/.env -f deploy/compose.yaml run --rm orbit backup create
docker compose --env-file deploy/.env -f deploy/compose.yaml start orbit
docker compose --env-file deploy/.env -f deploy/compose.yaml exec orbit /orbit backup list
```

Before upgrading, create and verify a backup. Keep the old image and matching backup; older binaries reject newer database schemas. On SQLite, never run two app processes against one database. On Postgres several servers can share one database; see [Orbit Cloud](../docs/operations.md#orbit-cloud-one-postgres-several-orbit-servers). Stop every app process before restoring. See [operations](../docs/operations.md) and [restore commands](../.ai/DEVELOPMENT.md#migration-backup-and-restore).

## Live task collaboration

Each tab opens a session-authenticated WebSocket for its active workspace. Every successful workspace audit mutation records a notification in the same database transaction. The server coalesces ordered invalidations to a high-water sequence and checks for new events every second. No resource content is sent over the socket; clients refresh through authorized HTTP endpoints.

Reconnects resume from the last successfully refreshed sequence. Missing, expired, or invalid cursors trigger a full workspace refresh. Event retention is seven days; persistent counters never reset when old events are pruned. Revoked/expired sessions, suspended users, and removed memberships are checked before delivery and at least once per second. Chat, presence and typing use the separate live socket, and page co-editing its own socket per page.

Focused text drafts and in-flight writes defer remote refresh until editing finishes. Version conflicts use the existing conflict flow rather than silently overwriting another user's write. A connection banner appears while live updates are unavailable; reconnection uses capped backoff. Without SMTP (Admin → Settings → Email), invitation and recovery links are shared manually.

## Acceptance checks

- Open `https://<your-domain>/health/ready` through Caddy; expect readiness success.
- Verify the fresh instance requires setup and rejects `test@example.com / password`.
- Use two browser profiles: create/edit a task, move it on the board, add a comment, and delete/restore it. The other profile should update without reloading, normally within about one second plus request latency.
- Disconnect/reconnect a browser, then confirm it catches up. Remove its membership or revoke its session and confirm access is lost.
- Restart the container and verify tasks and attachments persist.
- Restore a verified backup into separate stopped-instance storage and verify its tasks and attachments before depending on the deployment.

A repeatable two-browser smoke test against an **existing development server** is available without starting another server:

```sh
cd apps/web
ORBIT_SMOKE_URL=http://127.0.0.1:18888 node scripts/smoke-collaboration.cjs
```

It signs in with the seeded dev account, creates a uniquely named task, checks another browser receives it and catches up after a disconnect, then moves the smoke task to trash. Set `PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH` if using a separately installed Chromium.

Test a built production image in isolated temporary storage:

```sh
bash deploy/smoke.sh orbit:local
```

This uses a dynamically assigned localhost port and a simulated trusted HTTPS proxy header. It checks non-root execution, HTTPS enforcement, embedded assets, first-run setup, rejected development credentials/seed commands, and restart readiness. It does not replace testing real TLS and backups on the deployment host.
