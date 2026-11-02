# Apps

Add the web apps you build to Orbit, so that your team has one place to open them, with one login. Orbit does not host, deploy, restart or monitor them.

## Add an app

Workspace settings → **Apps** (owners and admins) lists the apps. **Add app** asks only for the name, the address and who can see it. Select an app to change the rest.

| Field | What it does |
| --- | --- |
| Name, icon | The entry in the sidebar. |
| Address | An absolute `https://` URL without a user name or password. Orbit refuses its own origin: an app must have its own host name. |
| Opens | **Inside Orbit**: a full-size frame at `/apps/<slug>`. **In a new tab**: the sidebar entry opens the address directly. |
| Who can see it | The roles that see the entry. Other members get a 404. The agent cannot use apps. |
| Sign in with Orbit | On: enter the redirect URIs. Leave it off if the app has its own login. |

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

Turn on **Sign in with Orbit**, enter the app's redirect URI, then copy the values from the **Sign-in** section of the app's settings. The client secret shows **only once**. **Make a new secret** replaces it, and ends all tokens of the app.

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

[`scripts/apps-demo/server.mjs`](../scripts/apps-demo/server.mjs) is a complete small app without packages. It also handles `login_required` and a blocked cookie. In Orbit's frame, `login_required` comes from a small Orbit page that tells Orbit (so it shows the card "Sign in to <app>") and then goes to the redirect URI; the app sees the same callback as with the redirect.

## Protect an app with Cloudflare Access

