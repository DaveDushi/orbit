# Orbit for iOS

A native shell around the web client. The app asks for the address of an Orbit server and loads that server in a
`WKWebView`. It bundles no web code, so the server's own client always runs: a server upgrade updates the app with no
App Store release, and the `X-Orbit-Contract` check and same-origin session cookies work as they do in Safari.

## Build

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
cookies, which Orbit allows only there.

## What the shell does

| Area | Behavior |
|---|---|
| Server picker | Accepts `orbit.example.com` or a full URL, keeps only the origin, and checks that `GET /api/v1/setup/status` answers like Orbit. Recent servers are listed; swipe one to forget it and delete its data. |
| Storage | Each server gets its own `WKWebsiteDataStore`, so cookies and local storage never mix between servers. |
| Links | Pages of the server stay in the app, including `target="_blank"`. Other sites, `mailto:` and `tel:` open outside the app. Frames (embeds) load normally. |
| Downloads | `<a download>` links and `Content-Disposition: attachment` responses download to a temporary file and open the share sheet. |
| Failures | A failed page load shows **Retry** and **Change server**. A crashed web process reloads. |
| Switch server | Long-press the app icon and choose **Switch server**. |
| Layout | The page handles safe areas itself (`viewport-fit=cover`); the keyboard shrinks the page. |
| Web Inspector | Debug builds are inspectable from Safari on the Mac. |

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
