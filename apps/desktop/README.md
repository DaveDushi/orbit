# Orbit desktop (Rune) — M0 spike, M1 sign-in and sockets

Design: `/root/orbit-designs/desktop-rune.md`. This is its own Cargo workspace, excluded from the root one:
Rune pins Rust 1.99.1 (Orbit pins 1.97.1), so it has its own `rust-toolchain.toml`, `Cargo.lock` and `target/`.

| Crate | What |
|---|---|
| `orbit-client` | `/api/v1` types, the HTTP client (`api.rs`: gateway cookies, Orbit login/2FA/logout, error classes), the gateway cookie hand-off (`cookies.rs`), the workspace sockets (`live.rs` rules, `socket.rs` threads), backoff, the fixture backend, the contract test against `packages/api/src/generated/openapi.json`. No Rune, no async runtime. |
| `orbit-store` | The local SQLite file (0600, deleted on sign-out): the outbox. |
| `orbit-desktop` | The Rune app (`app.rs` state and flow, `view.rs`), the gateway sign-in window (`signin.rs`, tao + wry), the keychain (`keychain.rs`), the risk-4b check (`check.rs`). |

Rune, rune-kit and rune-notify are pinned to one rev of Rune's `dev` branch (`a7f276c3`, 2026-11-08) in
`Cargo.toml`. Bump it on purpose only.

## Commands

On the shared VM, run every build through `orbit-build` (see the root `AGENTS.md`). Linux needs
`libxkbcommon-dev` to link, `libwebkit2gtk-4.1-dev` and `pkg-config` to build the sign-in window (wry),
`mesa-vulkan-drivers` (lavapipe) to render headless, and `libxkbcommon-x11-0` to open a window on X11.

```bash
cd apps/desktop
orbit-build cargo test --workspace                     # outbox, parsing, gateway rules, contract
orbit-build cargo build -p orbit-desktop
target/debug/orbit-desktop --fixture fixtures/demo                                   # a window
target/debug/orbit-desktop --fixture fixtures/demo --snap out.png --theme dark       # headless PNG
target/debug/orbit-desktop --fixture fixtures/demo --scene selected --tree tree.json # element + a11y tree
target/debug/orbit-desktop                                  # the real app (server → gateway window → sign-in)
target/debug/orbit-desktop --check-gateway https://orbit.wellrox.com   # risk 4b, needs a person (below)
python3 -I scripts/fake-gateway.py PORT PASSWORD_FILE [forward|strip|reject|keepcookie]  # local E2E server
scripts/gateway-probe.sh TOKEN_FILE      # M0: read-only gateway checks with an exe.dev VM token (G1, dropped)
```

Fixtures hold synthetic data only. Never put real users, messages or tokens in fixtures or goldens.

Scenes for `--snap` (with `--fixture fixtures/demo`): `signed-out`, `gateway`, `login`, `login-error`,
`two-factor`, `signed-in`, `reconnecting`, `session-ended`, `selected`.

## M1: sign-in and sockets (2026-11-09)

