# Orbit desktop: the Orbit web UI in a native window

The app shows the team's real Orbit web UI (the server's own pages) in a system web view, and adds what a
browser tab cannot do well: a tray icon, OS notifications, an unread badge on the taskbar or Dock icon,
downloads to the Downloads folder, links to other sites in the default browser, and "Start with Windows".
Windows is the main target, macOS second. Design history: `/root/orbit-designs/desktop-rune.md` (native
Rune, then GPUI chat screens until 2026-11-09; both are gone, see git history before this change).

This is its own Cargo workspace, excluded from the root one: it has its own `rust-toolchain.toml` (1.99.1;
Orbit pins 1.97.1), `Cargo.lock` and `target/`.

| Crate | What |
|---|---|
| `orbit-desktop` | The app. `shell.rs`: the main window (tao + wry), tray (`tray-icon`), notifications (`notify-rust`), badge, downloads. `instance.rs`: one app per person. `app_id.rs`: the Windows AppUserModelID. `build.rs`: the icon in `Orbit.exe`. `bridge.rs` + `assets/bridge.js`: the script the web view runs on the server's pages and the messages it sends. `route.rs`: links and download names. `settings.rs`: server, autostart flag, window place. `autostart.rs`: the OS entries. `signin.rs` + `check.rs`: the gateway sign-in window and `--check-gateway`. |
| `orbit-client` | `/api/v1` types, the blocking HTTP client and sockets, the gateway rules, the hand-off of gateway cookies (`cookies.rs`), the update check (`update.rs`), the fixture backend and the contract test against `packages/api/src/generated/openapi.json`. The app uses `update.rs` and the server address rules; `--check-gateway` uses the rest. |

## How it works

- **Server.** The first start shows a small start page that asks for the server (`orbit.example.com`;
  `http://` only for this computer); the release build fills in the workflow's `server` input
  (`ORBIT_DEFAULT_SERVER`). Change server… (tray or account menu) shows it again, with Cancel.
