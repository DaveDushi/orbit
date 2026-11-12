# Lessons

## Migration history
- Do not squash or edit a migration after a local database has applied it. Orbit checks both version and checksum at startup. If an uncommitted migration must be consolidated, compare the old and new schemas and back up the database before reconciling its migration records; do not reset user data to make the dev server start.
- SQLite table rebuilds (to change a CHECK): the runner holds a transaction with `foreign_keys=ON`, so `PRAGMA foreign_keys=OFF` is a no-op and `ALTER TABLE … RENAME` fails on triggers that name the table. Copy rows aside, `DROP`, re-`CREATE` under the same name, re-insert under `PRAGMA defer_foreign_keys=ON`, then recreate the table's own indexes and triggers (see `0021`).
- `scripts/dev-server-watch.sh` reruns `migrate run --seed` against `data/orbit.sqlite` on every source change, so a new migration is applied to the dev DB (and its checksum frozen) the moment it compiles. Get the SQL right in tests (`embedded_through`) before any `cargo build` of the server while the watcher runs (0023 was applied this way on 2026-09-25).
- A new migration also needs: its entry in `crates/platform/src/db/migrate.rs` (else every query on the new table returns 500), `SUPPORTED_SCHEMA_VERSION` in `crates/platform/src/backup.rs` (restore refuses newer snapshots), and the version number in `crates/platform/tests/{database,backup}.rs` and `apps/server/tests/app_smoke.rs`. A new route also needs the path and operation counts in `apps/server/tests/openapi_contract.rs` and `just api`.
- A rebuilt table becomes the *newest* FK child, and SQLite runs parent-delete cascades newest-child first. A `RESTRICT` reference to the rebuilt table (e.g. `tasks.status_id`) can then block a cascade that used to work. Hard deletes must delete the restricting children explicitly first.

## Build VM tooling
- The VM's system Node is 18, but Vite/rolldown and Playwright need Node 20+. Build the web app with `bun --bun run build` (also under `orbit-build`) and run Playwright scripts with `bun`, not `node`. The server embeds `apps/web/dist`, so a fresh worktree needs this web build before any `cargo test -p orbit-server`.

## Two database backends (SQLite + Postgres)
- All SQL goes through sqlx `Any`: write `$1, $2, …` (SQLite binds them by number too), import `orbit_platform::Row` (not `sqlx::Row`) so 0/1 flags decode as `bool`, decode boolean scalars as `Flag`, bind `i64` (never `bool`/`u32`), and build dynamic SQL with `SqlBuilder`. SQLite silently ignores extra bound values, so clashing `$N` numbers in `format!`-glued fragments only fail on Postgres.
- A new SQLite migration also needs its change in the Postgres schema (`apps/server/migrations/postgres/`) and that file's entry in `runner.postgres` in `crates/platform/src/db/migrate.rs`.
- Run the whole suite on Postgres with `ORBIT_TEST_POSTGRES_URL=postgres://orbit:orbit@127.0.0.1:15432/orbit`; each test database maps from its SQLite path.
- No Postgres listens on 15432 after a reboot unless the persistent dev one runs: `orbit-dev-pg` (volume `orbit-dev-pg-data`, `--restart unless-stopped`, databases `orbit` for tests and `orbit_dev` for `just dev`). A `--rm` container loses all data on reboot (the dev data was lost this way on 2026-10-24). For the full test suite use `-c max_connections=300`, at least 1 GB of memory and `--shm-size=1g` (the 64 MB default `/dev/shm` gives error 53100 in `collab_api`), or tests fail mid-suite. With a 64 MB `/dev/shm`, run the server suite with `-- --test-threads=4`, and rerun the failed binaries before you suspect the code.

## Team agent transcripts are shared per thread
- One Pi conversation serves a whole chat thread, so anything a run reads (tool results, prompt sections) stays in the transcript for the next driver of that thread. Data that only one person may see (their private agent memory) is read only where that person is alone with the agent (a DM of two); see `.ai/agent-design.md`, "Memory", M2.

## New agent tools
- A new sidecar tool also needs its row in `TOOL_CATALOG` and its routes in `route_need` (`apps/server/src/agent/profiles.rs`): custom agents' sessions are refused every route that no tool names there (fail closed). `apps/agent/test/profiles.test.ts` fails when the catalog and the sidecar's tools differ (2026-11-11).
- A routine's test run is a dry run enforced by Orbit (`agent::scope::dry_allows`): a dry session reaches only reads and the agent tool routes (`/agent-tools/call`, `/drive`, `/scheduled`). A new agent route that writes on POST must be refused there, or check `Option<Extension<scope::DryRun>>` and answer `{"status": "dry_run", "action": …}` like those routes (2026-11-11).

