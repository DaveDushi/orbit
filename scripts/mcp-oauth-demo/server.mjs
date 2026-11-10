// A mock remote MCP server with its own OAuth server, for local tests of "Connect" (docs/agent.md, "Connect").
// It works like TikTok Social's: a 401 names the protected resource metadata, the authorization server's metadata
// is only at the path-appended OIDC URL, dynamic client registration, PKCE S256, rotating refresh tokens. No deps.
//
//   node scripts/mcp-oauth-demo/server.mjs [port]      (default 4310)
//
// Two servers: /mcp (the grant lasts 1 year) and /mcp-short (10 days, to see the expiry warning).
// ACCESS_TTL (seconds, default 3600) sets the access token's lifetime. Orbit must list 127.0.0.1 in
// ORBIT_AGENT_INTEGRATION_TEST_HOSTS. Never use it with real data.
import { createHash, randomBytes } from 'node:crypto'
import { createServer } from 'node:http'

const port = Number(process.argv[2] ?? 4310)
const base = `http://127.0.0.1:${port}`
const issuer = `${base}/oauth`
const accessTtl = Number(process.env.ACCESS_TTL ?? 3600)
const grantDays = { '/mcp': 365, '/mcp-short': 10 }

const clients = new Map() // client_id → redirect_uris
const codes = new Map() // code → { challenge, redirect_uri, client_id, resource }
const access = new Map() // token → resource
const refresh = new Map() // token → { resource, client_id, expires }
const posts = []

const json = (response, status, body, headers = {}) => {
  response.writeHead(status, { 'content-type': 'application/json', ...headers })
  response.end(JSON.stringify(body))
}
const read = (request) => new Promise((resolve) => {
  let body = ''
  request.on('data', (chunk) => (body += chunk))
  request.on('end', () => resolve(body))
})
const token = () => randomBytes(24).toString('base64url')
const s256 = (verifier) => createHash('sha256').update(verifier).digest('base64url')
const escape = (text) => String(text).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c])

const tools = [
  { name: 'list_videos', title: 'List videos', description: 'List the latest videos of the connected TikTok account.', inputSchema: { type: 'object', properties: { limit: { type: 'number' } } }, annotations: { readOnlyHint: true } },
  { name: 'reply_comment', title: 'Reply to a comment', description: 'Post a reply to a comment on a video.', inputSchema: { type: 'object', properties: { comment_id: { type: 'string' }, text: { type: 'string' } }, required: ['comment_id', 'text'] } },
]

function tokens(resource, client_id) {
  const accessToken = token()
  const refreshToken = token()
  const grant = grantDays[new URL(resource).pathname] * 86400
  access.set(accessToken, resource)
  refresh.set(refreshToken, { resource, client_id, expires: Date.now() + grant * 1000 })
  return { access_token: accessToken, token_type: 'Bearer', expires_in: accessTtl, refresh_token: refreshToken, refresh_expires_in: grant, scope: 'video.list comment.list.manage', username: '@wellrox_brand' }
}

async function mcp(request, response, path) {
  const auth = request.headers.authorization ?? ''
  if (!auth.startsWith('Bearer ') || access.get(auth.slice(7)) !== `${base}${path}`) {
    response.writeHead(401, { 'www-authenticate': `Bearer resource_metadata="${base}/.well-known/oauth-protected-resource${path}"` })
    return response.end('unauthorized')
  }
  if (request.method === 'DELETE') return json(response, 200, {})
  const message = JSON.parse((await read(request)) || 'null')
  const reply = (result) => json(response, 200, { jsonrpc: '2.0', id: message.id, result }, { 'mcp-session-id': 'demo' })
  switch (message?.method) {
    case 'initialize': return reply({ protocolVersion: '2025-06-18', capabilities: { tools: {} }, serverInfo: { name: 'mock-social', version: '1' } })
    case 'notifications/initialized': response.writeHead(202); return response.end()
    case 'tools/list': return reply({ tools })
    case 'tools/call': {
      const { name, arguments: args = {} } = message.params ?? {}
      if (name === 'list_videos') return reply({ content: [{ type: 'text', text: JSON.stringify({ videos: [
        { id: 'v1', title: 'Behind the scenes at Wellrox', views: 12840, comments: 41 },
        { id: 'v2', title: 'New spring collection', views: 9310, comments: 27 },
        { id: 'v3', title: 'How we pack an order', views: 5402, comments: 12 },
      ].slice(0, args.limit ?? 3) }) }] })
      if (name === 'reply_comment') {
        posts.push(args)
        console.log('reply_comment', JSON.stringify(args))
        return reply({ content: [{ type: 'text', text: `Replied to ${args.comment_id}: ${args.text}` }] })
      }
      return json(response, 200, { jsonrpc: '2.0', id: message.id, error: { code: -32602, message: 'Unknown tool' } })
    }
    default: return json(response, 400, { error: 'bad request' })
  }
}

