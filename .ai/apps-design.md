# Apps: design and grilling

What: admins add custom web apps; each gets a sidebar entry, `/apps/<slug>` with a frame, and optional "Sign in with Orbit". User docs: [`docs/apps.md`](../docs/apps.md). Code: `apps/server/src/apps/`, `apps/web/src/features/apps/`.

## Decisions

| Decision | Why |
| --- | --- |
| Owned by the workspace; permission `apps.manage` (owners, admins); allowed roles per app | Same shape as agent integrations. |
| Wrapper route `GET …/apps/{slug}/frame` with its own CSP, not a shared CSP list | The global CSP is on public responses (it would leak internal host names) and a shared list needs invalidation across servers. |
| `add_security_headers` keeps a CSP a handler set | Only the wrapper sets one. Everything else still gets the global policy. |
| Global `frame-src` gained `'self'` | The SPA must frame the wrapper. Every other Orbit answer has `frame-ancestors 'none'`, so `'self'` makes nothing else frameable. |
| `/oauth/authorize` and `/oauth/token` hand `orbit-app-…` clients to `apps::oidc` | One issuer, one server, and the MCP code path is otherwise unchanged. |
| Own tables `app_oidc_codes`, `app_oidc_tokens` | Token separation by construction. See below. |
| RS256 key: `rsa` makes it, `ring` signs | `ring` cannot make RSA keys. `rsa` has an open timing advisory ([RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071)) for private-key operations an attacker can time; making a key once is not such an operation, and `ring` does all signing. |
| Key row with `ON CONFLICT DO NOTHING`, sealed with `ORBIT_APP_KEY` | Safe when several servers start the first sign-in together. |
| `http://` only in development mode | No extra setting that could stay on in production. |

## Grilling

**Clickjacking.** The wrapper has `frame-ancestors 'self'`, so only Orbit pages can frame it. Login and all other pages stay `frame-ancestors 'none'`. We did not weaken cookies or allow framing of login.

**An app reads Orbit?** `allow-same-origin` would be dangerous if the frame had Orbit's origin. So `check_app_url` refuses Orbit's origin (normalized by the URL parser, ports included), and the frame route checks again, in case the public origin changed. Without `allow-top-navigation`, the app cannot replace the Orbit page.

**Silent sign-in in a frame.** Because the cookie is sent, a page that frames an app sends the visitor through `/oauth/authorize` without a click. Only Orbit can frame the wrapper, and an app's redirect URIs are exact, so this gives a code only to an app the admin approved.

**Can the frame load an Orbit page?** With sign-in on, the wrapper's `frame-src` has `'self'`, because the sign-in redirect passes through `/oauth/authorize`. A redirect loads no document, and each final Orbit answer has `frame-ancestors 'none'`, except the wrapper itself, which holds only another frame of an allowed app. Test in the browser.

**CSP or HTML injection through an app URL.** The URL parser allows `;`, `,` and `'` in a host. `plain_host` allows only letters, digits, `.` and `-`, so nothing can start a new directive. All text in the wrapper is HTML-escaped. Tests: `the_wrapper_escapes_html…`, `app_urls_are_https…`.

**Token confusion with MCP.** `OAuthState::authenticate` reads only `oauth_tokens`; userinfo reads only `app_oidc_tokens`. No code path joins them. An app's token is refused at `/mcp` and at the API (session lookup fails), and an MCP or session token is refused at userinfo. `/oauth/register` makes `orbit-client-…` without a secret, and an OIDC client must have a row in `workspace_apps`, which only `apps.manage` can write. Test: `app_tokens_and_mcp_tokens_never_cross_over`.

**Open redirects.** Nothing returns to a redirect URI until the client is known and the URI matches exactly; the URI must also not be on Orbit's origin. The login's `return_to` accepts only `/oauth/authorize?` with an `orbit-app-` client (`appSignInDestination`), and that route checks the URI again.

**Leaked host names.** The list and the wrapper need a session and an allowed role; others get 404. The global CSP has no app addresses. Discovery and JWKS name no app. An `error` from an unknown client is plain JSON.

**Removed members.** `remove_member` deletes their tokens and codes in its transaction. Every token, authorize and userinfo call also reads membership, role, suspension and the workspace's trash state, so a role change takes effect at once even though the row remains. The app's own session lasts until it ends, which is a documented limit.

