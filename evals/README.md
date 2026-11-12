# Orbit evals: performance and security

One command measures Orbit's performance and checks its security, writes the result to
`evals/results/<date>-<sha>.json`, and compares it with the previous result and with `budgets.json`.

```bash
just evals                       # or: evals/run.sh — perf + security, about 6.5 minutes on the 4-vCPU VM
evals/run.sh --quick             # fewer runs and requests, no git-history secret scan
evals/run.sh --perf              # only performance
evals/run.sh --security          # only security
evals/run.sh --build             # first a clean release build (about 18 minutes, 1 job), and record its time and memory
evals/run.sh --no-prod           # no requests to production
evals/run.sh --noise[=N]         # N perf runs of this commit (default 5), writes evals/noise.json (about 4 minutes each)
bun evals/src/compare.ts <before.json> <after.json>   # compare two stored results again
```

The run needs the release binary `target/release/orbit` (with the web build embedded). Make it once with `--build`;
later runs reuse it and report the build numbers that `--build` stored in `target/evals-build.json` for that commit.

Exit code: `0` all good; `1` a metric is over its budget, a perf metric got worse than the previous result beyond its
noise (below), or a high/critical finding that the previous result did not have; `2` a step failed (see the log).

## Did my change make it slower?

Each run compares its perf numbers with the latest result in `results/` (`src/compare.ts`). A metric is **worse**, and
fails the run, only when all three are true:

1. it moved by more than **15%** (`THRESHOLD_PCT`; a rule in `RULES` can set its own),
2. by more than its **floor**, in its own unit: 1 ms for API latency, one frame (16.7 ms) for FCP and LCP, 50 ms for
   TBT, 0.02 for CLS, 2 kB for bundle sizes, 5 kB and 1 request for page transfers, 50 ms for server start, 5 MB for
   server memory, 30 s and 100 MB for builds, 100 requests/s for throughput,
3. and by more than its **noise**: how far it moved between runs of one commit on this machine (max - min, from
   `noise.json`).

Error responses (`non_200`) count from the first one. The same move the other way is **better**. A move over 15% that
fails the floor or the noise is shown as **under floor** or **in noise**. A metric the new run did not measure (for
example the build numbers of a run without `--build`) is **missing**. None of these fail the run. Metrics that moved
less than 15% are not shown. Without `noise.json` (or without the metric in it) only threshold and floor decide.

```text
Against 2026-11-05-1302-f1b4082.json (noise of 5 runs of 66dc33c):
  metric                                        before   after              delta   noise      verdict
  bundle.entry_js.gzip_kb                        253.4   424.8   +171.4 (+67.64%)      ±0        WORSE
  pageload.pages.docs.desktop.tbt_ms               117     661    +544 (+464.96%)    ±151        WORSE
  build.release_build_seconds                   1047.3       -                          -      missing
  latency.endpoints.docs_read.p95_ms              1.18    3.04   +1.86 (+157.63%)  ±12.21     in noise
  pageload.pages.app_tasks.desktop.cls           0.002   0.001       -0.001 (-50%)      ±0  under floor
  33 worse (fails the run), 0 better, 36 inside noise or floor, 5 missing
```

On the shared 4-vCPU VM (2026-11-10) bundle sizes, transfers, CLS and memory do not move between runs; LCP moves by
10-40%, TBT up to 140%, and API latency and throughput by 40-500%, so only large API changes count there.

Re-measure the noise (`evals/run.sh --noise`, 5 runs under one build lock, about 20 minutes) after a change to the VM
or to how a metric is measured, and commit `noise.json`. Compare results of the same machine and the same `--quick`
setting; the run prints a note when they differ. Five runs give a range that is a little narrower than the real one.

