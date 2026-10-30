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

## Not in v1

App bridge script, health status, hosting, restarts, deploys, frames on mobile, refresh tokens, key rotation (JWKS already returns a list).