## Guests (chat-only role)
- `membership::actor` returns `None` for a guest, so every repository that uses it refuses guests; only chat uses `chat_actor`. A route with its own membership SQL (`FROM memberships`) must exclude `role = 'guest'` itself. `tests/guest_access.rs` sweeps only the OpenAPI operations: a route outside the contract (agent, apps, internal) needs its own guest test (`agent_api.rs`, `apps_api.rs`).

## Settings and internal callers
- Orbit refuses to start on an unknown `ORBIT_*` environment variable. A new setting needs its key in `crates/platform/src/config.rs` (or an ignored prefix there, as `ORBIT_AGENT_` for the agent), or the server will not boot (2026-10-29).
- The origin policy refuses a write without `Origin` unless it carries `Authorization: Bearer` (and no cookie). A server-to-server caller (a sidecar, a script) sends its secret as a bearer token. Route tests that skip `HttpPlatformLayer` do not see this: check such callers against a running server; a route whose credential is in the path (incoming hooks, bot reply URLs) needs its prefix in `OriginPolicy::permits` (`crates/platform/src/http/security.rs`).

## Cron expressions
- The `cron` crate (0.15) counts weekdays from Sunday = 1 (`1-5` is Sunday–Thursday), and wants a seconds field first. People and models write standard 5-field cron (0 or 7 = Sunday, `1-5` = Monday–Friday): translate the weekday field to names first, as `scheduled::schedule` does (2026-11-11).

## Several servers on one Postgres database
- Any process-local state (maps, locks, counters, caches, "running" flags, background loops) is per server. If it must hold for every server, keep it in the database (a serialized `immediate_transaction`, `Database::try_singleton`) or send it through `crate::cluster`. Test such changes with two `App`s on one database (`apps/server/tests/shared_postgres`), and prove the test fails without the fix.
- `immediate_transaction()` takes one advisory lock for the whole Postgres database, so every writer of the cluster waits for it. A hot counter that needs no read-then-write logic uses one atomic statement on the pool instead (`INSERT … ON CONFLICT … DO UPDATE SET n = t.n + 1 RETURNING n`, as `rate_limit_windows` does); keep per-request state of very frequent requests in memory and document that it is per server.

## Write transactions and background services
- Do slow CPU work (Markdown merges, Yjs reconciles of big docs) before `immediate_transaction()`, on `spawn_blocking`, and only check and persist inside it: the transaction holds every writer of the database, and a CPU-bound task also stalls the other tasks of its tokio worker. Markdown doc writes do this under the collab room lock and check the revision and the log again in the transaction (2026-11-05).
- A background loop (job worker, scheduler, attachment reconciliation, integrity job) must treat `orbit_platform::is_transient_lock_error` as "retry on the next round", never as fatal: any service error stops the whole server (`AppError::Background`), and SQLite returns "database is locked" after the 5 s busy timeout (2026-11-05).

## Full Rust test run and disk space
- `cargo test -p orbit-platform -p orbit-server` with the default debug info makes about 75 test binaries of up to 800 MB each (over 45 GB): it filled this VM's disk on 2026-11-11, and the link failed with "Bus error". Run full suites with `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0` (about 8 GB of `target`), and remove `target` afterwards.

## Jean run environments share ports
- `jean.json` gives every worktree the same ports (web 18888, API 18080), and `just dev` uses `--strictPort`. Before you trust a run environment, check who owns the ports (`ss -ltnp | grep -E ':18080|:18888'`, then `readlink /proc/<pid>/cwd`): another worktree's Vite server can answer on 18888 with its own code (2026-10-20).