**Code theft.** Codes last 60 s, are stored hashed and work once; a second use deletes the first token. The secret is checked first, and PKCE is checked if the request used it. A wrong try does not use up the code.

**Frames and login.** Orbit cannot show login in a frame. A request with `Sec-Fetch-Dest: iframe` and no session gets `login_required` at the redirect URI instead of a blank refused frame. The header only chooses the friendlier answer: it grants nothing.

**Cookies in a frame (measured in Chromium, `scripts/apps-demo/e2e.mjs`).** The first design assumed Orbit's `SameSite=Lax` cookie is not sent in a cross-site frame. That is wrong here: Orbit is the top page, so its request in the frame is same-site with every ancestor and the cookie is sent. The cookie that fails is the app's own, which is third-party under Orbit. A cross-site app with a Lax session cookie therefore loops: sign-in succeeds, the cookie is dropped, it signs in again. So a subdomain remains the advice, for this reason. Orbit cannot detect the loop: a frame reports nothing to its parent. The page keeps "Open in a new tab".

**CORS on `/oauth/authorize`.** The MCP layer sends `Access-Control-Allow-Origin: *` on it, and app clients now read a cookie there. A cross-site `fetch` gets no Lax cookie, and a same-site one with credentials cannot read a `*` answer. The code in a redirect is not exposed.

**Sibling subdomains.** A same-site app can send requests with the Orbit cookie for simple top-level or frame navigation. `SameSite` gives no protection between subdomains, but Orbit's origin policy still refuses writes from another origin. Advise admins to treat a same-domain app as trusted.

**`Sec-Fetch-Dest` absent or forged** only changes login to an error or back; there is no access either way.

## Apps behind a login that refuses frames

Refused frames cannot be detected from the parent, so nothing is built on detection: **Sign in to app** is always in the header of an embedded app. `signInPopup.ts` opens the popup at `about:blank` and sets `opener = null` before it navigates, so Orbit has a handle for `closed` and the login page has no way back to Orbit. A login page with `Cross-Origin-Opener-Policy` makes `closed` true at once, so the first reload can be too early. The frame therefore also reloads on the first return to the Orbit page after that. Reload stays as the manual way out. The popup is not sandboxed, because it is a top-level page.

Measured in Chromium (`scripts/apps-demo/e2e.mjs`, with a stand-in for Access on its own origin whose answers all send `X-Frame-Options: DENY` and `frame-ancestors 'none'`): a redirect that refuses frames does not stop the frame, only a final page does. So Instant Auth works in the frame, and Orbit's own `frame-ancestors 'none'` on the `/oauth/authorize` redirect does no harm. A real Access can behave differently, for example with a page instead of a redirect in the chain.

**Phones and WebKit.** A white frame came from Access's own login page: on 2026-11-01 `products.wellrox.com` ended at a 200 "Sign in" page with Email login, `X-Frame-Options: DENY` and `frame-ancestors 'none'`, with no sign-in code or token on Orbit. That is an Access setting, not a phone fault. By the user's decision every browser shows the frame first; we tried and removed an "open first" path for WebKit. On a phone the page's own header replaces the global top bar, as Tasks and Docs do, and it keeps **Sign in** and **Open** visible, because a blank frame cannot be detected and a help line would cost height.

WebKit's limit differs by address. In this case (`orbit.wellrox.com`, `products.wellrox.com`, one site), `CF_Authorization` on the app's host is first-party in the frame, and only `CF_Session` and the other cookies on `edusi.cloudflareaccess.com` are third-party. Proved: the cookie names and attributes of the first hops, and the layout and popup flow in Chromium. Not proved, needs a real iPhone: whether the team-domain cookie blocks the first sign-in in the frame, and whether a popup's sign-in then counts in the frame. For an app on another site, its own cookies are third-party as well.

**The bar and the ready signal.** The `load` event fires for refused pages too, so the only reliable signal is one from the app: `window.top.postMessage({ type: 'orbit:app-ready' }, <Orbit origin>)`. It goes to `window.top`, not `parent`: the wrapper document between them has no scripts by design, and adding a relay would weaken that. `appReady.ts` accepts it only if `source` is the wrapper's one frame and `origin` is the app's. It can only hide the bar, so a forged or wrong signal costs nothing more than a hidden bar, and the actions stay in the corner **…** button. The timeout is 8 s: a sign-in through a proxy and Orbit on a phone can take seconds, and a slow app is only shown the bar for a moment, since a late signal hides it. No signal means today's behaviour, so a blank page always gets its way out. The cost: an app without the line shows nothing above the frame for those 8 s, and then keeps the bar and help text.

