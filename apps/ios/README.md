# Orbit for iOS

A native shell around the web client, with a Photos Share Extension for chat messages and task attachments.
The app asks for the address of an Orbit server and loads that server in a `WKWebView`. It bundles no web code, so the server's own client always runs: a server upgrade updates the app with no
App Store release, and the `X-Orbit-Contract` check and same-origin session cookies work as they do in Safari.

## Build on Linux (Omarchy)

Install the **full** [omarchy-apple-dev](https://github.com/joshuaswarren/omarchy-apple-dev) toolchain first:
`swift sdk list` must include `darwin`. The no-Xcode SDK does not cover this app's WebKit and SwiftUI APIs.
Python 3 with PyYAML is also required (`python3 -c 'import yaml'`).

```sh
cd apps/ios
./build-linux.sh
# Unsigned device app: xtool/Orbit.app
```

The script reads runtime metadata and versions from `project.yml`, then uses omarchy-apple-dev's `ship.sh --device`
to compile the app and place its icon and color assets in the main bundle, as Xcode does. It defaults to a debug
build; use `CONFIGURATION=release ./build-linux.sh` for release. If the toolchain checkout is not at
`~/omarchy-apple-dev`, set `OMARCHY_APPLE_DEV` to its path.

To sign and install on a USB-connected iPhone, unlock it and accept Trust if prompted:

```sh
xtool-1.22.2 auth login --mode password  # Enter Apple credentials privately
./install-linux.sh IPHONE_UDID
```

The installer needs `pymobiledevice3`, `rcodesign`, and `zip` on PATH. Set `PYMOBILEDEVICE3=/path/to/pymobiledevice3`
if it is installed in a venv. It reuses valid device profiles and the cached signing identity; when profiles need
provisioning, it uses `xtool-1.22.2` (override with `XTOOL_INSTALLER`). It then signs both targets with the same explicit
keychain group and verifies their final code-signature entitlements before installing. Temporary keys, profiles and
signed copies are removed on exit. **Don't use a plain `xtool install` for the final build:** xtool replaces each target's
keychain group with its own bundle ID, which breaks the extension's access to the app's sign-in.

If password login fails with **-80045**, older xtool builds use a login fingerprint Apple now rejects
([upstream issue](https://github.com/xtool-org/xtool/issues/329)). The fix is in **1.22.2**. Keep Omarchy's patched
xtool for building; use the fixed release for both authentication and installation. For example, if installed
alongside it as `xtool-1.22.2`, use that version to sign in and `install-linux.sh` to install.
Apple may identify that login as a **Pixel 7**, which is expected.
Both versions share login storage, but use different authentication provider IDs. Do not run the older
`xtool auth` commands (including `status`) after signing in with 1.22.2: they can clear the newer session.
The Linux build script isolates the older builder's Foundation home to prevent unsigned builds from clearing it too.

xtool 1.22.2 also compares certificate serials inconsistently when Apple's value starts with `0`, and can unnecessarily
ask to revoke a valid certificate. **Cancel rather than revoke it.** `tools/xtool-certificate-serial.patch` normalizes both
sides of the comparison; apply it to a separate 1.22.2 source checkout and build that installer alongside the originals.
On this development machine that fixed installer is `xtool-1.22.2-orbit`; use
`XTOOL_INSTALLER=xtool-1.22.2-orbit ./install-linux.sh IPHONE_UDID` when new profiles are needed. Neither original xtool
installation needs to be replaced, and no existing certificate needs to be revoked.

Enable **Settings > Privacy & Security > Developer Mode** on the phone and trust the development certificate under
**Settings > General > VPN & Device Management** if prompted. Open Orbit on the phone. For command-line launch/debugging,
see omarchy-apple-dev's `device-run.sh` documentation; iOS 17+ needs a developer tunnel and disk image.

`xtool` uses the bundle ID in `xtool.yml`, not `Config/Local.xcconfig`. Free accounts may prefix that ID when signing.
This is a physical-device build: Apple's iOS Simulator does not run on Linux, and a successful build alone does not
prove on-device behavior. No App Store upload is performed.

## Build on macOS

You need a Mac with Xcode 16 or later and [XcodeGen](https://github.com/yonaskolb/XcodeGen). The Xcode project is
generated and not committed.

```sh
brew install xcodegen
cd apps/ios
cat > Config/Local.xcconfig <<'EOF'
DEVELOPMENT_TEAM = YOUR_TEAM_ID
PRODUCT_BUNDLE_IDENTIFIER = com.example.orbit
EOF
xcodegen generate
open Orbit.xcodeproj
```

Run `xcodegen generate` again after adding or removing files or editing `project.yml`.

The manual `iOS` GitHub workflow generates the project and builds it for the simulator without signing.

## Try it against a dev server

Start `just dev` and run the app in the Simulator. Enter `http://localhost:8888`: the Simulator shares the Mac's
network, and plain `http` is accepted only for local hosts (`localhost`, `*.local`, bare names and IP addresses).

A physical phone needs a server it can reach over `https`. The dev server listens on loopback only and sets insecure
cookies, which Orbit allows only there. On a phone, `localhost` refers to the phone, not the Linux/Mac host; use a
reachable HTTPS Orbit server for device testing rather than changing the server's cookie security.

## What the shell does

| Area | Behavior |
|---|---|
| Server picker | Accepts `orbit.example.com` or a full URL, keeps only the origin, and checks that `GET /api/v1/setup/status` answers like Orbit. Access gateways (such as exe.dev) open in a cancellable sign-in sheet first. Recent servers are listed; swipe one to forget it and delete its data. |
| Storage | Each server gets its own `WKWebsiteDataStore`, so cookies and local storage never mix between servers. |
| Links | Pages of the server stay in the app, including `target="_blank"`. Other sites, `mailto:` and `tel:` open outside the app. Frames (embeds) load normally. |
| Downloads | `<a download>` links and `Content-Disposition: attachment` responses download to a temporary file and open the share sheet. |
| Share from Photos | Photos → Share → Orbit opens a native preview and server/workspace picker. Choose Chat / DM (with an optional message) or Task, select an existing destination, then Send / Attach. |
| Failures | A failed page load shows **Retry** and **Change server**. A crashed web process reloads. |
| Switch server | Long-press the app icon and choose **Switch server**. |
| Layout | The page handles safe areas itself (`viewport-fit=cover`); the keyboard shrinks the page. |
| Web Inspector | Debug builds are inspectable from Safari on the Mac. |

## Access gateways and connection tests

A private server behind exe.dev or another login gateway does not need to be made public. Connect to its normal
HTTPS address and sign in in the sheet. The sheet shows the current page's hostname; use **Cancel** to leave or
**Check again** after signing in if the gateway does not return automatically. For an emailed magic link, copy the
link from your mail app, tap **Use email sign-in link**, paste it, and choose **Open link**. The complete link opens
in the current in-app browser with the same cookies, not in Safari. Links must use HTTPS and are not saved to Recent.
Verification and the Orbit web view share the server's isolated WebKit data store, including gateway cookies. A login page is never saved as the Orbit
server: the setup endpoint must still return valid Orbit JSON. Gateway passkeys have the same embedded-web-view
restriction noted below; use the provider's email-code or password flow when offered.

Run the Foundation-only connection, cookie isolation, upload and delivery tests plus the Python signing tests on Linux
or macOS without a simulator (Swift, XCTest and Python 3 are required). The HTTP fixture uses loopback and synthetic
photos; it doesn't write to a real Orbit server:

```sh
cd apps/ios
./test.sh
```

Device smoke checks: connect to a direct Orbit server; connect through the gateway and sign in; relaunch and confirm
the session survives; cancel a new gateway connection and confirm it wasn't added to Recent. Other sites should
still open externally once inside Orbit, and forgetting a server should still delete its isolated data.

## Sharing photos into Orbit

1. Open the main app once after installing and sign in to the server. Your existing WebKit session is reused.
2. In Photos select 1–10 photos, tap Share, and choose Orbit. If it isn't visible, look under **More** and enable it.
3. Select the server and workspace, then **Chat / DM** or **Task**. Chat lists your writable, unarchived conversations
   (including existing DMs); tasks support server-side search and pagination. No destination is selected automatically.
4. Pick the destination and tap **Send** / **Attach**. The sheet closes only after the server confirms delivery.

Photos are resized to at most 2048 pixels and encoded as JPEG, including HEIC inputs, so the web client can display them.
Only the explicitly shared items are read; no whole-library Photos permission is requested. Temporary images are protected
while the phone is locked and removed when the share ends. Videos, arbitrary files, creating new tasks/DMs and offline
queueing are not part of this flow.

The main app observes its per-server WebKit cookie store and snapshots only that server's unexpired cookies (including
HttpOnly and gateway cookies) into a device-only, unlocked-only shared keychain group. The extension's native requests
carry only the selected server's cookies, with the matching Origin header; they never follow redirects or send photos to
a login provider. An expired Orbit/gateway session asks you to reconnect in the main app. Signing out removes that server's
snapshot, and forgetting a server deletes it before reporting success. No browser JavaScript receives credentials, and
no App Group or backend deployment is required. Both signed targets must retain the same first keychain access group.

Chat retries reuse acknowledged upload IDs and the original message nonce. Task uploads are individual, non-atomic writes:
a failure shows how many were confirmed and asks you to inspect the task rather than blindly retrying a possibly committed
upload. Nothing is automatically sent, and canceling before Send / Attach makes no upload requests.

Device smoke checks still require Photos on a real phone: send a single and multiple photos to a chat/DM and a task,
confirm the right destination and account, cancel without posting, try task search/pagination, then sign out or forget a
server and confirm it is no longer available to the extension. Also test a lost connection/expired gateway session: the
sheet must not report success, follow a login redirect with a photo, or silently duplicate a chat message.

## Contract with the server

The shell relies on these, so keep them stable:

- Orbit is served from the root of its origin.
- `GET /api/v1/setup/status` returns `200` with `{"complete": <bool>}` and no session.
- The user agent ends with `OrbitiOS/<app version>`. The web client can use it to tell the app from Safari.

## Not done yet

- **Push notifications.** `WKWebView` has no Web Push, so Orbit's notification setting is hidden here (it checks for
  `PushManager`). Push needs APNs, a relay that holds the APNs key, and a device-token endpoint in Orbit.
- **Passkeys.** A web view can only use passkeys for domains listed in the app's Associated Domains, which a
  self-hosted server can't be. Password login works. Passkeys need login through `ASWebAuthenticationSession` and a
  one-time session handoff endpoint in Orbit.
- **Installing the GitHub app** opens github.com in Safari, and GitHub's redirect back lands in Safari, not the app.
- **App Review** will also want a demo server and credentials, and likely in-app account deletion.