createServer(async (request, response) => {
  const url = new URL(request.url, base)
  const path = url.pathname
  if (path in grantDays) return mcp(request, response, path)
  if (path.startsWith('/.well-known/oauth-protected-resource/')) {
    const resource = path.slice('/.well-known/oauth-protected-resource'.length)
    if (!(resource in grantDays)) return json(response, 404, {})
    return json(response, 200, { resource: `${base}${resource}`, authorization_servers: [issuer], bearer_methods_supported: ['header'], scopes_supported: ['video.list', 'comment.list.manage'] })
  }
  if (path === '/oauth/.well-known/openid-configuration' || path === '/oauth/.well-known/oauth-authorization-server') {
    return json(response, 200, { issuer, authorization_endpoint: `${issuer}/authorize`, token_endpoint: `${issuer}/token`, registration_endpoint: `${issuer}/register`, code_challenge_methods_supported: ['S256'], grant_types_supported: ['authorization_code', 'refresh_token'], response_types_supported: ['code'], token_endpoint_auth_methods_supported: ['none'] })
  }
  if (path === '/oauth/register' && request.method === 'POST') {
    const body = JSON.parse(await read(request))
    const client_id = `client-${token().slice(0, 8)}`
    clients.set(client_id, body.redirect_uris ?? [])
    return json(response, 201, { client_id, client_name: body.client_name, redirect_uris: body.redirect_uris, token_endpoint_auth_method: 'none' })
  }
  if (path === '/oauth/authorize') {
    const q = Object.fromEntries(url.searchParams)
    const redirects = clients.get(q.client_id)
    if (!redirects?.includes(q.redirect_uri) || q.code_challenge_method !== 'S256' || !q.code_challenge || !q.resource) return json(response, 400, { error: 'invalid_request' })
    if (request.method === 'POST') {
      const form = new URLSearchParams(await read(request))
      const back = new URL(q.redirect_uri)
      back.searchParams.set('state', q.state)
      if (form.get('decision') === 'authorize') {
        const code = token()
        codes.set(code, { challenge: q.code_challenge, redirect_uri: q.redirect_uri, client_id: q.client_id, resource: q.resource })
        back.searchParams.set('code', code)
      } else back.searchParams.set('error', 'access_denied')
      response.writeHead(303, { location: back.toString() })
      return response.end()
    }
    response.writeHead(200, { 'content-type': 'text/html; charset=utf-8' })
    return response.end(`<!doctype html><title>Authorize · Mock Social</title>
<body style="font:15px system-ui;background:#121212;color:#eee;display:grid;place-items:center;height:100vh;margin:0">
<form method="post" style="background:#1e1e1e;padding:32px;border-radius:12px;width:380px">
<h2 style="margin-top:0">Mock Social</h2><p>Signed in as <b>@wellrox_brand</b></p>
<p><b>Orbit</b> wants to:</p><ul>${(q.scope ?? '').split(' ').filter(Boolean).map((s) => `<li>${escape(s)}</li>`).join('')}</ul>
<p style="color:#999;font-size:13px">For ${escape(q.resource)}. The grant lasts ${grantDays[new URL(q.resource).pathname] ?? '?'} days.</p>
<button name="decision" value="authorize" style="background:#fe2c55;color:#fff;border:0;padding:10px 18px;border-radius:6px;font-size:15px">Authorize</button>
<button name="decision" value="deny" style="margin-left:8px;padding:10px 18px;border-radius:6px">Cancel</button></form></body>`)
  }
  if (path === '/oauth/token' && request.method === 'POST') {
    const form = new URLSearchParams(await read(request))
    if (form.get('grant_type') === 'authorization_code') {
      const grant = codes.get(form.get('code'))
      codes.delete(form.get('code'))
      if (!grant || s256(form.get('code_verifier') ?? '') !== grant.challenge || form.get('redirect_uri') !== grant.redirect_uri || form.get('client_id') !== grant.client_id || form.get('resource') !== grant.resource) return json(response, 400, { error: 'invalid_grant' })
      return json(response, 200, tokens(grant.resource, grant.client_id))
    }
    if (form.get('grant_type') === 'refresh_token') {
      const old = refresh.get(form.get('refresh_token'))
      refresh.delete(form.get('refresh_token'))
      if (!old || old.expires < Date.now() || old.client_id !== form.get('client_id')) return json(response, 400, { error: 'invalid_grant' })
      const answer = tokens(old.resource, old.client_id)
      refresh.get(answer.refresh_token).expires = old.expires
      return json(response, 200, { ...answer, refresh_expires_in: Math.round((old.expires - Date.now()) / 1000) })
    }
    return json(response, 400, { error: 'unsupported_grant_type' })
  }
  if (path === '/posts') return json(response, 200, posts)
  json(response, 404, { error: 'not found' })
}).listen(port, '127.0.0.1', () => console.log(`mock MCP + OAuth on ${base} (/mcp: 1 year, /mcp-short: 10 days)`))
