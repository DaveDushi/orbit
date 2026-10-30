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

Remove any `X-Frame-Options` header. A page with neither header can be framed by every site, so add the header even if the frame works without it. Orbit cannot detect reliably that an app refused the frame, so the page always has **Open in a new tab** in its header.

In the frame, the app can run scripts, use forms, open popups and download files. It cannot navigate the Orbit page. The camera, microphone, location and payment features are off.

## Use a subdomain of the same domain

Host the app on a subdomain of Orbit's domain: for Orbit at `orbit.example.com`, use `reports.example.com`. This matters for sign-in in the frame:

- **Same site** (`reports.example.com`): the browser sends Orbit's session cookie in the frame, and the app's own cookies work. Sign-in is silent.
- **Other site** (`reports.other.net`): the browser sends no Orbit cookie in the frame, and most browsers also block the app's own cookies there. Orbit cannot show its login in a frame either. Orbit answers the app with `error=login_required`; the app must offer a link that signs in at the top level, in a new tab. Or set the app to **In a new tab**, which always works.

A person who is not signed in to Orbit also gets `login_required` in the frame on a subdomain, but Orbit's own page already asks them to sign in first.

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

[`scripts/apps-demo/server.mjs`](../scripts/apps-demo/server.mjs) is a complete small app without packages, which also shows the `login_required` case.

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
| Auth URL | `https://orbit.example.com/oauth/authorize` |
| Token URL | `https://orbit.example.com/oauth/token` |
| Certificate URL | `https://orbit.example.com/oauth/jwks` |
| Proof Key for Code Exchange (PKCE) | On |
| Email claim | `email` (the default scopes `openid email profile` include it) |

**3. In the Access application** of `products.wellrox.com`, choose this login method and a policy. Add `workspace_role` under OIDC Claims if a policy needs the role.

**Which addresses must be public.** Cloudflare's servers call Orbit without a browser session, so nothing in front of Orbit may ask them to sign in:

| Path | Called by | Must be public |
| --- | --- | --- |
| `/oauth/token` | Cloudflare's servers (`POST`) | Yes |
| `/oauth/jwks` | Cloudflare's servers | Yes |
| `/.well-known/openid-configuration` | Other OpenID Connect clients, setup tools | Yes for a standard client |
| `/oauth/userinfo` | Other OpenID Connect clients | Yes for a standard client |
| `/oauth/authorize`, `/login` and the web app | The person's browser | Reachable by the person. A login proxy in front of them breaks sign-in in a frame, because a proxy's login page usually refuses frames. |

None of these paths gives data without a secret or a token: discovery and JWKS hold only public values.

**In a frame.** Access makes the sign-in pass through `edusi.cloudflareaccess.com`, which Orbit allows because it is the origin of the redirect URI. Access must also allow frames and send its cookie in them: see `allow_iframe` and the cookie settings of the Access application. If the app is on another site than Orbit, browsers may block these cookies. In that case, set the app to **In a new tab**. Orbit cannot change these settings, and Access may refuse to show its own pages in a frame; test this with your account.

**A removed member** loses Orbit sign-in at once, but the Access session lasts for its own session duration. Keep that duration short.

## Limits

- **Tokens give no access to Orbit.** The access token works only at `/oauth/userinfo`, for 10 minutes. There are no refresh tokens. To let the agent call an app, add it in Workspace settings → **Agent tools**; an app's page links there for admins.
- **The app's own session is the app's.** When a member is removed or a role is no longer allowed, Orbit deletes or refuses their tokens at once, and they cannot sign in again. A session the app already made lasts until the app ends it. Keep app sessions short, or check `/oauth/userinfo` while the token lasts.
- **Removing an app** deletes its client and tokens. The app itself keeps running.
- **`prompt=login` is not supported**: Orbit answers `login_required`. `max_age`, `request` and `request_uri` are not supported.
- **The frame passes only through the app, its redirect URIs and Orbit.** An app that signs in through another identity provider inside the frame is blocked; use **In a new tab**.
- **The server needs `ORBIT_APP_KEY`** for sign-in: it seals the signing key. Without it, apps still open, without sign-in. If the key changes, sign-in stops until you restore it.