## SQLite 3.46.0 (bundled by libsqlite3-sys 0.30) FTS5 integrity false positive
- `PRAGMA integrity_check` reports "malformed inverted index for FTS5 table …" after a row of an FTS5 table is UPDATEd twice
  (or after `'rebuild'`), although queries are right and FTS5's own `INSERT INTO t(t) VALUES('integrity-check')` passes
  (Python's SQLite 3.46.1 also says ok). It made the weekly integrity job (which runs at App start in tests) stop the
  server once `page_search` rows were updated. `IntegrityService::full` now confirms such findings with FTS5's own check
  (2026-09-25). Check new virtual tables against `integrity_check` with the bundled SQLite, not the system one.

## API retry load
- A failed `invalidateQueries` call can refetch many active queries. Do not retry it on a short fixed UI timer; use bounded backoff so an API error cannot exhaust the shared rate limit.

## Design fidelity
- **Code must read like shadcn/ui** (user, 2026-09-29): inline Tailwind or a small component with `data-slot`/`cva`. Never add a `const fooClass = '…'`, a class helper function, or an app CSS class; style library DOM with arbitrary variants from our wrapper. Rules: `apps/web/README.md`.
- The accent is pink (`--primary`) in both themes; Coolify's purple/yellow accent notes are obsolete since the shadcn migration.
- the chat reference's chat chrome reads clean because of three things: outlined icons at 20px in 32px buttons with 8px gaps (all outline, one reicon family; state uses color, not fill — user, 2026-10-02), a borderless `bg-secondary/40` search field with a 16px right gutter, and neutral member names (white/muted) — role/user colors belong on avatars and message authors only. reicon's default weight is Outline; reicon `Pin` is a map marker, the pushpin is `PinTack`.
- Second sidebars (mail folders, docs tree, tasks projects, chat channels) all use a 48px `PaneHeader` title row (`components/common/Pane.tsx`) and 8px body padding; the app sidebar brand row is also 48px so everything lines up. Never let a list start flush at the top of a pane. Empty states fill the whole pane area (`flex-1`).
- **Coolify v4 is flat-first, not card-first.** The app canvas is one flat surface; columns and second sidebars are separated by 1px borders directly on the canvas. Rounded bordered cards are ONLY for form sections and resource tiles inside content — never wrap whole panes/lists/readers in cards. (Corrected by user on 2026-08-21 after I carded every pane.)
- When the user asks for a "1:1 copy" of a reference UI, port the reference CSS values verbatim (surface ladder, paddings, font sizes, grid columns) into our tokens/utilities instead of approximating. Read `resources/css/app.css` + `utilities.css` + the blade of the exact page first.
- Before restyling to match a reference, restudy the actual screenshot section by section (shell, second nav, content, controls) instead of extrapolating a single pattern to everything.

## Orbit is multi-app
- Features that belong to one app (task views, task presets) nest under that app in the sidebar and headers; never add them as top-level Workspace items next to Tasks/Docs/Mail/Chat (user, 2026-09-25).
- Contextual actions (e.g. "Save view") appear only when they can do something; do not show them on an unchanged page.
- Settings that belong to the person (profile, notifications, sessions, keyboard shortcuts) go in Account settings (`/profile/*`, `AccountLayout`), each as its own page. Workspace Settings (`/settings/*`) has only what belongs to the workspace (user, 2026-10-03).

## Reference discipline
- **When the user names a screen by the reference's name, build that exact screen at the reference's mount point — not a lookalike inside an existing page.** "Server settings" in the chat reference is its own route whose sidebar replaces the channel sidebar, opened from the server dropdown; I first put its tabs into the app-wide Settings page and had to redo it (2026-08-29). Also copy data semantics, not just markup: the member list groups by primary role and names take the highest role's color.
- **Check the reference's component tree, not just its classes.** the chat reference's `ChatArea` renders the member list as `rightPanel` *inside* the chat column under a full-width header (`header` + `flex min-h-0 flex-1` row). I ported the member list as a sibling pane, so the header stopped short of the right edge; the user had to point it out (2026-08-29). When porting a screen, first grep where each panel is mounted (`grep -n "<MemberList\|rightPanel"`) before styling.
- **Always study the assigned reference repo BEFORE building a feature, not after.** Chat was built generically without reading `the chat reference`; the user caught it. The brief maps each feature to a reference (Tasks/Mail→the tasks reference, Docs→the docs reference, Chat→the chat reference) — dispatch an Explore agent on the reference first and implement from its spec.
- Layer/nested two-tone card surfaces are a reference pattern, not Coolify. Coolify cards = one surface + one border.

## Tooling
- `jean.json` gives every worktree the same run ports (18888 web, 18080 API, `--strictPort`), so a second worktree's run environment cannot start while another one runs, and port 18080 can also be taken by a local VM. Check who owns the port (`ss -ltnp`, `/proc/<pid>/cwd`) before you trust a page there. Without a working run environment, build the web app and let Playwright serve `dist/` and stub `/api` with `page.route` (no extra server process) (2026-10-20).
- Do not run `bunx expo lint` in `apps/mobile`: mobile has no ESLint setup, and the command installs ESLint, edits `package.json` and `bun.lock` and writes `eslint.config.js`. The mobile checks are `bunx tsc --noEmit -p .`, `bun test src` and the iOS bundle (`bunx expo export --platform ios` when no Metro runs) (2026-10-28).
- `expo export` shares Metro's cache in `/tmp` with every worktree: a DOM bundle can fail with "Unable to resolve ./../../../../<other-worktree>/apps/mobile/…". Rerun with `--clear` when no Metro server runs (2026-11-10).
- Never `git stash` to compare against `main`: other sessions edit this working tree at the same time, so the stash takes their work and `stash pop` fails on their new edits. Undo your change in place (or use `git show HEAD:path`) instead (2026-10-15).
- The release image is static musl on Alpine with no OpenSSL. Pick pure-Rust crates (rustls, RustCrypto); `webauthn-rs` 0.5 needs OpenSSL, so Orbit pins the pure-Rust 0.6 line.
- The user-run Vite dev server (port 8888) can serve a broken module graph after files are renamed (`.tsx`→`.ts`) mid-session — pages render blank with no console errors. Verify against `aubr build` + `aubx vite preview` before debugging app code, and report the dev server needs a restart instead of touching it.
- Probing dev-mode React modules via `import()` from a bare page fails with "@vitejs/plugin-react can't detect preamble" — that's a probe artifact, not an app error.
- Headless Firefox `--screenshot` can paint stale CSS custom-property values (showed light-theme values inside dark theme). Trust a beacon probe of `getComputedStyle` over screenshot colors before "fixing" color bugs.
- Headless Firefox `--screenshot` does not wait for `loading="lazy"` images inside a scroll container (they show alt text). Data-URL images should load eagerly anyway; for real URLs, accept the artifact or test on a bare page.
- Headless Firefox `--screenshot` captures CSS animations at frame 0: anything with an enter animation (modal fade, `slide-in-right` panels) is invisible/off-screen in the PNG. Verify such UI with a static test page that links the built CSS and sets `*{animation:none!important}`; a 50%-black backdrop is also invisible on the near-black canvas, so do not read its absence as "not rendered".
- Browser-default `text-align: center` on `<button>` inherits into child spans — the base reset must set `text-align: left` on buttons.
- The production CSP is `style-src 'self'` plus the empty-string hash (an empty `<style>` filled with `insertRule` works): a library that adds a `<style>` at runtime (Sonner, Mermaid) works on the Vite dev server and loses its styles in production. Put the rules in `index.css` or a constructed style sheet (`document.adoptedStyleSheets`), and test a production build served with the server's CSP header.
- The web and mobile apps have no Prettier config (no semicolons, single quotes, long lines): never run `bunx prettier --write`, its defaults rewrite whole files. Format by hand like the code around it (2026-11-11).
- Typecheck the web app with `bunx tsc -b` (what `build` runs). `tsc --noEmit -p .` checks nothing: the root `tsconfig.json` only has references, so it passes on broken code.
- The `orbit` CLI logs to stdout (sqlx prints Postgres notices at INFO). A command whose stdout is data for scripts must log to stderr: add it to `Cli::output_is_data` (`cli.rs`), as `orbit improvements` does. A test that runs the CLI must drop its own `ORBIT_*` variables, or Orbit refuses to start.
- `cargo test --workspace` stops at the first failing test binary; use `--no-fail-fast` to see the whole suite.
- "the trait `Handler<_, _>` is not implemented" for an axum route usually means its future is not `Send` (axum's `debug_handler` macro is not enabled here). Find the value with a probe, `fn is_send<T: Send>(_: T) {}` called on the handler's future: rustc then names the value held across an `.await` (a `url::form_urlencoded::Serializer` on 2026-11-10). End such values in a block before the `.await`.
- Give every cargo call of a worktree the same `CARGO_PROFILE_*`/`CARGO_INCREMENTAL` values (run it through `orbit-build`, see `AGENTS.md`): a change rebuilds everything (2 GB more on 2026-10-29). To rerun built tests, run `target/debug/deps/<test>-<hash>` directly.
- On the shared VM, an incremental debug compile of the `orbit_server` lib can need more than the 3 GiB `MemoryHigh` and stall in memory reclaim for hours (rustc system time far above user time, `memory.pressure` near 100). Build with `CARGO_INCREMENTAL=0` (it rebuilds only the workspace crates); do not raise the limit yourself.
- `include_dir!` does not notice a rebuilt `apps/web/dist`: after a web build, `touch apps/server/src/static_assets.rs` before the Rust build, or the binary serves the old frontend.
- A fresh worktree has no `apps/web/dist`, so the `orbit_server` lib does not compile (`include_dir!` panics), and the web build needs the client that `just api` makes from the server. Break the cycle with an empty `apps/web/dist` for the test build and `just api`, then build the web app and the binary. `cargo run`/`cargo build` and `cargo test` each build their own `orbit_server` lib (dev-dependency features differ): plan two ~10-minute lib builds (2026-11-11).
- Disk on the shared VM: one debug build of `orbit-server` writes ~1.4 GB (the `orbit` binary ~650 MB and its rlib ~750 MB), its incremental cache ~1.3 GB, and each server test binary ~550 MB (58 binaries do not fit). With a few GB free, run test binaries one batch at a time, delete them after, and delete `target/debug/incremental/orbit_server-*` of your build (2026-11-04); or, for one full run, `cargo clean` and run the whole suite with `CARGO_PROFILE_DEV_DEBUG=0` (2026-11-11).
- `apps/web/test-results/` (Playwright output) is ignored; never commit traces or `error-context.md` from it (an upstream commit once did).
- On the shared VM, `node` and `npm` are not on `PATH`: use `/root/.local/node/bin` (Node 24, with npm) for `apps/agent` (`just agent-test` needs it).
- If the Agent Browser fails with a `SingletonLock: File exists` profile error, another session holds the profile. Do not delete the lock; drive the dev server with `playwright-core` from `apps/web/node_modules` instead (script in `/tmp`).
- If Playwright's pinned headless shell is not installed, set `PLAYWRIGHT_CHROMIUM_EXECUTABLE` to an installed one under `~/.cache/ms-playwright/` (`playwright.config.ts` reads it) instead of downloading browsers.
- Under bun + happy-dom, a failing `toBeNull()` on an element or a failing `waitFor` on a full page can exhaust a 4 GB memory cap with no output. Assert negatives by count and keep rendered page tests focused.
- happy-dom has no layout, so a TanStack Virtual list mounts no rows in tests. Put `data-virtual-scroller` on the scroll element: `src/test/dom.ts` gives it a height (and each `data-index` line 1px). Positions in such tests are line indexes in px.
- An `IntersectionObserver` with the default root (the window) is clipped by every scrolling ancestor first, so its `rootMargin` does nothing inside a scrolling pane. Pass the pane as `root` (`PdfPages.tsx`, 2026-11-02).
- Measure web CPU and memory with `scripts/perf/` (seed, measure, e2e; results and method in `.ai/web-perf.md`) and compare a before and an after run of the same script.
- A page that `App.tsx` loads lazily but that shares modules with the entry (auth components, API helpers) can make Rolldown's `shared` chunk group import the entry in a cycle: the built app then fails at load with "`_ is not a function`" (Vite dev works). Import such small public pages eagerly, like the other pages outside `AuthGate`, and check the built app in a browser. A second `lazy()` import of a module that a lazy page already loads (the editor's `DriveViewer`, 2026-11-11) did the same ("`J` is not a function" on the docs page): import it statically there.
- A TanStack virtualizer in a child component that scrolls a parent's element must get that element as state (a callback ref), not a `useRef`: child layout effects run before the parent's ref is attached, so `getScrollElement()` returns null and the list never attaches. Several virtualizers can share one scroller with `scrollMargin`; then set `shouldAdjustScrollPositionOnItemSizeChange = () => false`, as one list's scroll correction moves all the others (the task board, 2026-11-09).
- The React Compiler skips any component that calls `useVirtualizer`, so each range change (many a second while scrolling) renders every mounted row again. Render costly rows through a `memo` component: on the board this halved the time per scroll frame; when the component that calls `useVirtualizer` also builds the rows (the task list), compare an explicit `deps` list that holds everything the row and its handlers read (`MemoRow` in `TaskList.tsx`, 36% less scroll work) (2026-11-10).
- Tailwind v4 emits variant rules in its own order, not class order: two `data-[…]:` variants that set the same property (e.g. `top`) at equal specificity can resolve the wrong way. Make such variants mutually exclusive instead of relying on override order.
- The fake S3 (`start_fake_s3`) does not check signatures. To prove S3 signing against a real server, run `crates/platform/tests/object_storage.rs` with `ORBIT_TEST_S3_*` against a local versitygw (`versitygw posix <dir>`; MinIO has no free image or binary any more). versitygw writes ETag quotes as `&#34;`, and a copy onto itself does not change Last-Modified there (`a_duplicate_upload_to_s3_refreshes_the_existing_object` fails on it; R2/AWS are fine).

## HTML5 drag and drop: never unmount the drag source on dragstart
- Symptom: kanban cards went invisible while dragging and no drop indicator appeared.
- Cause: `dragstart` set state that filtered the dragged card out of the column; the browser cancels a drag when its source element leaves the DOM (no `dragover`/`drop` fire).
- Rule: keep the source mounted (fade it with a data attribute) and render the placeholder among the *other* items; only unmount after `drop`/`dragend`.

## Never kill processes by name; only kill PIDs you started
- `pkill -f firefox` killed the user's real Firefox during headless screenshot verification.
- Rule: start background helpers with `cmd & echo $!`, keep the PID, and `kill <pid>` that PID only. Never pkill by a generic name (firefox, node, vite).
- For headless Firefox screenshots: the `--screenshot` invocation exits on its own; wrap in `timeout N` instead of killing.

## Package-manager choice
- The current checkout was installed with **aube** (lockfile: `apps/web/aube-lock.yaml`; virtual store at `~/.cache/aube`), so existing automation commonly uses `aube run <script>` / `aube exec <bin>`.
- **Aube is optional.** The project is an ordinary Vite package and can be switched to Bun or pnpm at any time. Use one package manager consistently, generate its lockfile, and remove the obsolete lockfile in the same migration.

## shadcn + Base UI migration (2026-09-19)
- **Render Base UI Popover/menu content only while open.** In our Dropdown wrapper, `{open ? <PopoverContent>…</PopoverContent> : null}`. Closed Base UI popovers still mount their Portal/Positioner and do async work; a list view with hundreds of row-level dropdowns (TaskList: 3 per row × 100+ rows) hangs a render tick if all popover content mounts eagerly. This also matches the old createPortal-on-open behaviour and makes `children(close)` lazy.
- **Base UI popover testing under happy-dom:** `fireEvent.click` OPENS a Base UI trigger (a single click event); `userEvent.click` DOUBLE-toggles it (full pointer sequence → open then close) so the content never appears — use fireEvent to open. For the option/menu item inside, use `userEvent.click(await findByRole(...))` — it waits until the opened popover is actionable (a bare `fireEvent.click` right after opening can no-op because the positioner hasn't settled). Assert open/closed via the trigger's `aria-expanded`, not DOM removal (closed popups linger for an exit animation that never fires in happy-dom). Outside-click dismissal needs `userEvent.click(outside)`, not `fireEvent.pointerDown`.
- **Base UI Popover is modal by default** (inerts the page) — pass `modal={false}` for dropdowns/menus, or the rest of the page becomes inert and content can render oddly.
- **Render the trigger AS the consumer's element** (`<PopoverTrigger render={triggerEl} />`), don't wrap it in a `nativeButton={false}` span — the span adds an extra `role=button` whose accessible name collides with menu items (getByRole ambiguity).
- **Casing collisions:** shadcn primitives are lowercase (`avatar.tsx`); our PascalCase files (`Avatar.tsx`) collide on case-insensitive TS module resolution. Rename our wrapper (e.g. `UserAvatar.tsx`) or the build fails with TS1261.
- **cn is its own package now** (`import { cn } from "cn"`), not `@/lib/utils` internals; `src/lib/utils.ts` just re-exports it. TS6/TS7 deprecate `baseUrl` — use `paths` without it.

## Prefer registry components over hand-maintained wrappers
- After a shadcn migration, converting raw HTML to Tailwind classes is only half the job: also check whether the registry
  already has the component we hand-maintain (`shadcn search @shadcn -l 100`). The user had to point this out for the
  command palette (2026-09-20).
- Concretely replaced: custom `Dropdown` wrapper → `DropdownMenu`/`Popover`, custom `Listbox` → `Select`,
  hand-rolled command palette → `Command`/`CommandDialog`, CSS spinner → `Spinner`, ⌘K spans → `Kbd`,
  icon+input search boxes → `InputGroup`, label+control rows → `Field`.
- Keep custom only where the primitive fights the interaction: 2-D emoji grid, slash menu that must keep the caret in the
  editor, @mention list that must not take focus.

## shadcn layout primitives carry `w-full` — override it when replacing content-sized markup
- The tasks top-bar search stretched across the header after the `InputGroup` migration: `InputGroup`'s base class list contains
  `w-full`, while the old `<label>` wrapper had no width and sized to `min-w-[180px]` (2026-09-20, user reported).
- `Input`, `Textarea` and `SelectTrigger` carry `w-full` too. When porting a flex-row control, diff the old wrapper's width
  classes against the primitive's base and add an explicit `w-auto`/`w-56`/`flex-1` override; `cn()` then drops `w-full`.
- Guard it with a source-level assertion in the layout tests, since the failure is invisible to tsc and jsdom-style tests.

## Verifying layout without a dev server
- Chromium for Playwright is installed at `~/.cache/ms-playwright/chromium_headless_shell-1234/...`, but `playwright-core`
  expects a newer revision, so pass `executablePath` explicitly to `chromium.launch()`.
- Working recipe: `vite build`, then `page.setContent()` with `<style>` = `dist/assets/*.css` and a small markup reproduction,
  then `getBoundingClientRect()` on before/after variants.
- **Build the class string with the real `cn()`** (import it from node_modules). Hand-concatenating base + override classes
  lets both survive, and the stylesheet order decides the winner, so the probe silently disagrees with the app.

## Production CSP blocks injected `<style>` tags
- Production CSP is `style-src 'self'` (`crates/platform/src/http/security.rs`). Libraries that inject a `<style>` tag at runtime (Tiptap/BlockNote `injectCSS`, sonner toasts) break silently in production but work in dev (Vite serves no CSP). Sonner: import `sonner/dist/styles.css` in `components/ui/sonner.tsx`. Verify against the Rust-served build (`include_dir!` embeds `apps/web/dist` at compile time: `bun run build`, then rebuild/run the server) and count `[data-…]` rules in `document.styleSheets`. Turn the injection off (`_tiptapOptions: { injectCSS: false }`) and copy the CSS into our stylesheet. `img-src` is `'self' data: blob: https:` since 2026-09-25: external https images (image blocks, cover URLs) load, plain http ones do not. Tradeoff: the image host sees every viewer's IP and user agent (tracking pixels work); uploaded page files stay same-origin.

## Bun + happy-dom: never let `expect(domNode)` fail inside `waitFor`
- A failing `expect(node).toBeNull()` makes Bun format the whole happy-dom node (it links to the window) for the error. It costs ~40 ms alone and ~750 ms late in the full run, and `waitFor` repeats it on every poll, so tests time out only in the full suite (SessionsPage, 2026-09-25).
- Use `waitForAbsence(() => view.queryBy…)` from `src/test/waitForAbsence.ts`, or throw a plain `Error` in the callback.

## Parallel agents: format only your own files
- `cargo fmt --all` rewrote another agent's in-progress files under `apps/server/src/notion/` (2026-09-25). While someone
  else works in the tree, run `cargo fmt --check` and `rustfmt` on your own files instead of formatting the workspace.

## Base UI Input in tests and dark-mode checkbox states
- `fireEvent.change` and `userEvent.clear` do not reach a *controlled* Base UI `Input` / `InputGroupInput` under happy-dom
  (the DOM value changes, React state does not). Use `userEvent.type(input, text)` and clear with backspaces
  (`'{Backspace}'.repeat(value.length)`) (Notion import pane, 2026-09-25).
- shadcn checkbox fills need a `dark:` twin (`dark:data-checked:bg-primary`) to beat `dark:bg-input/30`; a new state such as
  `data-indeterminate` needs `dark:data-indeterminate:bg-primary` too, or it renders as an outline in dark mode only.


## Realtime refresh pauses while a draft is focused
- `useWorkspaceEvents` skips query invalidation while an input/textarea/contenteditable is focused (protects form drafts).
  Surfaces that are focused almost all the time must merge remote updates themselves and opt out with
  `data-realtime-safe` (Docs `DocEditor`), or they never refresh while someone types — the other user's title change
  never arrived and the next save hit a 409 (2026-09-25). Portalled dialogs/popovers stay outside the marker.

## Print CSS: cascade layers and BlockNote's `.dark`
- Unlayered `!important` loses to layered `!important` (important order reverses layer priority). Tailwind `…!` utilities
  and the base `max-md:text-[16px]!` input rule beat any plain `@media print { … !important }` override; the page title
  printed at 16px because A4 print width is under `md`. Put such overrides in `@layer base` with a more specific
  selector (docs export, 2026-09-25).
- BlockNote puts the scheme class (`dark`) on its own containers, so `.dark { --foreground … }` re-applies there: a
  light-theme override on `<html>` alone left white editor text on white paper. Override `html.x, html.x .dark`.

## Opening detail views without flicker
- Do not show a "Loading…" boundary between a list and a detail view, and do not let detail sections arrive one by one (a late query that toggles read-only/editable remounts the fields). Put the detail queries in shared `queryOptions` factories, prefetch them all on open and navigate when they resolve (short cap, e.g. 300ms); on a direct URL load render a blank canvas until every detail query settles (`features/tasks/api/tasks.ts` `prefetchTaskDetail`, 2026-09-26).

## Brand mark
- The logo is the pink pixel "W" (Wellrox fork; upstream has an "O") in `apps/web/public/logo.svg` (11x11 grid, pink `#f2458f`, shadow `#742f4d` 1 unit down-right). `scripts/logo-icons.sh` makes the favicons, the PNG app icons and the desktop `orbit-64.png`/`orbit.ico` from it; the loader in `OrbitUnavailable.tsx`, the launch-video `OrbitMark` and mobile `icons.gen.ts` (`bun scripts/generate-icons.ts`) copy it too. Change them together. The UI `--primary`/`--sidebar-primary` tokens (both themes) use the same pink, `oklch(0.659 0.216 358.989)`, with white foreground. Show it at multiples of 11px (22, 44…) so pixels stay crisp.

## Permissions
- Every authorization rule lives in `crates/orbit/src/policy.rs`: `Policy::can(role, Permission)` for role rules, methods on `Actor` for rules that depend on a record (author, owner, target member). Repositories get the `Actor` from `repositories/membership.rs` inside their own transaction; they never compare role names or write their own role SQL.
- The web app holds no rules. Workspace abilities come from `workspace.permissions` (`useCan`), installation admin from `useIsInstallationAdmin`, and per-record abilities from `can_*` flags on the record (`can_edit`, `can_delete`, `can_purge`, ...). A new gated action is a new `Permission` or `Actor` method, a flag on the record if the UI needs it, then `just api`.
- Build workspace fixtures in web tests and e2e mocks with `testWorkspace(role)` (`src/test/workspace.ts`); its role table is generated from the server (`packages/api/src/generated/rolePermissions.ts`). Hand-written literals passed to `setQueryData` or `Response.json` are untyped, so a new required field breaks them only at run time.
- Sessions (cookie or `Authorization: Bearer`) are read only through `auth_routes::request_session`.

## One editor schema for every client of a Y.Doc
- A page's Y.Doc is shared by the web, the mobile editor and the server's converter. An editor with any other schema
  (a missing block, a different prop default, no comment mark) silently drops or rewrites nodes for everyone. Build
  every editor from `createEditorSchemas` in `packages/editor` and pass only views; `packages/editor/src/schema.test.ts`
  checks the result against `apps/server/src/collab/blocknote-schema.json`. Never PATCH `content` of a page from a client
  that is not the collaborative editor: it replaces the live document (the old mobile "Add text" did, 2026-10-24).
- Expo DOM components render in a web view whose `#root` is a flex row: give the component's root `flex: 1`, or it is
  only as wide as its content. The Expo web target renders the component inline, so the preview does not show this;
  check the exported bundle (`expo export --platform ios`, `www.bundle/*.html`) with a fake `window.ReactNativeWebView`.

## exe.dev gateway in front of production
- The exe.dev proxy answers a request with no token with `307` to `/__exe.dev/login`, and a bad or expired token with `401 text/plain` + `WWW-Authenticate: Bearer, Basic` (Orbit's 401 is problem+json). It also reads a plain `Authorization: Bearer` as its own token, so a native client or script that sends only Orbit's bearer session gets the proxy's 401. Send the exe.dev token in `X-Exedev-Authorization` (2026-11-08, `apps/desktop/README.md`).

## Desktop app (apps/desktop)
- The desktop app is the web UI in a system web view (WebView2 / WKWebView / WebKitGTK). Its bridge (`apps/desktop/crates/orbit-desktop/assets/bridge.js`) replaces `Notification`, `ServiceWorkerRegistration.showNotification` and `navigator.setAppBadge`, and dispatches the worker's `orbit:navigate` message on a notification click. When you change how the web app notifies, badges or follows a notification click (`features/realtime/notify.ts`, `push.ts`, `lib/favicon.ts`), keep those calls or update the bridge (2026-11-09).