The method comes from Ember's `perf/src/compare.ts` and its README section "Did my change make it slower?"
([NYLLON-SOFTWARE/ember-cloudflare](https://github.com/NYLLON-SOFTWARE/ember-cloudflare), MIT licence, copyright
37signals, LLC and NYLLON LLC). Orbit's version is a rewrite for its own metrics, not a copy.

## Rules it keeps

- **Shared VM.** On the build VM the whole run re-executes itself through `orbit-build` (global lock, build slice), and
  cargo uses `CARGO_INCREMENTAL=0`. It waits for another session's build; it never bypasses the lock.
- **Active tests only on an isolated instance.** The run starts its own server on a free loopback port with a new data
  directory and a random app key under `/tmp/orbit-evals.*`, seeds it, and deletes it at the end.
- **Production: read-only.** `security-prod.ts` sends only `GET`/`HEAD` and a TLS handshake to
  `https://orbit.wellrox.com`, never logs in, and its browser aborts any other method.
- **No secrets in results.** Passwords, cookies and tokens stay in the temp directory. gitleaks runs with `--redact`, and
  findings keep only rule, file, line and commit. Evidence shows seeded ids as `{A}`, `{pageB}`, ….

## What it measures

| Step | Script | What |
| --- | --- | --- |
| Build | `lib/build.sh` | `--build` only: web build and clean `cargo build --release`, wall time and peak RSS (`/usr/bin/time -v`) |
| Bundle | `src/bundle.ts` | gzip-9 and brotli-11 of the entry JS/CSS (index.html plus static imports) and of every lazy chunk; lazy chunks over 100 kB gzip |
| Server | `run.sh` | Start to first healthy `/health/ready` (fresh database and seeded database), RSS idle, after the latency run and after the page loads |
| API latency | `src/latency.ts` | `oha`, 10 connections, 500 requests per endpoint (100 with `--quick`), signed in: session, workspaces, docs list/read/search, app list, tasks list/read, file read |
| Page load | `src/pageload.ts` | Playwright Chromium, cold cache, median of 3: FCP, LCP, TBT, CLS, transfer, JS/CSS per page for login, home, tasks (app page) and a doc. Desktop: 40 ms RTT, 10 Mbit/s. Phone: 412×823, 150 ms RTT, 1.6 Mbit/s, CPU 4× slower. Also checks asset compression and CSP console errors |
| Supply chain | `src/supply-chain.ts` | `cargo audit`, `cargo deny` (config `deny.toml`), `bun audit` (root workspace), `npm audit` (apps/agent), gitleaks on the tree and the git history (config `gitleaks.toml`) |
| Production | `src/security-prod.ts` | HSTS, http→https, TLS 1.3 and no TLS 1.0/1.1; security headers, CSP quality, cookie flags and console errors when the pages are public |
| Active checks | `src/security-local.ts` | On the production-mode test instance: headers and CSP, cookie flags, unauthenticated access to every route (from the Rust router, `src/routes.ts`), IDOR between workspaces A and B, API token / MCP / OAuth scope, CSRF and cross-site WebSocket, upload size and active content, path traversal, SSRF (link previews and agent integrations, with a loopback canary), login and reset enumeration, logout, rate limits |

Lighthouse is not used (not installed on the VM, large download); the page metrics come from the browser's own
performance entries. TBT is summed from FCP to the end of the observation window, so it can be a little higher than
Lighthouse's.

## Test instance

`run.sh` starts the release binary twice on one throwaway database:

- **Development mode** (`http://127.0.0.1:<port>`) for seeding, latency and page loads, with
  `ORBIT_RATE_LIMIT_GENERAL_PER_MINUTE=1000000` so the load is not throttled.
- **Production mode** (`ORBIT_PUBLIC_ORIGIN=https://orbit.test`, trusted proxy `127.0.0.1/32`) for the security checks,
  with default rate limits. The client acts as the TLS proxy (`X-Forwarded-Proto: https`, `X-Forwarded-For`), and each
  check uses its own client address so the rate limit buckets do not mix. Production mode matters: development mode
  accepts any http Origin and sets no `Secure` cookies.

Seed (`src/seed.ts`): user A owns workspace A, user B owns workspace B and is not a member of A. Each workspace has a
doc, a text file, a task and two MCP tokens (`read`; all scopes). A has 50 more docs and tasks (10 with `--quick`).

## Production behind the access gate

`https://orbit.wellrox.com` answers anonymous requests with a redirect to the exe.dev login, so Orbit's own pages are not
reachable without signing in, which these checks must not do. The production check then covers the edge (HSTS,
redirect, TLS) and reports the gate page's headers as info; the local production-mode instance covers Orbit's headers,
cookies and console.

## Files

- `budgets.json` — upper limits per metric (dotted path in the result). Lower a limit when a fix lands.
- `noise.json` — run-to-run noise per perf metric (commit, run count, date, machine); `run.sh --noise` writes it.
- `src/compare.ts` — the noise-aware comparison (thresholds and floors in `RULES`); tests: `bun test evals/src`.
- `deny.toml`, `gitleaks.toml` — scanner configs. gitleaks allows only triaged false positives; keep entries narrow.
- `results/` — one JSON per run: commit, date, machine, every metric, every check with evidence, and the comparison.
- `lib/install-tools.sh` — installs pinned `cargo-audit`, `cargo-deny`, `gitleaks` and `oha` release binaries into
  `~/.cargo/bin` when they are missing (checksums where the project publishes them).
