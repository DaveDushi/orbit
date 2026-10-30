# Apps

Add the web apps you build to Orbit, so that your team has one place to open them, with one login. Orbit does not host, deploy, restart or monitor them.

## Add an app

Workspace settings → **Apps** (owners and admins).

| Field | What it does |
| --- | --- |
| Name, icon | The entry in the sidebar. |
| Address | An absolute `https://` URL without a user name or password. Orbit refuses its own origin: an app must have its own host name. |
| Opens | **Inside Orbit**: a full-size frame at `/apps/<slug>`. **In a new tab**: the sidebar entry opens the address directly. |
| Who may open it | The roles that see the entry. Other members get a 404. The agent cannot use apps. |
| Redirect URIs | Turn on "Sign in with Orbit". Leave them empty if the app has its own login. |

Changes apply at once, without a restart. On a phone, the Orbit app lists these apps in Settings and opens them in the browser.

## Allow Orbit to frame the app

To open inside Orbit, the app must let Orbit frame it. Send this header on its pages, with Orbit's origin:

```
Content-Security-Policy: frame-ancestors https://orbit.example.com
```

Remove any `X-Frame-Options` header. A page with neither header can be framed by every site, so add the header even if the frame works without it. Orbit cannot detect reliably that an app refused the frame, so the page always has **Sign in to app** and **Open in a new tab** in its header.

In the frame, the app can run scripts, use forms, open popups and download files. It cannot navigate the Orbit page. The camera, microphone, location and payment features are off.

## Use a subdomain of the same domain

Host the app on a subdomain of Orbit's domain: for Orbit at `orbit.example.com`, use `reports.example.com`. This decides whether the app's **own** cookies work in the frame:

- **Same site** (`reports.example.com`): the app's cookies work. Sign-in in the frame is silent.
- **Other site** (`reports.other.net`): Orbit is the top page, so its session cookie is sent and "Sign in with Orbit" succeeds, but the browser treats the app's cookies as third-party. A `SameSite=Lax` session cookie never comes back, and an app that signs in again each time loops forever. Then choose one of these:
  - Set the app's session cookie to `SameSite=None; Secure; Partitioned`, on HTTPS. Browsers with strict third-party cookie settings can still block it.
  - Set the app to **In a new tab**. This always works.

If a person has no Orbit session when the frame signs in (it ended in another tab), Orbit cannot show its login in a frame. It answers the app with `error=login_required`, so the app can offer a link that signs in at the top level. Without a frame, Orbit sends the person to its login and back.

## Add "Sign in with Orbit"

Enter the app's redirect URI, then copy the values from the app's card. The client secret shows **only once**. **Make a new secret** replaces it, and ends all tokens of the app.

| Setting | Value |
| --- | --- |
| Discovery URL | `https://orbit.example.com/.well-known/openid-configuration` |
| Issuer | `https://orbit.example.com` |
| Flow | Authorization code. PKCE (`S256`) is optional and recommended. |
| Client authentication | `client_secret_basic` or `client_secret_post` |
| Scopes | `openid profile email` |
| ID token | RS256; keys at `/oauth/jwks` |

Any OpenID Connect library works. Redirect URIs must match exactly: path, query and trailing slash. There is no consent screen, because an admin approved the app. Each sign-in checks that the person is an active member with an allowed role.

The ID token and `/oauth/userinfo` give these claims:

| Claim | Scope | Meaning |
| --- | --- | --- |
| `sub` | `openid` | The stable ID of the user. Use this to identify people, not the email. |
| `workspace_id`, `workspace_role` | `openid` | The workspace of the app, and `owner`, `admin` or `member`. |
| `email`, `email_verified` | `email` | |
| `name`, `picture` | `profile` | `picture` loads only for a browser that is signed in to Orbit in the same workspace, so copy nothing from it on the server. |