**The sign-in card** (user's decisions, 2026-11-02; investigation and mocks in `/root/app-signin-detect-test/`). Three failures are certain enough for a card over the frame, measured in Chromium and WebKit:
1. *Orbit's `login_required` in a frame.* Instead of the 302, `oidc.rs` answers a framed (`Sec-Fetch-Dest: iframe`) `login_required` with a page: one constant script (CSP allows only its hash, `frame-ancestors 'self'`, `no-store`, `Referrer-Policy: no-referrer`; the security layer keeps a response's own referrer policy) that sends `top.postMessage({ type: 'orbit:app-login-required' }, location.origin)` and then `location.replace`s to the same validated callback (in an escaped `<meta>`). The message has no state, code or address. The Orbit page accepts it only with `origin` = Orbit and `source` = the app frame's window, so neither the app nor another window can show the card (it only shows a card anyway). Top-level requests keep the 302.
2. *A navigation that `frame-src` refuses.* `securitypolicyviolation` fires on the wrapper document. The wrapper stays script-free: its frame has `data-src`; the Orbit page (same origin) attaches the `securitypolicyviolation` and `load` listeners when the wrapper loads, then sets `src` (`startAppFrame`). Without that order the first refusal can come before the listener.
3. *Known ready.* An app that sent the signal in this browser (`orbit:app-ready-seen:<workspace>:<app>` in localStorage) and is silent 3 s after the frame's last `load` (8 s if none) has failed. Loads after the signal or a failure do not restart the time, so apps with the line on one page only do not get a false card. Apps that never sent it keep the bar, with no card: `load` fires for refused pages too, so silence alone proves nothing for them.
The card logic is pure (`watchAppReady` in `appReady.ts`, `nextAppFrame`/`appBarView` in `appBar.ts`) and unit-tested; `scripts/apps-demo/e2e-card.mjs` is the browser proof.

## Public gateway for Cloudflare (`deploy/oidc-gateway/`)

nginx, because the machine already has it, it needs no new code to maintain, and `$request_uri` gives the raw request line to compare. It is its own process and unit, so turning it off touches nothing else. Header forwarding is an allow-list: a list of headers to strip would miss a new one. Plain strings in an nginx `map` ignore case, so the rules are regular expressions (`test.sh` caught this). Orbit only trusts the gateway as it trusts any configured proxy (`X-Forwarded-For`, `X-Forwarded-Proto`, `X-Request-Id`, read in `crates/platform/src/http/security.rs`). It reads no `X-ExeDev-*` header anywhere.

**Tunnel, decided by the user.** Cloudflare's servers reach the gateway through a remotely-managed Cloudflare Tunnel (`cloudflared.service`, token in a mode 600 environment file) to `127.0.0.1:8400`, with the hostname `orbit-oidc.wellrox.com`. Orbit stays behind the exe.dev login, and no exe.dev setting changes. We rejected the gateway on exe.dev's root URL (it changes `ORBIT_PUBLIC_ORIGIN`) and a second VM.

**Client address.** Only `CF-Connecting-IP` counts. Cloudflare's docs say it holds the client address on edge-to-origin traffic, and `X-Forwarded-For` keeps what the caller sent, so that header is ignored. Measured through the real tunnel: Cloudflare refuses a forged `CF-Connecting-IP` at its edge (403, `error code: 1000`), and a new forged `X-Forwarded-For` on each request does not avoid the limit (first 429 at request 31). Cloudflare's docs do not say what the header holds on Access's own calls to an identity provider, so only a real sign-in shows that. Without a valid value, the request counts as the gateway: it still works, and it cannot choose its address. We chose this over a refusal because a wrong guess about Access's calls would otherwise break every sign-in, with only a 404 to explain it.

**Issuer.** `iss` stays `ORBIT_PUBLIC_ORIGIN`. OpenID Connect lets the token endpoint be on another host. Cloudflare's form has no issuer field and its docs do not say if it checks `iss`.

**Remaining risk.** The token runs the tunnel, and a Cloudflare dashboard user can add a route around the gateway. See `docs/apps.md`.

## Not in v1

App bridge script, health status, hosting, restarts, deploys, frames on mobile, refresh tokens, key rotation (JWKS already returns a list).
