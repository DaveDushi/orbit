# Orbit Mobile

The iOS and Android app for Orbit, in `apps/mobile` of the Orbit monorepo. Built with Expo and React Native.

The app signs in to an Orbit server and works in one workspace at a time: Home (greeting, my tasks, unread counts), Tasks (by status, task detail with status change and comments, new task), Chat (channels and DMs, read and send messages), Inbox (notifications) and Account (workspace switcher, sign out).

## Code map

- `src/data/`: no React. `orbit.ts` has every API call over the generated SDK (`@orbit/api`), `keys.ts` every query key, and `tasks.ts`, `chat.ts`, `inbox.ts`, `markdown.ts`, `live.ts`, `socket.ts` the logic the screens and the live sockets use. Unit tests sit next to them.
- `src/session-context.tsx`: the stored session and one API client per session. A 401 signs out.
- `src/orbit-context.tsx`: `useOrbit()` gives the client, the user and the current workspace (the last one chosen, else the first).
- `src/live.ts`: the workspace events socket (marks task, project and inbox queries stale) and the chat live socket (patches messages and read states). Both close in the background and reconnect in the foreground.

## How the app talks to Orbit

- The app sends no `Origin` header, so the server treats it as a native client.
- `POST /api/v1/auth/login` (and `/login/two-factor` when 2FA is on) returns `session_token` in the body. No cookie is set.
- The app keeps the token in the device keychain (`expo-secure-store`). It sends the token as `Authorization: Bearer <token>` on every request and on the WebSocket upgrade.
- The token is a normal Orbit session. It shows in Account → Sessions, and you can revoke it there.

The server side lives in `apps/server` of the same repo.

## Develop

Requirements: Bun, the Orbit dev server (`just dev` at the repo root), and a phone with **Expo Go**, or an iOS simulator or Android emulator.

1. Install dependencies at the repo root. The web and mobile apps share one Bun workspace and one React version.

   ```bash
   bun install
   ```

2. Set the server the sign-in screen fills in.

   ```bash
   cp apps/mobile/.env.example apps/mobile/.env.local
   ```

   The phone must reach this URL. Use one of these:

   - **Tailscale** (recommended). Put the phone on the same tailnet and use the dev server's `https://<host>.ts.net:<port>` URL. The Vite dev server proxies `/api` and WebSockets to the Rust server.
   - **LAN.** Use `http://<computer-LAN-IP>:8888`. Vite must listen on `0.0.0.0`.

3. Sign in to Expo. iOS Expo Go only opens a project when Expo CLI and Expo Go use the same account. On a remote server, `bunx expo login` opens a browser that calls back to `localhost`, so use `bunx expo login --no-browser`, or put a **personal** access token in `.env.local` as `EXPO_TOKEN` (a robot token does not work). Sign in to the same account in Expo Go.

4. Start Metro.

   ```bash
   just mobile               # Metro on port 8082, advertised as ORBIT_MOBILE_HOST
   ```

   If the phone cannot reach the computer directly, run `bunx expo start --tunnel` in `apps/mobile`.

5. Open the app in Expo Go. The phone must be on the tailnet. Use one of these:

   - Tap the server under **Development servers** on the Expo Go Home tab (shows when both sides use the same Expo account).
   - iOS: type `exp://devserver.tail661ee3.ts.net:8082` in the Safari address bar and open it in Expo Go.
   - Android: tap **Enter URL manually** in Expo Go and enter the same URL.
   - Scan the QR code that Metro prints.

   `just mobile` sets `REACT_NATIVE_PACKAGER_HOSTNAME` from `ORBIT_MOBILE_HOST`, so Metro tells the phone to load the app from that host.

6. Sign in with a seed account: `member@example.com` / `password` (create it with `just seed`).

You should see the Home tab. Account shows your name, email and server, and signs you out.

## Checks

```bash
cd apps/mobile
bunx tsc --noEmit
bunx tsc --noEmit -p scripts
bun test
bun scripts/api-check.ts   # a running server (default http://127.0.0.1:18080, test@example.com / password); creates a task, a comment and a message, then deletes them
bunx expo-doctor
```

## Theme and icons

The colors, radii and icons come from the web app. Do not edit the generated files; regenerate them from the repo root after the web changes:

```bash
bun apps/mobile/scripts/generate-theme.ts   # apps/web/src/index.css → src/theme/tokens.gen.ts
bun apps/mobile/scripts/generate-icons.ts   # reicon-react Outline icons → src/ui/icons.gen.ts
```

To add an icon, add its reicon name to `ICONS` in `scripts/generate-icons.ts` and rerun the script. Use the name the web imports.

`just check` at the repo root also runs the mobile typecheck.

## Next steps

- Move to a development build (`bunx expo run:ios|android`, or EAS) when the app needs native modules that Expo Go does not include, for example APNs/FCM push.
- Browser-sheet sign-in for passkeys and SSO.