An app behind [Cloudflare Access](https://developers.cloudflare.com/cloudflare-one/integrations/identity-providers/generic-oidc/) needs no code: Access uses Orbit as a generic OpenID Connect provider, and people sign in to the app with their Orbit account. In this example, the app is `products.wellrox.com` and the Access team is `edusi`.

**1. In Orbit**, add the app with the address `https://products.wellrox.com`, and this redirect URI:

```
https://edusi.cloudflareaccess.com/cdn-cgi/access/callback
```

Copy the client ID and the secret from the app's settings.

**2. In Cloudflare**, go to Zero Trust → Settings → Authentication → Add new → OpenID Connect:

| Cloudflare field | Value |
| --- | --- |
| App ID | The client ID from Orbit |
| Client secret | The secret from Orbit |
| Auth URL | `<Orbit address>/oauth/authorize`: the browser goes here |
| Token URL | `https://orbit-oidc.wellrox.com/oauth/token`: Cloudflare's servers call this, through the tunnel below |
| Certificate URL | `https://orbit-oidc.wellrox.com/oauth/jwks`: Cloudflare's servers call this, through the tunnel below |
| Proof Key for Code Exchange (PKCE) | On |
| Email claim | `email` (the default scopes `openid email profile` include it) |

`<Orbit address>` is `ORBIT_PUBLIC_ORIGIN`, which stays behind its login. If Orbit has no login in front of it, use that address for all three URLs and skip the tunnel. The ID token's `iss` is always `ORBIT_PUBLIC_ORIGIN`, whichever host served the token. Cloudflare's [generic OIDC documentation](https://developers.cloudflare.com/cloudflare-one/integrations/identity-providers/generic-oidc/) has no issuer field and does not say if it checks `iss`, so only a real sign-in proves this.

**3. In the Access application** of `products.wellrox.com`, choose this login method and a policy. Add `workspace_role` under OIDC Claims if a policy needs the role.

**Which addresses must be public.** Cloudflare's servers call Orbit without a browser session, so nothing may ask them to sign in:

| Path | Called by | Must be public |
| --- | --- | --- |
| `POST /oauth/token` | Cloudflare's servers | Yes, through the tunnel |
| `GET /oauth/jwks` | Cloudflare's servers | Yes, through the tunnel |
| `/oauth/authorize`, `/login`, the web app | The person's browser | No, if everyone who uses the app can pass the login in front of Orbit |

Cloudflare's documentation names only these URLs: you enter them yourself, so it does not need discovery, and it does not mention userinfo. Other OpenID Connect clients may also need `/.well-known/openid-configuration` and `/oauth/userinfo`.

### Reach Orbit through a Cloudflare Tunnel when it is behind a login

If a login proxy protects Orbit, Cloudflare's servers cannot reach it. Run [`deploy/oidc-gateway/`](../deploy/oidc-gateway/) on the machine: `cloudflared` opens an outbound connection to Cloudflare, and calls a small nginx gateway on `127.0.0.1:8400`. Orbit's address and its login stay as they are.

The gateway allows exactly `POST /oauth/token` and `GET /oauth/jwks`, and gives a 404 for everything else without calling Orbit. It compares the raw request line, so encoded or traversal paths do not match. It forwards only `Content-Type`, `Content-Length`, `Authorization`, and its own `X-Forwarded-For` and `X-Forwarded-Proto`. It keeps no access log and no body on disk. Orbit gives these requests no extra trust: a token still needs a valid client secret.

**The limit of 30 token requests a minute.** Orbit counts per client address, and the gateway takes that address only from `CF-Connecting-IP`, which Cloudflare's edge sets. `X-Forwarded-For` is ignored, because Cloudflare keeps what the caller wrote there. Each sign-in makes one token request, and the caller is Cloudflare's servers. So:

- Expect at most 30 sign-ins a minute through Access, if Cloudflare calls from one address. More can get a 429, and the person sees a login error from Access until the next minute. `ORBIT_RATE_LIMIT_AUTHENTICATION_PER_MINUTE` changes this limit, but also the limit of Orbit's other sign-in routes.
- A stranger who calls `orbit-oidc.wellrox.com` counts against their own address, so they should not use up Cloudflare's 30. Cloudflare's docs do not state which address its own Access calls carry, so only a real sign-in shows this.
- A request without one valid `CF-Connecting-IP` counts as the gateway's own address, and all such requests share one limit. This should not happen through the tunnel.

**1. Check the outbound port.** `cloudflared` needs port 7844 outbound, on TCP (HTTP/2) or UDP (QUIC). This starts no tunnel:

```sh
dig +short A region1.v2.argotunnel.com
nc -vz -w 3 198.41.192.167 7844    # TCP
nc -uvz -w 3 198.41.192.167 7844   # UDP (a success here is weak: UDP has no handshake)
```

**2. Deploy the gateway:**

```sh
sudo mkdir -p /etc/orbit-oidc-gateway
sudo cp deploy/oidc-gateway/nginx.conf /etc/orbit-oidc-gateway/nginx.conf
# If Orbit is not on 127.0.0.1:8080, change the address in `proxy_pass`.
sudo cp deploy/oidc-gateway/orbit-oidc-gateway.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now orbit-oidc-gateway
```

Orbit's `ORBIT_TRUSTED_PROXIES` must contain `127.0.0.1/32`, or the limit counts all callers as one address, and Orbit refuses the request as not HTTPS.

**3. Create the tunnel in Cloudflare:**

1. Go to **Networking** > **Tunnels** > **Create a tunnel**. Name it `orbit-oidc`, and select **Create Tunnel**.
2. Copy only the token from the install command that the page shows. Do not run that command: it puts the token on a command line.
3. On the **Routes** tab, select **Add route** > **Published application**. Subdomain `orbit-oidc`, domain `wellrox.com`, **Service URL** `http://127.0.0.1:8400`. Add no other route.
4. Turn on **Always Use HTTPS** for the domain, or check that it is on already.

**4. Install `cloudflared` with its unit:**

```sh
sudo mkdir -p --mode=0755 /usr/share/keyrings
curl -fsSL https://pkg.cloudflare.com/cloudflare-main.gpg | sudo tee /usr/share/keyrings/cloudflare-main.gpg >/dev/null
echo 'deb [signed-by=/usr/share/keyrings/cloudflare-main.gpg] https://pkg.cloudflare.com/cloudflared any main' | sudo tee /etc/apt/sources.list.d/cloudflared.list
sudo apt-get update && sudo apt-get install cloudflared

# The token never goes on a command line or into a shell history.
sudo install -m 600 -o root -g root /dev/null /etc/orbit-oidc-gateway/tunnel.env
read -rs TOKEN && printf 'TUNNEL_TOKEN=%s\n' "$TOKEN" | sudo tee /etc/orbit-oidc-gateway/tunnel.env >/dev/null; unset TOKEN

sudo cp deploy/oidc-gateway/cloudflared.service /etc/systemd/system/orbit-oidc-tunnel.service
sudo systemctl daemon-reload
sudo systemctl enable --now orbit-oidc-tunnel
```

Keep the log level at its default. At `debug`, `cloudflared` logs request URLs and headers.

**5. Check from outside**, on a machine that is not this one:

```sh
PUBLIC=https://orbit-oidc.wellrox.com
curl -s -o /dev/null -w '%{http_code}\n' "$PUBLIC/oauth/jwks"                            # 200
curl -s -o /dev/null -w '%{http_code}\n' -X POST -d x=1 "$PUBLIC/oauth/token"            # 401 or 400, from Orbit
curl -s -o /dev/null -w '%{http_code}\n' "$PUBLIC/"                                      # 404
curl -s -o /dev/null -w '%{http_code}\n' "$PUBLIC/login"                                 # 404
curl -s -o /dev/null -w '%{http_code}\n' "$PUBLIC/oauth/authorize"                       # 404
curl -s -o /dev/null -w '%{http_code}\n' "$PUBLIC/api/v1/auth/me"                        # 404
curl -s -o /dev/null -w '%{http_code}\n' --path-as-is "$PUBLIC/oauth/../api/v1/auth/me"  # 404
# A forged X-Forwarded-For must not avoid the limit: the first 429 comes at about the 31st request.
for i in $(seq 35); do curl -s -o /dev/null -w '%{http_code} ' -X POST -H "X-Forwarded-For: 198.51.100.$i" -d x=1 "$PUBLIC/oauth/token"; done
```

Run the limit check at the start of a minute, and last: a 429 stops further token calls from that address for the rest of the minute. Measured on `orbit-oidc.wellrox.com`: 400 for requests 1 to 30, 429 for requests 31 to 35.

Do not put `CF-Connecting-IP` in this check. Cloudflare refuses a request with a forged `CF-Connecting-IP` at its edge, with 403 and `error code: 1000`, so the request never reaches the gateway. Cloudflare can also normalize a path before it reaches the gateway. Both are fine as long as Orbit's other routes do not answer.

**Test the gateway's rules** on any machine with `deploy/oidc-gateway/test.sh`.

**Turn it off:**

```sh
sudo systemctl disable --now orbit-oidc-tunnel orbit-oidc-gateway
sudo rm /etc/systemd/system/orbit-oidc-tunnel.service /etc/systemd/system/orbit-oidc-gateway.service
sudo rm -r /etc/orbit-oidc-gateway
sudo systemctl daemon-reload
```

Then delete the tunnel in **Networking** > **Tunnels**, and the DNS record of `orbit-oidc.wellrox.com`. Sign-in through Access stops at once. Nothing else in Orbit changes.

**Limits.** The token alone can run this tunnel, so keep it secret, and replace it in Cloudflare if it leaks. Anyone who can edit the tunnel in Cloudflare can add a route to another local address, such as Orbit itself, which skips the gateway. So keep that dashboard access small.

**On exe.dev.** A tunnel changes no exe.dev setting, and needs no public port. Its [docs](https://exe.dev/docs/internet-access) have no rule that names tunnels, and the default egress policy `all` does not restrict outbound traffic. The policy `integrations` blocks the public internet, so it would stop the tunnel. The [terms](https://exe.dev/docs/terms-of-service) do not name tunnels either; section 2.2 forbids to "bypass the measures we may use to prevent or restrict access to the Service", so ask exe.dev if you are not sure.

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

**Check the login first.** If the frame stays white, open the app's address in a private window. If you see Cloudflare's "Sign in" page (for example "Email, Send login code"), Access is not using Instant Auth with Orbit as its only login method: that page always refuses frames, on every browser. Fix the Access application as in step 1.

**The bar with the actions** shows only if something did not load. A page cannot tell a working frame from a refused one, so the app says it loaded, with one line:

```html
<script>
  if (window.top !== window) window.top.postMessage({ type: 'orbit:app-ready' }, 'https://orbit.wellrox.com')
</script>
```

Put it in the page the app shows after sign-in, with your Orbit address. In a single-page app, run the same line once when the app has rendered. Orbit accepts it only from the app's frame and the app's origin, and it only hides the bar and the card. Outside a frame, the line does nothing.

- **With the signal:** the frame takes the full height. The actions are in a small **…** button in the corner: **Sign in to app**, **Reload**, **Open in a new tab**, and on a phone also **Menu**.
- **Without a signal within 8 seconds** (an app without the line, or a refused frame): the bar and its help text show. A late signal hides the bar again. **Sign in** and **Reload** start the same rule again.
- **The card "Sign in to <app>"** shows over the frame when Orbit knows that the app failed to load. It has a line with the reason and **Sign in to app**, **Open in a new tab**, **Reload** and **Show the app** (which hides the card until the next failure). With the bar open, the card replaces the help text; it also shows with the bar closed. A late signal hides it at once. It shows in three cases:
  - **Orbit asked for a sign-in.** The app's "Sign in with Orbit" reached Orbit in the frame without an Orbit session (or with `prompt=login`). Orbit tells its own page, then sends the frame to the app's redirect URI with `error=login_required`, as before. A top-level request still gets the redirect.
  - **The frame went to a login that Orbit does not allow.** The app sent the frame to an origin that is not the app, a redirect URI or Orbit (see [Limits](#limits)).
  - **An app that sent the signal before is silent.** If the app sent the signal in this browser before, Orbit expects it again: no signal 3 seconds after the frame's last page load (or 8 seconds if no page loads) is a failure. The browser keeps this per workspace and app (`orbit:app-ready-seen:<workspace>:<app>`). An app that never sent the signal keeps the bar and help text, with no card.
- **Closed by hand:** the **X** in the bar hides the bar and its help text, for example for an app that you cannot change. The frame takes the full height, and the actions are in the **…** button, as with the signal. This browser remembers the choice per workspace and app, so the bar stays closed after a reload, on the next open, and when **Sign in** or **Reload** start the rule again. The card does not change this choice. If the browser blocks the sign-in popup, the page still says so, in the card or above the frame. To get the bar back, delete the `orbit:app-bar-closed:<workspace>:<app>` entry from the browser's local storage.

On a phone the bar has one row: the menu, the app's name, **Sign in**, **Open**, a menu with **Reload**, and the **X**.

**Safari and every iOS browser** block third-party cookies by default. What that means depends on the app's address:

- **The app on the same site as Orbit** (`orbit.wellrox.com` and `products.wellrox.com`): the app's own `CF_Authorization` is not third-party in the frame, so a popup's sign-in should count in the frame. Only the Access cookies on `<team>.cloudflareaccess.com` are third-party. So the first sign-in inside the frame may fail on the callback, and **Sign in** in the popup is the way out; after it, the frame should work while the app's cookie lasts. Only a real iPhone can prove these points. Orbit's tests run only in Chromium.
- **The app on another site:** its own cookies are third-party too, so the frame can stay signed out even after a popup sign-in. Use **Open**, or set the app to **In a new tab**.

**A removed member** loses Orbit sign-in at once, but the Access session lasts for its own session duration. Keep that duration short.

## Limits

- **Tokens give no access to Orbit.** The access token works only at `/oauth/userinfo`, for 10 minutes. There are no refresh tokens. To let the agent call an app, add it in **Agentic → Workspace**; an app's page links there for admins.
- **The app's own session is the app's.** When a member is removed or a role is no longer allowed, Orbit deletes or refuses their tokens at once, and they cannot sign in again. A session the app already made lasts until the app ends it. Keep app sessions short, or check `/oauth/userinfo` while the token lasts.
- **Removing an app** deletes its client and tokens. The app itself keeps running.
- **`prompt=login` is not supported**: Orbit answers `login_required`. `max_age`, `request` and `request_uri` are not supported.
- **The frame passes only through the app, its redirect URIs and Orbit.** An app that signs in through another identity provider inside the frame is blocked, and Orbit shows the card "Sign in to <app>"; use **Sign in to app**, or add that provider's callback as a redirect URI.
- **The server needs `ORBIT_APP_KEY`** for sign-in: it seals the signing key. Without it, apps still open, without sign-in. If the key changes, sign-in stops until you restore it.
