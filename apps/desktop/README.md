# Orbit desktop (Rune) — M0 spike

Design: `/root/orbit-designs/desktop-rune.md`. This is its own Cargo workspace, excluded from the root one:
Rune pins Rust 1.99.1 (Orbit pins 1.97.1), so it has its own `rust-toolchain.toml`, `Cargo.lock` and `target/`.

| Crate | What |
|---|---|
| `orbit-client` | `/api/v1` types (`ConversationRecord`, `ConversationStateRecord`), the exe.dev gateway rules, the fixture backend, the contract test against `packages/api/src/generated/openapi.json`. No Rune. |
| `orbit-store` | The local SQLite file (0600, deleted on sign-out): the outbox. |
| `orbit-desktop` | The Rune app: a window with the workspace's conversations. |

Rune, rune-kit and rune-notify are pinned to one rev of Rune's `dev` branch (`a7f276c3`, 2026-11-08) in
`Cargo.toml`. Bump it on purpose only.

## Commands

On the shared VM, run every build through `orbit-build` (see the root `AGENTS.md`). Linux needs
`libxkbcommon-dev` to link and `mesa-vulkan-drivers` (lavapipe) to render headless.

```bash
cd apps/desktop
orbit-build cargo test --workspace                     # outbox, parsing, gateway rules, contract
orbit-build cargo build -p orbit-desktop
target/debug/orbit-desktop --fixture fixtures/demo                                   # a window
target/debug/orbit-desktop --fixture fixtures/demo --snap out.png --theme dark       # headless PNG
target/debug/orbit-desktop --fixture fixtures/demo --scene selected --tree tree.json # element + a11y tree
scripts/gateway-probe.sh TOKEN_FILE      # read-only gateway checks with an exe.dev VM token
```

Fixtures hold synthetic data only. Never put real users, messages or tokens in fixtures or goldens.

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
