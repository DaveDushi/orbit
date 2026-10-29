# Orbit agent instructions

Read [`.ai/README.md`](.ai/README.md) first: it lists the project guides in order.

## Shared build VM resource controls

On the configured Linux VM (`/etc/systemd/system/orbit-builds.slice` exists), run heavy native commands
through `orbit-build`, for example `orbit-build cargo test --workspace` or `orbit-build just build`.
Never bypass the guard, increase Cargo jobs, run parallel heavy builds, or build in Jean's cgroup.
The shared lock serializes builds across users/worktrees; the separate slice has a 3 GiB soft / 4 GiB hard
memory budget. Cargo's installed rustc wrapper is a backstop, not a substitute for guarding the whole build.
Use `docker build` / `docker buildx build --builder=orbit-limited` through the installed Docker guard, or
`orbit-build /usr/bin/docker buildx build --builder=orbit-limited --load ...`.
Do not use alternate builders, `/usr/bin/docker build`, or Compose `--build`: build the image with the
limited builder first, then run Compose without `--build`. Do not restart Jean to manage build memory.
See [the development guide](.ai/DEVELOPMENT.md#shared-build-vm-resource-limits) and
[the VM operator runbook](deploy/README.md#shared-build-vm-operator-setup).
Other developer machines keep the normal portable recipes; `just limited …` is an explicit VM-only helper.

## Apps

The repo root is one Bun workspace: `apps/web` (Vite) and `apps/mobile` (Expo, see [`apps/mobile/AGENTS.md`](apps/mobile/AGENTS.md)).
Run `bun install` at the root. Both apps share one hoisted `node_modules`, so they must use the same `react` and
`react-dom` version. Find a package's files with module resolution, not a fixed `node_modules/…` path.

**Always port web changes to mobile.** When you change a feature, a behavior or a style in `apps/web`, make the
same change in `apps/mobile` in the same task, without waiting for the user to ask. A web task is not done until the
mobile port is done and checked (`bunx tsc --noEmit -p .` and `bun test src` in `apps/mobile`, and the iOS bundle
builds on the running Metro server). Skip a change only when mobile does not have the problem it fixes (for example,
a keyboard-only or desktop-only feature, or a fix for a component that mobile does not use). Then tell the user
which changes you did not port, and why. When mobile lacks a screen that the change needs, say so and offer to port
that screen.

## Databases: SQLite and Postgres

Orbit runs on SQLite (default, `ORBIT_DATABASE_PATH`) or Postgres (`ORBIT_DATABASE_URL`).
Every change must work on both.

- **SQL:** all queries use the sqlx `Any` driver. Write placeholders as `$1, $2, …` (never `?`). Build dynamic SQL
  with `orbit_platform::SqlBuilder`, not `sqlx::QueryBuilder`.
- **Rows and types:** import `orbit_platform::Row`, not `sqlx::Row`. Flags are 0/1 `BIGINT` columns: bind `i64`
  (never `bool`, `u32` or `u64`), and decode boolean results such as `EXISTS (…)` as `orbit_platform::Flag`.
- **Portable SQL:** use `ON CONFLICT` (not `INSERT OR IGNORE`), `IS [NOT] DISTINCT FROM` (not `IS $n`),
  `CAST(SUM(…) AS BIGINT)`, and `lower(…)` for case-insensitive text. When SQL must differ (search, JSON, time),
  branch on `database.backend()` (`Backend::Sqlite` / `Backend::Postgres`).
- **Transactions:** writes use `immediate_transaction()`. It serializes writers on both databases. On Postgres, an
  error ends the whole transaction, so do not catch a database error and continue in the same transaction.
- **Migrations:** a new SQLite migration in `apps/server/migrations/` also needs the same change in the Postgres
  schema in `apps/server/migrations/postgres/` (see `.ai/lessons.md` for the other places to update).
- **Tests:** run the tests that cover your change on both databases:
  `cargo test -p orbit-server --test <name>`, then the same with
  `ORBIT_TEST_POSTGRES_URL=postgres://orbit:orbit@127.0.0.1:15432/orbit`. Tests that only make sense on SQLite
  start with `if orbit_platform::testing_postgres() { return; }` and a one-line reason.
- **Postgres setup:** the database must use `LOCALE_PROVIDER builtin BUILTIN_LOCALE 'C.UTF-8' LOCALE 'C.UTF-8'`
  (Orbit refuses other databases), and the server needs `ORBIT_APP_KEY`. S3 for files and backups stays optional
  and is set in Admin → Storage. Operator details: [`docs/operations.md`](docs/operations.md).