Example with [`openid-client`](https://github.com/panva/openid-client) for Node:

```js
import * as client from 'openid-client'

const config = await client.discovery(new URL('https://orbit.example.com'), CLIENT_ID, CLIENT_SECRET)

// Start of the sign-in: keep verifier, state and nonce in the visitor's session.
const verifier = client.randomPKCECodeVerifier()
const state = client.randomState()
const nonce = client.randomNonce()
const url = client.buildAuthorizationUrl(config, {
  redirect_uri: 'https://reports.example.com/auth/callback',
  scope: 'openid profile email',
  code_challenge: await client.calculatePKCECodeChallenge(verifier),
  code_challenge_method: 'S256',
  state,
  nonce,
})

// On the callback: this checks the signature, issuer, audience and nonce.
const tokens = await client.authorizationCodeGrant(config, currentUrl, {
  pkceCodeVerifier: verifier,
  expectedState: state,
  expectedNonce: nonce,
})
const person = tokens.claims()
```

[`scripts/apps-demo/server.mjs`](../scripts/apps-demo/server.mjs) is a complete small app without packages. It also handles `login_required` and a blocked cookie.

## Protect an app with Cloudflare Access

An app behind [Cloudflare Access](https://developers.cloudflare.com/cloudflare-one/integrations/identity-providers/generic-oidc/) needs no code: Access uses Orbit as a generic OpenID Connect provider, and people sign in to the app with their Orbit account. In this example, the app is `products.wellrox.com` and the Access team is `edusi`.

**1. In Orbit**, add the app with the address `https://products.wellrox.com`, and this redirect URI:

```
https://edusi.cloudflareaccess.com/cdn-cgi/access/callback
```

Copy the client ID and the secret.

**2. In Cloudflare**, go to Zero Trust → Settings → Authentication → Add new → OpenID Connect:

| Cloudflare field | Value |
| --- | --- |
| App ID | The client ID from Orbit |
| Client secret | The secret from Orbit |
| Auth URL | `<Orbit address>/oauth/authorize`: the browser goes here |
| Token URL | `<public address>/oauth/token`: Cloudflare's servers call this |
| Certificate URL | `<public address>/oauth/jwks`: Cloudflare's servers call this |
| Proof Key for Code Exchange (PKCE) | On |
| Email claim | `email` (the default scopes `openid email profile` include it) |

`<Orbit address>` is `ORBIT_PUBLIC_ORIGIN`. The ID token's `iss` is always this address, also when the Token URL is on another address. `<public address>` is the gateway below if Orbit is behind a login, else the same as `<Orbit address>`.

**3. In the Access application** of `products.wellrox.com`, choose this login method and a policy. Add `workspace_role` under OIDC Claims if a policy needs the role.

**Which addresses must be public.** Cloudflare's servers call Orbit without a browser session, so nothing may ask them to sign in:

| Path | Called by | Must be public |
| --- | --- | --- |
| `POST /oauth/token` | Cloudflare's servers | Yes |
| `GET /oauth/jwks` | Cloudflare's servers | Yes |
| `/oauth/authorize`, `/login`, the web app | The person's browser | No, if everyone who uses the app can pass the login in front of Orbit |

Cloudflare's documentation names only these URLs: you enter them yourself, so it does not need discovery, and it does not mention userinfo. Other OpenID Connect clients may also need `/.well-known/openid-configuration` and `/oauth/userinfo`.

### A public gateway when Orbit is behind a login

If a login proxy protects Orbit and has no setting per path, run the gateway in [`deploy/oidc-gateway/`](../deploy/oidc-gateway/) on a port of its own. It is a separate nginx process that allows exactly `POST /oauth/token` and `GET /oauth/jwks` and gives a 404 for everything else, without calling Orbit. It compares the raw request line, so encoded or traversal paths do not match. It forwards only `Content-Type`, `Content-Length`, `Authorization`, and its own `X-Forwarded-For` and `X-Forwarded-Proto`: no cookie, no `Origin`, no header of the login proxy. It keeps no access log and no body on disk.

Orbit gives these requests no extra trust. A valid client secret is still needed for a token, and the existing limit of 30 token requests a minute per client address applies. The gateway passes the last `X-Forwarded-For` entry as the client address, which must be the entry that the proxy in front adds. If all requests counted as one address, one attacker could use up the 30 requests and stop sign-in for everyone until the next minute.

**Deploy:**

```sh
sudo mkdir -p /etc/orbit-oidc-gateway
sudo cp deploy/oidc-gateway/nginx.conf /etc/orbit-oidc-gateway/nginx.conf
# Set the two values in that file: `listen` (8400) and the Orbit address in `proxy_pass` (127.0.0.1:8080).
sudo cp deploy/oidc-gateway/orbit-oidc-gateway.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now orbit-oidc-gateway
```

Orbit's `ORBIT_TRUSTED_PROXIES` must contain the address the gateway connects from (`127.0.0.1/32` here). Do not add the proxy in front to it. Then make the port public at the proxy.

**Check it** from outside, with the public address:

```sh
curl -s -o /dev/null -w '%{http_code}\n' "$PUBLIC/oauth/jwks"              # 200
curl -s -o /dev/null -w '%{http_code}\n' "$PUBLIC/login"                   # 404
curl -s -o /dev/null -w '%{http_code}\n' --path-as-is "$PUBLIC/oauth/../api/v1/auth/me"  # 404
# The proxy must add the real client last. A forged value must not avoid the limit: at about
# the 31st request this prints 429. If it never does, stop: the client address can be forged.
for i in $(seq 35); do curl -s -o /dev/null -w '%{http_code} ' -X POST -H "X-Forwarded-For: 198.51.100.$i" -d x=1 "$PUBLIC/oauth/token"; done
```

**Test the rules** on any machine with `deploy/oidc-gateway/test.sh`.

**Turn it off:** make the port private again at the proxy, then:

```sh
sudo systemctl disable --now orbit-oidc-gateway
sudo rm /etc/systemd/system/orbit-oidc-gateway.service
sudo rm -r /etc/orbit-oidc-gateway
sudo systemctl daemon-reload
```

Sign-in through Cloudflare stops at once. Nothing else in Orbit changes.

**On exe.dev.** Its proxy forwards ports 3000 to 9999, and only one port can be public. That one port is the one set with `share port`, which also serves the root URL (`https://<vm>.exe.xyz/`). Other ports stay at `https://<vm>.exe.xyz:<port>/` for people with access to the VM. So the gateway cannot simply be public next to an Orbit on the root URL: read [`.ai/apps-design.md`](../.ai/apps-design.md), "exe.dev".

### Run the app inside Orbit behind Access

The Access login page refuses frames (`X-Frame-Options: DENY`). So the frame must never have to show it:

1. **Use Instant Auth.** In the Access application, turn on **Apply instant authentication** with Orbit as the only login method ([Cloudflare's docs](https://developers.cloudflare.com/fundamentals/manage-members/dashboard-sso/)). Access then redirects straight to Orbit. A browser checks frame refusal only on the final page, not on redirects, so a signed-in person passes through: the app, Access, `/oauth/authorize`, the Access callback, the app. Orbit allows each step: the frame may use the app's origin, Orbit, and the origin of the redirect URI (`edusi.cloudflareaccess.com`).
2. **Let the app allow frames from Orbit.** Set `allow_iframe` in the Access application, and send `Content-Security-Policy: frame-ancestors <Orbit address>` on the app's pages, for example with a Cloudflare response header rule. Remove `X-Frame-Options`. This needs no change in the app's code.
3. **Use a subdomain of one domain** for Orbit and the app, for example `orbit.wellrox.com` and `products.wellrox.com`. See the cookie table.

**When the frame cannot sign in**, each embedded app's page has **Sign in to app** in its header. It opens the app in a popup, where the login pages work normally. Sign in once and close the popup: the frame reloads. If the browser blocks the popup, the page says so. If the frame stays empty after that, use **Reload**. Orbit cannot detect a refused frame, so the control is always there.

If a person has no Orbit session when the frame reaches `/oauth/authorize`, Orbit does not send the frame to its login, which would be refused: it answers Access with `error=login_required`, and Access shows an error or a refused page in the frame. **Sign in to app** is the way out: the popup is a normal page, so Orbit shows its login there.

| Cookie | Set by, for | Orbit and the app on one domain | On different sites |
| --- | --- | --- | --- |
| `__Host-orbit_session` (`SameSite=Lax`, `Secure`, one host) | Orbit | Sent in the frame, because Orbit is the top page. The app's host never gets it. | The same. |
| `CF_Authorization` on the app's host | Access | Sent in the frame. The popup's sign-in also counts in the frame. | A third-party cookie. It needs SameSite None in the Access application, and browsers can block it. Firefox keeps it apart per top site, so a popup's sign-in may not count in the frame. |
| `CF_Authorization` on `<team>.cloudflareaccess.com` | Access | A third-party cookie in the frame (`SameSite=None`), always. If a browser blocks it, Instant Auth in the frame can fail. Then use **Sign in to app**. | The same. |
| The login cookie of a proxy in front of Orbit, such as exe.dev | The proxy | Needed for every step on Orbit's address. If the proxy's cookie is `SameSite=Lax` or looser, it should behave like Orbit's. Without it, the proxy's login page is refused in the frame; open Orbit again in the top page. | The same. |

**Safari** blocks third-party cookies by default. Expect the team-domain cookie to fail there, and on different sites the app's cookie too: use one domain, and test with your own account. Orbit's own tests run only in Chromium.

**A removed member** loses Orbit sign-in at once, but the Access session lasts for its own session duration. Keep that duration short.

## Limits

- **Tokens give no access to Orbit.** The access token works only at `/oauth/userinfo`, for 10 minutes. There are no refresh tokens. To let the agent call an app, add it in Workspace settings → **Agent tools**; an app's page links there for admins.
- **The app's own session is the app's.** When a member is removed or a role is no longer allowed, Orbit deletes or refuses their tokens at once, and they cannot sign in again. A session the app already made lasts until the app ends it. Keep app sessions short, or check `/oauth/userinfo` while the token lasts.
- **Removing an app** deletes its client and tokens. The app itself keeps running.
- **`prompt=login` is not supported**: Orbit answers `login_required`. `max_age`, `request` and `request_uri` are not supported.
- **The frame passes only through the app, its redirect URIs and Orbit.** An app that signs in through another identity provider inside the frame is blocked; use **Sign in to app**, or add that provider's callback as a redirect URI.
- **The server needs `ORBIT_APP_KEY`** for sign-in: it seals the signing key. Without it, apps still open, without sign-in. If the key changes, sign-in stops until you restore it.
