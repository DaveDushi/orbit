#!/usr/bin/env node
// A small web app with "Sign in with Orbit", for docs/apps.md and the browser test. No packages;
// a real app should use an OpenID Connect library, which also checks the ID token's signature.
//
//   ORBIT_ISSUER=https://orbit.example.com CLIENT_ID=orbit-app-… CLIENT_SECRET=orbit-app-s-… \
//   APP_URL=https://reports.example.com FRAME_ANCESTORS=https://orbit.example.com PORT=4202 node server.mjs
import { createHash, randomBytes } from 'node:crypto'
import { createServer } from 'node:http'

const { ORBIT_ISSUER, CLIENT_ID, CLIENT_SECRET, APP_URL, FRAME_ANCESTORS, PORT = '4202' } = process.env
const redirectUri = `${APP_URL}/auth/callback`
const sessions = new Map()
const pending = new Map()
const esc = (text) => String(text).replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`)
const b64url = (buffer) => buffer.toString('base64url')

const page = (res, body, headers = {}) => {
  res.writeHead(200, {
    'content-type': 'text/html; charset=utf-8',
    // Which pages may frame this app. Without this header, or with another origin, the frame is refused.
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

createServer(async (req, res) => {
  const url = new URL(req.url, APP_URL)
  const sid = /(?:^|; )demo_session=([^;]+)/.exec(req.headers.cookie ?? '')?.[1]
  const user = sessions.get(sid)
  if (url.pathname === '/auth/start') {
    res.writeHead(302, { location: await signInUrl() })
    return res.end()
  }
  if (url.pathname === '/auth/callback') {
    const error = url.searchParams.get('error')
    // Orbit answers login_required when it cannot show its login: inside a frame.
    if (error) {
      return page(res, `<h1>Demo app</h1><p id="signin-error">Sign-in said: <b>${esc(error)}</b></p>
        <p><a href="/auth/start" target="_blank" rel="noopener">Sign in with Orbit in a new tab</a></p>`)
    }
    const flow = pending.get(url.searchParams.get('state'))
    if (!flow) return page(res, '<p>This sign-in is not known. Start again.</p>')
    pending.delete(url.searchParams.get('state'))
    const discovery = await (await fetch(`${ORBIT_ISSUER}/.well-known/openid-configuration`)).json()
    const answer = await fetch(discovery.token_endpoint, {
      method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded', authorization: `Basic ${Buffer.from(`${CLIENT_ID}:${CLIENT_SECRET}`).toString('base64')}` },
      body: new URLSearchParams({ grant_type: 'authorization_code', code: url.searchParams.get('code'), redirect_uri: redirectUri, code_verifier: flow.verifier }),
    })
    if (!answer.ok) return page(res, `<p>The token request failed: ${esc(await answer.text())}</p>`)
    const tokens = await answer.json()
    const claims = JSON.parse(Buffer.from(tokens.id_token.split('.')[1], 'base64url'))
    if (claims.nonce !== flow.nonce || claims.aud !== CLIENT_ID || claims.iss !== ORBIT_ISSUER) return page(res, '<p>The ID token does not match this sign-in.</p>')
    const info = await (await fetch(discovery.userinfo_endpoint, { headers: { authorization: `Bearer ${tokens.access_token}` } })).json()
    const id = b64url(randomBytes(24))
    sessions.set(id, info)
    res.writeHead(302, { location: '/', 'set-cookie': `demo_session=${id}; HttpOnly; SameSite=Lax; Path=/` })
    return res.end()
  }
  if (user) {
    return page(res, `<h1>Demo app</h1><p id="who">Signed in as <b>${esc(user.name)}</b> (${esc(user.email)})</p>
      <p>Role in the workspace: <b id="role">${esc(user.workspace_role)}</b></p>`)
  }
  res.writeHead(302, { location: '/auth/start' })
  res.end()
}).listen(Number(PORT), '127.0.0.1', () => console.log(`demo app on ${APP_URL}`))