- **Connect page** (`assets/connect.html`, 0.1.8). An `https://` server loads through a local page that first
  checks that the server answers (`fetch`, `no-cors`: any answer counts, also the gateway's redirect), then
  loads it. Without an answer (autostart at sign-in before the network is up, a wrong address) it says
  "Cannot reach <host>", tries again after 2, 4 … 30 s and on the `online` event, and has Try again and Change
  server. Before, the web view's error page stayed and never tried again. `http://` (loopback) servers load at
  once: a page without an origin may not reach a loopback address. The choice is in
  `<config dir>/orbit-desktop/settings.json` (with the autostart flag and the window place; no secrets).
- **Sign-in.** The window loads the server. With an exe.dev gateway in front, the gateway's sign-in shows in
  the window, then Orbit's own sign-in page, like in a browser. The web view keeps its cookies in one profile
  per server (`<data dir>/orbit-desktop/gateway/<host>`, the same one the `--gateway-signin` window uses), so
  the person stays signed in after a restart. Nothing goes to the OS keychain: the web view's profile is the
  only copy (WebView2 encrypts its cookies with DPAPI; WebKit keeps them in its own store).
- **Bridge (no Orbit change needed).** `assets/bridge.js` runs before the page's scripts, on the server's own
  pages only (not on the gateway's `/__exe.dev/` pages, not in frames), and posts JSON through `window.ipc`.
  The app reads only the shapes in `bridge.rs`, clipped, and only from the server's pages.
  - Notifications: `Notification` and `ServiceWorkerRegistration.showNotification` become OS notifications;
    the permission reads "granted". Orbit's live socket already sends its `notify` signal to every open tab,
    and the web app shows it when its window is not in front (`apps/web/src/features/realtime/notify.ts`).
    So the app notifies for what the person's Orbit notification settings allow (DMs, mentions, threads),
    while it runs, also hidden in the tray. A click brings the window up and opens the message, the way the
    web app's own click does (the notice's id goes back to the bridge, which runs the page's click handler).
    Windows: the toast's sender is "Orbit" when the installer's Start-menu shortcut is there (it carries the
    AppUserModelID `Wellrox.Orbit`, which the app also sets on its process, `app_id.rs`); from the zip it is
    "Windows PowerShell" (a toast with an ID no shortcut carries does not show at all). The click works while
    the toast is up; a click on it later in the notification center starts `Orbit.exe` through the
    shortcut, which shows the running window (one instance) but does not open the message.
  - Unread badge: `navigator.setAppBadge` (the Badging API). The web app calls it with the same count as its
    favicon dot (`apps/web/src/lib/favicon.ts`, ships with the next web deploy). Until a server has that, the
    badge counts the notifications that came while the window was not in front, until it comes to the front.
    Windows: an overlay on the taskbar button, a red circle with the number (1 to 9, then 9+; a small red dot for
    `setAppBadge()` without a number), drawn at 16 px × the display scale (`src/overlay.rs`); macOS: the Dock
    badge; Linux: the Unity launcher count where there is one. The tray icon gets a red dot and its tooltip says the count.
  - Links: a click on a link to another site goes to the default browser; `target="_blank"` and `window.open`
    too (`route.rs`). `mailto:` and `tel:` go to their OS apps; `javascript:`, `file:` and others do nothing.
    A blank popup (`window.open('about:blank')`) is the exception: it opens as the web view's own window, in the
    same profile, because the web app loads an embedded app's sign-in in it ("Sign in to app"), and the app in
    the frame reads that sign-in's cookies from this web view, not from the browser (until 0.1.2 it was refused,
    and the web app said the sign-in window was blocked).
  - Update check: the page reads `/downloads/desktop/latest.json` with its own gateway cookies (at the start,
    every 6 hours, and on Check for updates); when that version is newer, a notification, a banner at the
    bottom right of the page (the bridge draws it in a shadow root, so it needs no web deploy; × hides it until
    the app quits) and the menus say so. Windows (0.1.8): "Restart to update" loads the installer URL (only on
    the server's own origin) in the web view, so its gateway cookies apply; the download goes to the temp
    folder, then the app runs it with `/SILENT /SUPPRESSMSGBOXES /NORESTART /relaunch=1` and quits; the
    installer's `[Run]` entry with `Check: Relaunch` starts Orbit again. macOS and Linux open the download in
    the browser. A Check for updates without an update says "up to date" (or "cannot check").
  - App state and menu (0.1.8): the bridge posts `ready`; the app answers with `__orbitDesktop.setState(…)`
    (version, OS, autostart and its label, update, updating, install) and the `orbit-desktop` window event.
    The web app's account menu → Desktop app (`apps/web/src/lib/desktopApp.ts`) shows the tray's items and
    sends `{kind:"action", name}` (`autostart`, `update`, `check`, `server`, `diagnostics`, `quit`): Windows 11
    hides new tray icons behind the ^, so the tray must not be the only place. The app's own pages may send
    only `server` and `home`. It works only on the saved server's own
    pages: `orbit.wellrox.com` redirects to `orbit.easytomanage.com` since 2026-11-12, so 0.1.4 loads that saved
    server as the new one (`settings.rs` `MOVED`); before, those apps saw no update, notification or badge.
- **Voice input** (the mic in the chat composer). Windows: WebView2's `webkitSpeechRecognition` does not work in the
  app. Up to 0.1.4 it failed with `network` (runtime 153+ sends speech to a service that answers 403,
  [WebView2Feedback#5724](https://github.com/MicrosoftEdge/WebView2Feedback/issues/5724); 0.1.5 turned that off with
  `signin::WEBVIEW2_ARGS`). On a Windows 11 PC with 0.1.6 it still gave `not-allowed` without any WebView2
  permission request and without opening the microphone. 0.1.7/0.1.8 used Windows' own dictation
  (`Windows.Media.SpeechRecognition`), which needs Settings → Privacy & security → Speech → "Online speech
  recognition" (`0x80045509` without it). Since 0.1.9 the bridge script replaces the page's
  `SpeechRecognition`/`webkitSpeechRecognition` with a class that runs in the app (`src/speech.rs`): start records
  the default microphone (cpal, WASAPI) and loads the model; while it records, a second thread transcribes parts of
  about 5 s (cut at the quietest moment of their last 2 s) with whisper.cpp (`whisper-rs`) on the computer, and each
  part's text goes into the field at once, so after stop only the last part is left. No cloud, no key, no Windows
  setting. 0.1.9 took several seconds after stop: whisper.cpp always encoded a whole 30 s window, and with timestamps
  it kept decoding that window's empty rest when the speech went on to the end (a 10 s clip: 15-20 s on the build VM).
  0.1.10 sets the encoder's context to the part's length (`speech::audio_ctx`), turns off timestamps and the
  temperature fallback, limits the tokens per part, prompts each part with the text before it, and keeps the loaded
  model for the app's run: on the build VM (4 shared cores) a 10 s clip takes 0.7 s, the last part after stop
  0.4-0.9 s. The page gets the microphone's peak `level` 10 times a second (the web app's meter) and `progress`. The model is Whisper
  base, multilingual, 5-bit (`ggml-base-q5_1.bin`, 59.7 MB), downloaded at the first use from a pinned Hugging Face
  commit into `<local app data>/orbit-desktop/models/`, checked against its SHA-256 (`speech::MODEL`); it records
  meanwhile, and the web app shows the download's percent beside the mic (a web app before 0.1.10 that does not
  listen to `onprogress` gets the bridge's own small status). The page's language picks
  Whisper's (`en-US` → `en`). `.cargo/config.toml` builds ggml for AVX2 (not the CI runner's CPU) and links the C
  runtime statically (no Visual C++ Redistributable). Every step (recording device and rate, download, model load
  time, each part's transcribe time, the time from stop to the text, errors, never the words) goes to the
  diagnostics log. The app still saves "allow" for the
  microphone of the server's origin in WebView2 (`src/microphone.rs`), for `getUserMedia`. When the recognizer says
  `not-allowed`, the web app checks the microphone itself (`getUserMedia`) and tries once more, and the toast names
  the cause, the error text and the app version (`__orbitDesktop.app`). macOS: WKWebView uses Apple's speech
  recognition and needs `NSMicrophoneUsageDescription` and `NSSpeechRecognitionUsageDescription` in
  `packaging/macos/Info.plist`; wry grants the media capture, macOS asks. Linux (WebKitGTK) has no recognizer: the
  mic shows "not available".
- **Diagnostics log** (tray or account menu → "Open diagnostics log", `<local app data>/orbit-desktop/diagnostics.log`, 64 KB max):
  start lines (version, server), the WebView2 runtime version, the saved microphone permission, every WebView2
  permission request (kind, page, answer), the page's voice input errors (`__orbitDesktop.log`), every menu action,
  the update steps, the connect page's failures, the first close to the tray and refused `file:` pages. No message text
  and no cookies. The update check runs at the start and every 6 hours (before 0.1.6: at the start only).
- **Downloads.** Every download goes to the Downloads folder (the home folder when there is none), with
  " (2)", " (3)" … when the name is taken; a notification says where, and a click on it shows the file
  (Explorer `/select`, Finder `open -R`, the folder on Linux). No save dialog.
- **Navigation.** The web view refuses `file:` pages: a file dropped outside a drop zone used to replace the
  Orbit page with the file. Ctrl + / Ctrl − / Ctrl 0 and Ctrl + wheel zoom (Windows; `with_hotkeys_zoom`).
- **One instance.** The first start locks `<data dir>/orbit-desktop/instance.lock` and listens on a loopback
  port (in `instance.port`). A later start sends `show` there and ends: the running window comes to the
  front, with no second window or tray icon. A later `--hidden` start (autostart) just ends. Change server
  frees the lock before it starts the app again.
- **Window and tray.** Closing the window hides it to the tray (the first time, a notification says that Orbit
  still runs; `close_hint_shown` in the settings); Quit Orbit (tray or account menu) ends the app. macOS: ⌘W
  closes, ⌘M minimizes (Window menu). A left click on
  the tray icon (Windows) or Open Orbit shows the window. The window's place, size and maximized state are
  saved on close and quit (a minimized or maximized window saves its last normal place, not Windows'
  -32000, -32000), and come back at the next start if they still fit on a monitor.
- **Start with Windows** (macOS: Open at login; Linux: Start at login): a check item in the tray menu, off
  by default. On, it writes `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\Orbit` (macOS:
  `~/Library/LaunchAgents/com.wellrox.orbit.plist`; Linux: `~/.config/autostart/orbit-desktop.desktop`)
  with `--hidden`, so the app starts in the tray. Each start writes it again while on (a moved app still starts).

## Commands

On the shared VM, run every build through `orbit-build` (see the root `AGENTS.md`), debug only. Linux needs
`libwebkit2gtk-4.1-dev` and `pkg-config` to build, and `libayatana-appindicator3-1` (or `libappindicator3-1`)
at run time for the tray. Windows needs the WebView2 runtime (Windows 10 and 11 have it).

```bash
cd apps/desktop
CARGO_INCREMENTAL=0 orbit-build cargo test --workspace     # bridge, routes, settings, autostart, gateway, contract
CARGO_INCREMENTAL=0 orbit-build cargo build -p orbit-desktop
target/debug/orbit-desktop                                  # the app (start page the first time)
target/debug/orbit-desktop --hidden                         # in the tray only
target/debug/orbit-desktop --check-gateway https://orbit.wellrox.com   # risk 4b, needs a person (below)
python3 -I scripts/fake-gateway.py PORT PASSWORD_FILE [forward|strip|reject|keepcookie|nogateway]  # local E2E server
scripts/check-gateway-smoke.sh target/debug/orbit-desktop   # --check-gateway end to end on the fake server
```

**Local check under Xvfb (2026-11-09, Linux only).** A private Orbit (`/usr/local/bin/orbit seed` + `serve`
on a fresh SQLite file in `/tmp`, port 18990, seed users only), Xvfb, `dbus-launch`, `stalonetray` (tray
host), `dunst` (notifications), `xdotool`: sign-in in the window, chat, tray menu, autostart on (entry
written), a DM from the second seed user → OS notification and a red dot on the tray icon, notification click →
the DM opens in the window, external link → `xdg-open`, two audit-log CSV exports → `orbit-audit.csv` and
`orbit-audit (2).csv`, window moved → Quit → same place at the next start, start page refuses `http://` to
another host, `--hidden` shows no window.

## Packaging (`.github/workflows/desktop.yml`)

Run by hand (Actions → Desktop → Run workflow) or push a tag `desktop-v<version>`. Jobs: Linux tests and the
`--check-gateway` smoke run; Windows release build (GUI program, no console, the Orbit icon from
`packaging/windows/orbit.ico` embedded by `build.rs`) → `Orbit.exe` in `Orbit-<version>-windows-x64.zip` and
in the installer `Orbit-<version>-windows-x64-setup.exe` (Inno Setup 6, `packaging/windows/orbit.iss`: per
user, no admin, `%LOCALAPPDATA%\Programs\Orbit`, Start-menu shortcut, optional desktop shortcut, uninstaller;
the job installs it silently, checks the files and shortcut, and uninstalls; all unsigned); macOS (`macos-14`, arm64, parked: only with the
`macos` input of a run by hand) release build → `Orbit.app` (ad-hoc
signed, `codesign -s -`) in `Orbit-<version>-macos-arm64.dmg`. Both packages run `--version` and
`scripts/check-gateway-smoke.sh` (the packaged `--check-gateway` against the fake server). The `manifest`
job writes `latest.json` (version, URLs under `<server>/downloads/desktop/`, sha256; `windows` is the
installer, `windows_zip` the zip; the macOS fields only when that job ran). It needs only the Windows job.
Publish with `deploy/desktop-downloads.sh [<run id>]` on the server: it checks the sha256 values and copies the
files to `/srv/orbit-downloads/desktop/`, which nginx serves as `/downloads/desktop/` (the nginx block is in the
script). The folder is outside the deploy, so deploys keep it. The icons come from `apps/web/public/logo.svg`
through `scripts/logo-icons.sh`.

- **Windows SmartScreen:** the exe is not signed, so on the first start Windows says "Windows protected your
  PC". Click **More info**, then **Run anyway**. Only a code-signing certificate removes this warning.
  `--check-gateway` in the package: `start /wait Orbit.exe --check-gateway https://orbit.wellrox.com` in a
  Command Prompt.
- macOS: first open needs right-click → Open (no notarization). `--check-gateway` in the package:
  `/Applications/Orbit.app/Contents/MacOS/orbit-desktop --check-gateway https://orbit.wellrox.com`.

## Open items

- **Tested on Linux only** (WebKitGTK under Xvfb). Windows and macOS build in CI (the Windows code also
  builds here with `--target x86_64-pc-windows-gnu`), but nobody has run the window, tray, toasts and their
  click, the AppUserModelID, the overlay badge, the Dock badge, the installer, autostart or WebView2
  downloads on a real machine.
- **macOS notification click**: the app waits for it the same way (notify-rust's synchronous
  `NSUserNotification` send, one waiting thread per notification until it is clicked or closed). Not tried
  on a Mac.
- **Windows: a toast clicked in the notification center** after it left the screen shows the window but does
  not open the message (no COM activator).
- **Gateway cookie lifetime**: when the exe.dev cookie expires, the gateway's sign-in shows in the window again.

## The gateway sign-in window and `--check-gateway`

`--check-gateway` signs in natively (bearer session) to answer risk 4b. When the server answers `307` to
`/__exe.dev/login` it runs `orbit-desktop --gateway-signin <server>`: a tao + wry window that loads the probe,
shows the gateway's sign-in, and when a page of the server's own origin finishes loading, writes the server's
cookies (`cookies_for_url`) as one JSON line to its stdout pipe (`orbit_client::cookies`, refuses a terminal).
Then Orbit's own sign-in: `POST /api/v1/auth/login` (+ `/login/two-factor`) without `Origin`, the token from
the body, `Authorization: Bearer` on every request and socket upgrade; secrets are `Secret`s (redacted
`Debug`/`Display`) and never go in a URL or a log. Sockets: `orbit-client/src/socket.rs` (one thread per
socket, blocking `tungstenite` with rustls/ring, backoff 1 … 30 s plus jitter, close `4401`/`4403` stop).

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
1. **Browser mode, no server change** (the web-view app since 2026-11-09 is this by nature: a browser on the server's own origin): send `Origin: <server>` and use Orbit's own session cookie
   (`__Host-orbit_session`, taken from the sign-in's `Set-Cookie`) instead of the bearer. Orbit accepts it as
   a browser session from its own origin; it fixes all three failures if the proxy forwards cookies.
2. **Small server change:** read the native session from a second header the proxy leaves alone (for example
   `X-Orbit-Session`), and let a request without `Origin` carry cookies when it has that header.
3. **Gateway:** ask exe.dev to forward `Authorization` / strip its cookie for this VM (not in our control).

**Not done in M1:** gateway cookie refresh from `Set-Cookie` (when the cookies expire the app opens the window
again); typing and presence frames; tested on Linux only (macOS and Windows build in CI).
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
`apps/server/tests/chat_api.rs:425`. The old native outbox (`orbit-store`, removed 2026-11-09) gave each entry one nonce for its life and sent one head per
conversation.

**Notifications (macOS, ad-hoc signed; found with Rune, before the switch to GPUI):** `rune-notify` posts only from a bundle (`mac.rs` `bundled()`: a bundle id
and a `.app` path) and asks permission on the first post. Rune's own installer signs ad hoc by default
(`scripts/mac_app_install.py`, `MAC_APP_SIGN` defaults to `-`). Reports say an ad-hoc-signed bundle can get
permission; a bare binary cannot. Not tested on a Mac yet.
