#!/usr/bin/env node
// A small web app with "Sign in with Orbit", for docs/apps.md and the browser test. No packages;
// a real app should use an OpenID Connect library, which also checks the ID token's signature.
//
//   ORBIT_ISSUER=https://orbit.example.com CLIENT_ID=orbit-app-… CLIENT_SECRET=orbit-app-s-… \
//   APP_URL=https://reports.example.com FRAME_ANCESTORS=https://orbit.example.com PORT=4202 node server.mjs
// See below for ACCESS_URL, which the browser test uses.
import { createHash, randomBytes } from 'node:crypto'
import { createServer } from 'node:http'

const { ORBIT_ISSUER, CLIENT_ID, CLIENT_SECRET, APP_URL, FRAME_ANCESTORS, PORT = '4202', ACCESS_URL, ACCESS_MODE, ACCESS_COOP } = process.env
// With ACCESS_URL, a stand-in for Cloudflare Access guards the app on its own origin, and it is the
// OpenID Connect client. ACCESS_MODE=instant redirects straight to Orbit (Instant Auth); ACCESS_MODE=page
// shows a login page. All its answers refuse frames, as the real one does.
const redirectUri = ACCESS_URL ? `${ACCESS_URL}/callback` : `${APP_URL}/auth/callback`
const cookieName = `demo_session_${PORT}`
const tickets = new Map()
const sessions = new Map()
const pending = new Map()
const esc = (text) => String(text).replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`)
const b64url = (buffer) => buffer.toString('base64url')

const page = (res, body, headers = {}) => {
  res.writeHead(200, {
    'content-type': 'text/html; charset=utf-8',
    // Which pages may frame this app: only Orbit.
    'content-security-policy': `frame-ancestors ${FRAME_ANCESTORS}`,
    ...headers,
  })
  res.end(`<!doctype html><meta charset="utf-8"><title>Demo app</title><body style="font:15px system-ui;margin:2rem;color:#222;background:#fff">${body}`)
}

const signInUrl = async () => {
  const discovery = await (await fetch(`${ORBIT_ISSUER}/.well-known/openid-configuration`)).json()
  const state = b64url(randomBytes(24))
  const nonce = b64url(randomBytes(24))
  const verifier = b64url(randomBytes(48))
  pending.set(state, { nonce, verifier })
  const url = new URL(discovery.authorization_endpoint)
  url.search = new URLSearchParams({
    client_id: CLIENT_ID, response_type: 'code', redirect_uri: redirectUri, scope: 'openid profile email',
    state, nonce, code_challenge: b64url(createHash('sha256').update(verifier).digest()), code_challenge_method: 'S256',
  })
  return url.href
}

const startSession = (res, info) => {
  const id = b64url(randomBytes(24))
  sessions.set(id, info)
  res.writeHead(302, { location: '/?signed_in=1', 'set-cookie': `${cookieName}=${id}; HttpOnly; SameSite=Lax; Path=/` })
  res.end()
}

// Handles the answer of Orbit: `{ info }` is the signed-in person, `{ html }` a page to show.
const finishSignIn = async (url) => {
  const error = url.searchParams.get('error')
  // Orbit answers login_required when it cannot show its login: inside a frame, without a session.
  if (error) {
    return { html: `<h1>Demo app</h1><p id="signin-error">Sign-in said: <b>${esc(error)}</b></p>
      <p><a href="/auth/start" target="_blank" rel="noopener">Sign in with Orbit in a new tab</a></p>` }
  }
  const flow = pending.get(url.searchParams.get('state'))
  if (!flow) return { html: '<p>This sign-in is not known. Start again.</p>' }
  pending.delete(url.searchParams.get('state'))
  const discovery = await (await fetch(`${ORBIT_ISSUER}/.well-known/openid-configuration`)).json()
  const answer = await fetch(discovery.token_endpoint, {
    method: 'POST',
    headers: { 'content-type': 'application/x-www-form-urlencoded', authorization: `Basic ${Buffer.from(`${CLIENT_ID}:${CLIENT_SECRET}`).toString('base64')}` },
    body: new URLSearchParams({ grant_type: 'authorization_code', code: url.searchParams.get('code'), redirect_uri: redirectUri, code_verifier: flow.verifier }),
  })
  if (!answer.ok) return { html: `<p>The token request failed: ${esc(await answer.text())}</p>` }
  const tokens = await answer.json()
  const claims = JSON.parse(Buffer.from(tokens.id_token.split('.')[1], 'base64url'))
  if (claims.nonce !== flow.nonce || claims.aud !== CLIENT_ID || claims.iss !== ORBIT_ISSUER) return { html: '<p>The ID token does not match this sign-in.</p>' }
  const info = await (await fetch(discovery.userinfo_endpoint, { headers: { authorization: `Bearer ${tokens.access_token}` } })).json()
  return { info }
}

if (ACCESS_URL) {
  const refuseFrames = { 'x-frame-options': 'DENY', 'content-security-policy': "frame-ancestors 'none'", ...(ACCESS_COOP ? { 'cross-origin-opener-policy': 'same-origin' } : {}) }
  const access = new URL(ACCESS_URL)
  createServer(async (req, res) => {
    const url = new URL(req.url, ACCESS_URL)
    const redirect = (location) => { res.writeHead(302, { ...refuseFrames, location }); res.end() }
    const html = (body) => { res.writeHead(200, { ...refuseFrames, 'content-type': 'text/html; charset=utf-8' }); res.end(`<!doctype html><meta charset="utf-8"><title>Access</title><body style="font:15px system-ui;margin:2rem">${body}`) }
    if (url.pathname === '/login/start' || (url.pathname === '/login' && ACCESS_MODE === 'instant')) return redirect(await signInUrl())
    if (url.pathname === '/login') return html('<h1>Access login</h1><p><a id="sign-in" href="/login/start">Sign in with Orbit</a></p>')
    if (url.pathname === '/callback') {
      const result = await finishSignIn(url)
      if (result.html) return html(result.html)
      const ticket = b64url(randomBytes(24))
      tickets.set(ticket, result.info)
      return redirect(`${APP_URL}/authorized?ticket=${ticket}`)
    }
    res.writeHead(404); res.end()
  }).listen(Number(access.port), access.hostname)
}

createServer(async (req, res) => {
  const url = new URL(req.url, APP_URL)
  const sid = new RegExp(`(?:^|; )${cookieName}=([^;]+)`).exec(req.headers.cookie ?? '')?.[1]
  const user = sessions.get(sid)
  if (url.pathname === '/auth/start') {
    res.writeHead(302, { location: await signInUrl() })
    return res.end()
  }
  if (url.pathname === '/authorized') {
    const info = tickets.get(url.searchParams.get('ticket'))
    tickets.delete(url.searchParams.get('ticket'))
    if (!info) return page(res, '<p>This ticket is not known.</p>')
    return startSession(res, info)
  }
  if (url.pathname === '/auth/callback') {
    const result = await finishSignIn(url)
    if (result.html) return page(res, result.html)
    return startSession(res, result.info)
  }
  if (user) {
    return page(res, `<h1>Demo app</h1><p id="who">Signed in as <b>${esc(user.name)}</b> (${esc(user.email)})</p>
      <p>Role in the workspace: <b id="role">${esc(user.workspace_role)}</b></p>`)
  }
  // Signed in a moment ago, and the cookie did not come back: the browser blocks it in a frame of
  // another site. Signing in again would loop forever.
  if (url.searchParams.has('signed_in')) {
    return page(res, `<h1>Demo app</h1><p id="cookie-blocked">Your browser blocks this app's cookie in a frame.</p>
      <p><a href="/auth/start" target="_blank" rel="noopener">Open the app in a new tab</a></p>`)
  }
  res.writeHead(302, { location: ACCESS_URL ? `${ACCESS_URL}/login` : '/auth/start' })
  res.end()
}).listen(Number(PORT), new URL(APP_URL).hostname, () => console.log(`demo app on ${APP_URL}`))