**Sign-in (decision D1 = G2).** The app asks for the server, probes `GET /api/v1/setup/status`, and when the
exe.dev gateway answers `307` to `/__exe.dev/login` it runs itself as `orbit-desktop --gateway-signin <server>`:
a tao + wry window (a separate process, because Rune's winit owns the main thread) that loads the probe, shows
the gateway's sign-in, and when a page of the server's own origin finishes loading, writes the server's cookies
(`cookies_for_url`) as one JSON line to its stdout pipe (`orbit_client::cookies`, refuses a terminal). Each
server has its own web view data dir on Linux and Windows; macOS uses the default WebKit store. Then Orbit's
own sign-in: `POST /api/v1/auth/login` (+ `/login/two-factor`) without `Origin`, the token from the body,
`Authorization: Bearer` on every request and socket upgrade. Gateway cookies and the session are JSON entries
in the OS keychain (`keyring`: Keychain, Credential Manager, Secret Service), one per server; secrets are
`Secret`s (redacted `Debug`/`Display`) and never go in a URL or a log.

**Who refused** (`api::classify_response`): gateway `307` → open the sign-in window again; gateway `401` (with
`WWW-Authenticate: Bearer`) → same; Orbit `401` problem+json → sign in to Orbit again; `409 contract_mismatch`
→ update the app; anything else → an error line.

**Sockets** (`orbit-client/src/socket.rs`): one thread per socket, blocking `tungstenite` with rustls (ring).
`/live` resumes with `?epoch=&after=` (`LiveCursor`, same rules as `apps/mobile/src/data/live.ts`), `/events`
with `?after=`; `resync` / `resync_required` refetch. Close `4401` → signed out, `4403` → no access (both stop);
`1012`, drops and other codes reconnect after 1×, 2×, 4× … 30× 1 s plus up to 50 % jitter; the delays start
again only after a connection held 10 s. A refused upgrade is classified like HTTP (gateway 307/401 stop and
reopen the sign-in window). Silence over 60 s counts as lost.

**Local end-to-end** (Xvfb, WebKitGTK, synthetic data): `scripts/fake-gateway.py` is a fake gateway (307 to a
fake sign-in page that sets a cookie) in front of a fake Orbit that applies Orbit's origin rule. With it the
real app went server → sign-in window → Orbit sign-in → sidebar → both sockets connected → server down
(reconnecting) → server back with new cookies (window reopened, "session ended") → sign-out. `--check-gateway`
gives the right verdict in all four modes and always revokes its test session.

### Risk 4b: does the gateway pass `Authorization: Bearer` (and leave Orbit's origin rule alone)?

Not yet answered on production: it needs a person to sign in to exe.dev and to Orbit (no saved credentials).
Run on a computer with a display (macOS: `cargo build -p orbit-desktop` in `apps/desktop`):

```bash
target/debug/orbit-desktop --check-gateway https://orbit.wellrox.com
```

It probes, opens the sign-in window, then asks for the Orbit email, password (hidden) and 2FA code in the
terminal. GETs only, plus one sign-in and one sign-out; it prints no secret and revokes its session at the end
(by bearer, or with Orbit's session cookie and `Origin` when the bearer does not reach Orbit).

| Verdict | Meaning |
|---|---|
| `FORWARDED` | Orbit got the bearer on HTTP, `/live` and `/events`. The M1 design works as is. |
| `STRIPPED` | The proxy removed `Authorization`. |
| `REJECTED` | The proxy answered `401` itself: it reads `Authorization` as its own token even with valid cookies. |
| `COOKIE FORWARDED` | Sign-in got `403 origin_forbidden`: the proxy passes its own cookie to Orbit, and Orbit refuses a request without `Origin` that has any cookie (`crates/platform/src/http/security.rs:112-127`). |

Options if it is not `FORWARDED` (none changes the server in M1; the user decides):
1. **Browser mode, no server change:** send `Origin: <server>` and use Orbit's own session cookie
   (`__Host-orbit_session`, taken from the sign-in's `Set-Cookie`) instead of the bearer. Orbit accepts it as
   a browser session from its own origin; it fixes all three failures if the proxy forwards cookies.
2. **Small server change:** read the native session from a second header the proxy leaves alone (for example
   `X-Orbit-Session`), and let a request without `Origin` carry cookies when it has that header.
3. **Gateway:** ask exe.dev to forward `Authorization` / strip its cookie for this VM (not in our control).

**Not done in M1:** gateway cookie refresh from `Set-Cookie` (when the cookies expire the app opens the window
again); messages, typing and presence frames (M2/M3); tested on Linux only (macOS and Windows build in CI).
On Linux without a Secret Service (a bare VM) the keychain save fails: the app logs the entry name and keeps
the secrets in memory only, so it asks again after a restart.

## M0 findings (2026-11-08)

**Gateway (exe.dev proxy), seen on `orbit.wellrox.com` with GETs only:**
- No token: `307` to `/__exe.dev/login?exedev_host=…&redirect=…`, for HTTP and for WebSocket upgrades.
- Invalid or expired token in `X-Exedev-Authorization`: `401`, `text/plain` "invalid or missing authentication
  (trace …)", `WWW-Authenticate: Bearer, Basic`, for HTTP and upgrades. Orbit's own `401` is
  `application/problem+json` without `WWW-Authenticate`, so `gateway::classify` tells them apart.
- **The proxy also reads `Authorization: Bearer` as an exe.dev token** (its deprecated form, per
  https://exe.dev/docs/https-tokens-for-vms.md): Orbit's session header alone gets the proxy's `401`. Not yet known:
  whether, with a valid `X-Exedev-Authorization`, the proxy forwards `Authorization` to Orbit. `scripts/gateway-probe.sh`
  answers it (step 4: Orbit answers `403 origin_forbidden` when it did not get the header). If it strips it, Orbit
  needs another way to take the native session (M1 decision).
- Token lifetime (https://exe.dev/docs/https-api.md): the signed payload has optional `exp`/`nbf`; without `exp` the
  token never expires. `ssh exe.dev ssh-key generate-api-key --vm=<vm> --exp=30d` sets one. Revoke by removing the
  signing ssh key from exe.dev. No replay protection. An expired token gets the same `401` as a bad one.

**Outbox dedupe:** the server dedupes on `nonce` (1–64 bytes, required on `POST …/messages`):
`apps/server/src/repositories/chat/messages.rs:251-286` (`send_message` returns the first message for a repeated
`(conversation, author, nonce)`), `sent_before` at `:923`, unique index `chat_messages_nonce`
(`apps/server/migrations/0037_chat.sql:136`, Postgres `0046_baseline.sql:2173`), test
`apps/server/tests/chat_api.rs:425`. `orbit-store` gives each entry one nonce for its life and sends one head per
conversation.

**Notifications (macOS, ad-hoc signed):** `rune-notify` posts only from a bundle (`mac.rs` `bundled()`: a bundle id
and a `.app` path) and asks permission on the first post. Rune's own installer signs ad hoc by default
(`scripts/mac_app_install.py`, `MAC_APP_SIGN` defaults to `-`). Reports say an ad-hoc-signed bundle can get
permission; a bare binary cannot. Not tested on a Mac yet.
