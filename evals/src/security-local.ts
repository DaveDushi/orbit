// Active security checks against the isolated test instance (production mode, seeded by seed.ts). Never point this
// at a real server. Evidence lists requests and statuses only: no cookies, tokens or passwords.
// Writes $EVALS_WORK/parts/security-local.json.
import { spawnSync } from 'node:child_process'
import { createHash, randomBytes } from 'node:crypto'
import { readdirSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { call, instance, login, quick, readJson, sessionFrom, writePart, type Reply, type Seed } from './common'
import { check, cookieChecks, documentHeaderChecks, type Check } from './headers'
import { routes } from './routes'

const server = instance()
if (server.mode !== 'production') throw new Error('security-local needs the production-mode test instance')
const seed = readJson<Seed>('seed.json')
const { a, b } = seed
const checks: Check[] = []
const add = (...items: Check[]) => {
  checks.push(...items)
  for (const item of items) if (item.result !== 'pass') console.log(`  ${item.result.toUpperCase()} ${item.id}: ${item.title}`)
}
// Evidence shows the seeded ids by name ({A} = workspace A, {pageB} = B's doc, …).
const names = (text: string) =>
  [a, b].reduce(
    (out, account, index) => {
      const x = index ? 'B' : 'A'
      return out
        .replaceAll(account.workspaceId, `{${x}}`)
        .replaceAll(account.pageId, `{page${x}}`)
        .replaceAll(account.fileId, `{file${x}}`)
        .replaceAll(account.taskId, `{task${x}}`)
        .replaceAll(account.projectId, `{project${x}}`)
        .replaceAll(account.userId, `{user${x}}`)
    },
    text,
  )
const line = (method: string, path: string, reply: Pick<Reply, 'status'>) => names(`${method} ${path} -> ${reply.status}`)

const sessionA = await login(a.email, seed.password)
const sessionB = await login(b.email, seed.password)
const ws = (id: string) => `/api/v1/workspaces/${id}`

// --- Headers and cookies of the test instance (production mode) -------------------------------------------------
{
  const index = await call('GET', '/')
  add(...documentHeaderChecks('local.index', '/', index.headers, true))
  const api = await call('GET', '/api/v1/auth/options')
  add(check('local.api.nosniff', 'headers', 'API responses send nosniff', api.headers.get('x-content-type-options') === 'nosniff', 'low', [line('GET', '/api/v1/auth/options', api)]))
  const reply = await call('POST', '/api/v1/auth/login', { body: { email: a.email, password: seed.password }, ip: '198.51.100.10' })
  add(...cookieChecks('local.login', 'POST /api/v1/auth/login', reply.headers.getSetCookie(), true))
  const names = reply.headers.getSetCookie().map((cookie) => cookie.split('=')[0])
  add(check('local.cookie-prefix', 'cookies', 'Session cookie uses the __Host- prefix', names.some((name) => name.startsWith('__Host-')), 'low', [`cookies: ${names.join(', ')}`]))
}

// --- Unauthenticated access to every route --------------------------------------------------------------------
{
  // Routes that must answer without a session (they check a token, a signature or a password themselves).
  const publicRoutes = [
    /^GET \/$/,
    /^GET \/\.well-known\//,
    /^GET \/health\//,
    /^(GET|POST) \/oauth\/(authorize|jwks|register|token|revoke)$/,
    /^GET \/api\/v1\/setup\/status$/,
    /^POST \/api\/v1\/setup\/complete$/,
    /^GET \/api\/v1\/auth\/options$/,
    /^POST \/api\/v1\/auth\/(login|login\/two-factor|passkey\/login\/(start|finish)|recovery\/(request|complete)|register\/(request|complete)|email-change\/confirm)$/,
    /^POST \/api\/v1\/workspaces\/invitations\/(preview|accept)$/,
    /^POST \/api\/v1\/(integrations\/(discord\/events|github\/webhook)|billing\/stripe\/webhook)$/,
    /^GET \/api\/v1\/integrations\/github\/manifest\/callback$/,
    /^GET \/api\/v1\/push\/key$/,
  ]
  const ids: Record<string, string> = {
    workspace_id: a.workspaceId,
    page_id: a.pageId,
    file_id: a.fileId,
    task_id: a.taskId,
    project_id: a.projectId,
    status_id: a.statusId,
    user_id: a.userId,
  }
  const failures: string[] = []
  const warnings: string[] = []
  const errors: string[] = []
  let tested = 0
  let rateLimited = 0
  // /metrics is served on its own loopback listener (ORBIT_METRICS_LISTEN), not on the app port.
  for (const route of routes().filter((route) => !route.file.endsWith('metrics.rs'))) {
    const path = route.path.replace(/\{([a-z_]+)\}/g, (_, name: string) => ids[name] ?? '0000000000000000000000000')
    const isPublic = publicRoutes.some((pattern) => pattern.test(`${route.method} ${route.path}`))
    let status: number
    try {
      // A streaming endpoint must not hang the sweep: read the status, then drop the body.
      const response = await fetch(server.base + path, {
        method: route.method,
        headers: {
          'x-forwarded-proto': 'https',
          'x-forwarded-for': '198.51.100.20',
          'x-orbit-contract': 'orbit-api-v1',
          ...(['GET', 'HEAD'].includes(route.method) ? {} : { origin: server.origin, 'content-type': 'application/json' }),
        },
        body: ['GET', 'HEAD'].includes(route.method) ? undefined : '{}',
        redirect: 'manual',
        signal: AbortSignal.timeout(10_000),
      })
      status = response.status
      await response.body?.cancel()
    } catch (error) {
      errors.push(`${route.method} ${route.path} -> ${(error as Error).name}`)
      continue
    }
    tested++
    const entry = `${route.method} ${route.path} -> ${status}`
    if (status === 429) rateLimited++
    else if (status >= 500) errors.push(entry)
    else if (isPublic) continue
    else if (status < 400) failures.push(entry)
    else if (status !== 401 && status !== 403) warnings.push(entry)
  }
  add(check('authz.unauthenticated', 'authz', `No protected route answers without a session (${tested} routes)`, failures.length === 0, 'high', failures.length ? failures : [`${tested} routes tested, all protected routes refused`], 'Require a session or token before the handler runs.'))
  add(check('authz.unauthenticated-other-4xx', 'authz', 'Protected routes refuse with 401/403 (not 400/404/405 first)', warnings.length === 0 ? true : 'warn', 'info', warnings.length ? warnings.slice(0, 25).concat(warnings.length > 25 ? [`… ${warnings.length - 25} more`] : []) : ['all 401/403'], 'Run the auth extractor before body and path validation.'))
  add(check('authz.unauthenticated-5xx', 'errors', 'No route fails with 5xx or hangs without a session', errors.length === 0, 'low', errors.length ? errors : ['none']))
  if (rateLimited) add(check('authz.unauthenticated-429', 'authz', 'Sweep hit the rate limit (results less exact)', 'warn', 'info', [`${rateLimited} answers were 429`]))
}

// --- IDOR: user B (owner of workspace B, not a member of A) against A's objects -----------------------------------
{
  const pageA = `${ws(a.workspaceId)}/pages/${a.pageId}`
  const fileA = `${pageA}/files/${a.fileId}`
  const before = await call('GET', pageA, { session: sessionA })
  const taskBefore = await call('GET', `${ws(a.workspaceId)}/tasks/${a.taskId}`, { session: sessionA })
  const probes: [string, string, unknown?][] = [
    ['GET', ws(a.workspaceId)],
    ['GET', `${ws(a.workspaceId)}/pages`],
    ['GET', pageA],
    ['GET', `${pageA}/versions`],
    ['GET', `${pageA}/files`],
    ['GET', fileA],
    ['GET', `${fileA}/content`],
    ['GET', `${ws(a.workspaceId)}/pages/search?q=searchable`],
    ['GET', `${ws(a.workspaceId)}/tasks`],
    ['GET', `${ws(a.workspaceId)}/tasks/${a.taskId}`],
    ['GET', `${ws(a.workspaceId)}/projects`],
    ['GET', `${ws(a.workspaceId)}/members`],
    ['GET', `${ws(a.workspaceId)}/apps`],
    ['GET', `${ws(a.workspaceId)}/workspace-apps`],
    ['GET', `${ws(a.workspaceId)}/api-tokens`],
    ['GET', `${ws(a.workspaceId)}/audit`],
    ['GET', `${ws(a.workspaceId)}/exports`],
    ['GET', `${ws(a.workspaceId)}/chat/conversations`],
    ['GET', `${ws(a.workspaceId)}/storage`],
    // A's objects under B's own workspace id: the object must belong to the workspace in the path.
    ['GET', `${ws(b.workspaceId)}/pages/${a.pageId}`],
    ['GET', `${ws(b.workspaceId)}/pages/${a.pageId}/files/${a.fileId}`],
    ['GET', `${ws(b.workspaceId)}/pages/${b.pageId}/files/${a.fileId}`],
    ['GET', `${ws(b.workspaceId)}/pages/${b.pageId}/files/${a.fileId}/content`],
    ['GET', `${ws(b.workspaceId)}/tasks/${a.taskId}`],
    ['GET', `${ws(b.workspaceId)}/tasks/${a.taskId}/comments`],
    // Writes.
    ['PATCH', pageA, { expected_version: before.json?.version ?? 1, title: 'changed by B' }],
    ['PATCH', `${ws(b.workspaceId)}/pages/${a.pageId}`, { expected_version: before.json?.version ?? 1, title: 'changed by B' }],
    ['DELETE', pageA],
    ['DELETE', `${ws(b.workspaceId)}/pages/${a.pageId}`],
    ['POST', `${ws(a.workspaceId)}/pages`, { title: 'created by B' }],
    ['PATCH', `${ws(a.workspaceId)}/tasks/${a.taskId}`, { expected_version: taskBefore.json?.version ?? 1, title: 'changed by B' }],
    ['PATCH', `${ws(b.workspaceId)}/tasks/${a.taskId}`, { expected_version: taskBefore.json?.version ?? 1, title: 'changed by B' }],
    ['POST', `${ws(b.workspaceId)}/pages/${a.pageId}/move`, { parent_id: b.pageId }],
    ['POST', `${ws(b.workspaceId)}/pages/${a.pageId}/duplicate`, {}],
    ['PUT', `${ws(b.workspaceId)}/pages/${b.pageId}/files/${a.fileId}/content`, { expected_version: 1, text: 'changed by B' }],
    ['POST', `${ws(a.workspaceId)}/invitations`, { email: 'eval-b2@example.test', role: 'owner', delivery: 'manual' }],
    ['PATCH', ws(a.workspaceId), { name: 'renamed by B', expected_version: 1 }],
    ['POST', `${ws(a.workspaceId)}/api-tokens`, { name: 'B token', scopes: ['read'] }],
    ['DELETE', `${ws(a.workspaceId)}/api-tokens/0000000000000000000000000`],
    ['POST', `${ws(a.workspaceId)}/exports`, {}],
    ['POST', `${ws(a.workspaceId)}/transfer-ownership`, { user_id: b.userId }],
  ]
  const leaks: string[] = []
  const evidence: string[] = []
  for (const [method, path, body] of probes) {
    const reply = await call(method, path, { session: sessionB, body, ip: '198.51.100.30' })
    evidence.push(line(method, path, reply))
    if (reply.status < 400) leaks.push(line(method, path, reply))
  }
  const after = await call('GET', pageA, { session: sessionA })
  const taskAfter = await call('GET', `${ws(a.workspaceId)}/tasks/${a.taskId}`, { session: sessionA })
  const unchanged = after.status === 200 && after.json?.title === before.json?.title && taskAfter.json?.title === taskBefore.json?.title
  add(check('idor.cross-workspace', 'idor', `User B cannot read or change workspace A (${probes.length} requests)`, leaks.length === 0, 'critical', leaks.length ? leaks : evidence.slice(0, 6).concat([`… ${evidence.length - 6} more, all 4xx`]), 'Check workspace membership and the object-workspace link in every handler.'))
  add(check('idor.state-unchanged', 'idor', "A's doc and task are unchanged after B's writes", unchanged, 'critical', [`page A ${after.status}, title same: ${after.json?.title === before.json?.title}`, `task A title same: ${taskAfter.json?.title === taskBefore.json?.title}`]))
}

// --- API tokens, MCP and OAuth scope ----------------------------------------------------------------------------
{
  const evidence: string[] = []
  const bad: string[] = []
  const control = await call('GET', `${ws(a.workspaceId)}/pages`, { bearer: a.readToken, ip: '198.51.100.40' })
  evidence.push(`read token: ${line('GET', `${ws(a.workspaceId)}/pages`, control)}`)
  if (control.status < 300) bad.push(`read token reads the REST API: ${line('GET', `${ws(a.workspaceId)}/pages`, control)}`)
  const probe = async (label: string, token: string, method: string, path: string, body?: unknown) => {
    const reply = await call(method, path, { bearer: token, body, origin: null, ip: '198.51.100.40' })
    evidence.push(`${label}: ${line(method, path, reply)}`)
    if (reply.status < 300) bad.push(`${label}: ${line(method, path, reply)}`)
  }
  await probe('read token A', a.readToken, 'POST', `${ws(a.workspaceId)}/pages`, { title: 'written with a read token' })
  await probe('read token A', a.readToken, 'PATCH', `${ws(a.workspaceId)}/tasks/${a.taskId}`, { expected_version: 1, title: 'x' })
  for (const token of [a.readToken, a.writeToken]) {
    const label = token === a.readToken ? 'read token A' : 'write token A'
    await probe(label, token, 'GET', `${ws(b.workspaceId)}/pages`)
    await probe(label, token, 'GET', `${ws(b.workspaceId)}/pages/${b.pageId}`)
    await probe(label, token, 'POST', `${ws(b.workspaceId)}/pages`, { title: 'cross-workspace write' })
    await probe(label, token, 'POST', '/api/v1/workspaces', { name: 'made with a token' })
    await probe(label, token, 'PATCH', '/api/v1/auth/me', { display_name: 'renamed with a token' })
    await probe(label, token, 'GET', '/api/v1/admin/users')
    await probe(label, token, 'POST', `${ws(a.workspaceId)}/api-tokens`, { name: 'token minted by a token', scopes: ['read', 'write'] })
  }
  add(check('tokens.rest-scope', 'tokens', 'Workspace API tokens (MCP-only) cannot use the REST API or act for the account', bad.length === 0, 'high', bad.length ? bad : evidence))

  // MCP with workspace API tokens. rmcp checks Host against the public origin, as a reverse proxy would pass it.
  const mcp = async (token: string, method: string, params: unknown) => {
    const reply = await call('POST', '/mcp', {
      bearer: token,
      origin: null,
      ip: '198.51.100.41',
      headers: { accept: 'application/json, text/event-stream', host: new URL(server.origin).host },
      body: { jsonrpc: '2.0', id: 1, method, params },
    })
    let data: any = reply.json
    if (!data) {
      try {
        data = JSON.parse(reply.text.match(/^data: (.*)$/m)?.[1] ?? 'null')
      } catch {}
    }
    return { status: reply.status, data, text: JSON.stringify(data ?? {}) }
  }
  const returned = (reply: Awaited<ReturnType<typeof mcp>>, marker: string) => reply.status === 200 && reply.text.includes(marker) && !reply.data?.result?.isError && !reply.data?.error
  const mcpEvidence: string[] = []
  const mcpBad: string[] = []
  const init = await mcp(a.writeToken, 'initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'evals', version: '1' } })
  mcpEvidence.push(`initialize -> ${init.status}`)
  let controlOk = false
  if (init.status === 200) {
    const own = await mcp(a.writeToken, 'tools/call', { name: 'get_doc', arguments: { page_id: a.pageId } })
    controlOk = returned(own, 'Eval A paragraph')
    mcpEvidence.push(`all-scope token A: get_doc {pageA} -> ${own.status}, ${controlOk ? 'returned data (control)' : 'refused (control failed)'}`)
    const calls: [string, string, string, Record<string, string>, string][] = [
      ['all-scope token A', a.writeToken, 'get_doc', { page_id: b.pageId }, 'Eval B'],
      ['all-scope token A', a.writeToken, 'get_task', { task_id: b.taskId }, 'Eval B'],
      ['all-scope token A', a.writeToken, 'update_task', { task_id: b.taskId, expected_version: '1', title: 'changed through MCP' }, 'changed through MCP'],
      ['read token A', a.readToken, 'get_doc', { page_id: a.pageId }, 'Eval A paragraph'],
      ['read token A', a.readToken, 'create_task', { project_id: a.projectId, status_id: a.statusId, title: 'MCP write with read token' }, 'MCP write'],
    ]
    for (const [label, token, name, args, marker] of calls) {
      const reply = await mcp(token, 'tools/call', { name, arguments: args })
      const leaked = returned(reply, marker)
      mcpEvidence.push(names(`${label}: ${name} ${JSON.stringify(Object.values(args)[0])} -> ${reply.status}, ${leaked ? 'returned data' : 'refused'}`))
      if (leaked) mcpBad.push(names(`${label}: ${name} ${JSON.stringify(Object.values(args)[0])} returned data`))
    }
  }
  add(check('tokens.mcp-scope', 'tokens', 'MCP: a token of A cannot reach B, and a read token cannot read docs or write', controlOk ? mcpBad.length === 0 : 'skip', 'high', mcpBad.length ? mcpBad : mcpEvidence))

  // OAuth: a grant for workspace A with tasks:read only.
  const oauthEvidence: string[] = []
  const oauthBad: string[] = []
  let oauthResult: boolean | 'skip' = 'skip'
  let oauthControl = false
  try {
    const redirect = 'http://127.0.0.1:9/callback'
    const client = await call('POST', '/oauth/register', { origin: null, ip: '198.51.100.42', body: { redirect_uris: [redirect], client_name: 'evals', token_endpoint_auth_method: 'none' } })
    oauthEvidence.push(line('POST', '/oauth/register', client))
    const verifier = randomBytes(32).toString('base64url')
    const challenge = createHash('sha256').update(verifier).digest('base64url')
    const query = new URLSearchParams({ response_type: 'code', client_id: client.json.client_id, redirect_uri: redirect, code_challenge: challenge, code_challenge_method: 'S256', state: 'evals', scope: 'tasks:read', resource: `${server.origin}/mcp` })
    const authorize = await call('GET', `/oauth/authorize?${query}`, { session: sessionA, ip: '198.51.100.42' })
    oauthEvidence.push(`GET /oauth/authorize -> ${authorize.status}`)
    const requestId = new URL(authorize.headers.get('location') ?? '', server.origin).searchParams.get('request')
    const consent = await call('GET', `/api/v1/oauth/consent/${requestId}`, { session: sessionA, ip: '198.51.100.42' })
    oauthEvidence.push(`GET /api/v1/oauth/consent/{id} -> ${consent.status}`)
    const approval = await call('POST', `/api/v1/oauth/consent/${requestId}`, {
      session: sessionA,
      ip: '198.51.100.42',
      body: { csrf_token: consent.json?.csrf_token, approved: true, workspace_id: a.workspaceId, project_ids: [a.projectId], scopes: ['tasks:read'] },
    })
    oauthEvidence.push(`POST /api/v1/oauth/consent/{id} -> ${approval.status}`)
    const code = new URL(approval.json?.redirect_to ?? approval.json?.redirect ?? approval.json?.location ?? Object.values(approval.json ?? {}).find((v) => typeof v === 'string' && v.startsWith(redirect)) ?? 'http://x').searchParams.get('code')
    const tokenReply = await fetch(`${server.base}/oauth/token`, {
      method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded', 'x-forwarded-proto': 'https', 'x-forwarded-for': '198.51.100.42' },
      body: new URLSearchParams({ grant_type: 'authorization_code', code: code ?? '', code_verifier: verifier, client_id: client.json.client_id, redirect_uri: redirect, resource: `${server.origin}/mcp` }),
    })
    const token = (await tokenReply.json().catch(() => ({}))) as { access_token?: string }
    oauthEvidence.push(`POST /oauth/token -> ${tokenReply.status}`)
    if (token.access_token) {
      oauthResult = true
      const tools = await mcp(token.access_token, 'tools/list', {})
      const names: string[] = tools.data?.result?.tools?.map((tool: { name: string }) => tool.name) ?? []
      oauthEvidence.push(`tasks:read token tools/list -> ${tools.status}: ${names.join(', ')}`)
      for (const [name, args, marker] of [
        ['create_task', { project_id: a.projectId, status_id: a.statusId, title: 'OAuth write with read scope' }, 'OAuth write'],
        ['get_doc', { page_id: a.pageId }, 'Eval A paragraph'],
        ['get_task', { task_id: b.taskId }, 'Eval B'],
        ['list_tasks', {}, 'Eval A task'],
      ] as const) {
        const reply = await mcp(token.access_token, 'tools/call', { name, arguments: args })
        const leaked = returned(reply, marker)
        oauthEvidence.push(`tasks:read token tools/call ${name} -> ${reply.status}, ${leaked ? 'returned data' : 'refused'}`)
        // list_tasks in A is the control: tasks:read may do it.
        if (name === 'list_tasks') oauthControl = leaked
        else if (leaked) oauthBad.push(`tasks:read OAuth token: ${name} returned data`)
      }
      const rest = await call('GET', `${ws(a.workspaceId)}/pages`, { bearer: token.access_token, origin: null, ip: '198.51.100.42' })
      oauthEvidence.push(`OAuth token on REST ${line('GET', `${ws(a.workspaceId)}/pages`, rest)}`)
      if (rest.status < 300) oauthBad.push('OAuth MCP token works on the REST API')
    }
  } catch (error) {
    oauthEvidence.push(`flow stopped: ${(error as Error).message.slice(0, 120)}`)
  }
  add(check('tokens.oauth-scope', 'tokens', 'OAuth tasks:read token cannot write, read docs, or reach B', oauthResult === true && oauthControl ? oauthBad.length === 0 : 'skip', 'high', oauthBad.length ? oauthBad : oauthEvidence))
}

// --- CSRF and cross-site WebSocket ------------------------------------------------------------------------------
{
  const path = `${ws(a.workspaceId)}/pages`
  const cases: [string, string | null, number[]][] = [
    ['foreign Origin', 'https://evil.example', [403]],
    ['Origin null', 'null', [403]],
    ['no Origin with cookie', null, [403]],
    ['sibling http Origin', 'http://orbit.test', [403]],
  ]
  const evidence: string[] = []
  const bad: string[] = []
  for (const [label, origin, expected] of cases) {
    const reply = await call('POST', path, { session: sessionA, origin, body: { title: `csrf ${label}` }, ip: '198.51.100.50' })
    evidence.push(`${label}: ${line('POST', path, reply)}`)
    if (!expected.includes(reply.status)) bad.push(`${label}: ${line('POST', path, reply)}`)
  }
  const simple = await fetch(server.base + path, {
    method: 'POST',
    headers: { 'content-type': 'text/plain', origin: 'https://evil.example', cookie: `${server.cookie}=${sessionA}`, 'x-forwarded-proto': 'https', 'x-forwarded-for': '198.51.100.50' },
    body: JSON.stringify({ title: 'csrf text/plain' }),
  })
  evidence.push(`text/plain simple request from foreign Origin -> ${simple.status}`)
  if (simple.status < 300) bad.push(`text/plain simple request -> ${simple.status}`)
  const control = await call('POST', path, { session: sessionA, body: { title: 'csrf control' }, ip: '198.51.100.50' })
  evidence.push(`same Origin (control): ${line('POST', path, control)}`)
  add(check('csrf.origin', 'csrf', 'Cookie writes from a foreign or missing Origin are refused', bad.length === 0 && control.status === 201, 'high', bad.length ? bad : evidence))

  // Cross-site WebSocket hijacking: an upgrade with the cookie from a foreign Origin.
  const upgrade = (origin: string) =>
    spawnSync('curl', ['-s', '-o', '/dev/null', '-w', '%{http_code}', '--max-time', '3', '-H', 'Connection: Upgrade', '-H', 'Upgrade: websocket', '-H', 'Sec-WebSocket-Version: 13', '-H', `Sec-WebSocket-Key: ${randomBytes(16).toString('base64')}`, '-H', `Origin: ${origin}`, '-H', `Cookie: ${server.cookie}=${sessionA}`, '-H', 'X-Forwarded-Proto: https', `${server.base}${ws(a.workspaceId)}/live`], { encoding: 'utf8' }).stdout.trim()
  const foreign = upgrade('https://evil.example')
  const own = upgrade(server.origin)
  add(check('csrf.websocket', 'csrf', 'WebSocket upgrade from a foreign Origin is refused', foreign !== '101' && foreign !== '000', 'high', [`GET ${ws('{A}')}/live upgrade, foreign Origin -> ${foreign}`, `same Origin (control) -> ${own}`]))
}

// --- Uploads: size limit and active content -----------------------------------------------------------------------
{
  const files = `${ws(a.workspaceId)}/pages/${a.pageId}/files`
  const upload = async (name: string, type: string, content: BlobPart) => {
    const form = new FormData()
    form.append('file', new File([content], name, { type }))
    return call('POST', files, { session: sessionA, form, ip: '198.51.100.60', timeout: 120_000 })
  }
  const big = await upload('big.bin', 'application/octet-stream', new Uint8Array(25 * 1024 * 1024 + 1))
  add(check('upload.size-limit', 'upload', 'A file over the 25 MiB limit is refused', big.status === 413 || (big.status >= 400 && big.status < 500), 'medium', [`POST ${files.replace(a.workspaceId, '{A}')} (25 MiB + 1 byte) -> ${big.status}`]))

  const active: [string, string, string][] = [
    ['evil.html', 'text/html', '<html><script>document.title="xss"</script></html>'],
    ['evil.svg', 'image/svg+xml', '<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>'],
    ['evil.txt', 'text/html', '<script>alert(1)</script>'],
  ]
  const unsafe: string[] = []
  const evidence: string[] = []
  for (const [name, type, content] of active) {
    const reply = await upload(name, type, content)
    if (reply.status !== 201) {
      evidence.push(`upload ${name} (${type}) -> ${reply.status} (refused)`)
      continue
    }
    for (const url of [reply.json.url, `${files}/${reply.json.id}`].filter(Boolean)) {
      const download = await call('GET', url, { session: sessionA, ip: '198.51.100.60' })
      const contentType = download.headers.get('content-type') ?? ''
      const disposition = download.headers.get('content-disposition') ?? ''
      const csp = download.headers.get('content-security-policy') ?? ''
      const nosniff = download.headers.get('x-content-type-options') === 'nosniff'
      const rendersActive = /html|svg|xml|javascript/i.test(contentType) && !/^attachment/i.test(disposition) && !/sandbox|default-src 'none'/.test(csp)
      evidence.push(`GET ${name} -> ${download.status}, type ${contentType || '-'}, disposition ${disposition.split(';')[0] || '-'}, nosniff ${nosniff}, csp ${csp ? 'yes' : 'no'}`)
      if (download.status === 200 && (rendersActive || !nosniff)) unsafe.push(`${name}: served as ${contentType} inline`)
    }
  }
  add(check('upload.active-content', 'upload', 'Uploaded HTML/SVG is never served as active inline content', unsafe.length === 0, 'high', unsafe.length ? unsafe : evidence, 'Serve user files with Content-Disposition: attachment (or a sandbox CSP) and nosniff.'))

  const traversalName = await upload('../../../../tmp/evals-traversal.txt', 'text/plain', 'traversal')
  const stray: string[] = []
  const walk = (dir: string) => {
    for (const entry of readdirSync(dir)) {
      const path = join(dir, entry)
      if (statSync(path).isDirectory()) walk(path)
      else if (entry.includes('evals-traversal')) stray.push(path.replace(server.dataDir, '{data}'))
    }
  }
  walk(join(server.dataDir, '..'))
  const storedName = traversalName.json?.file_name ?? ''
  add(check('upload.filename-traversal', 'path-traversal', 'An upload name with ../ cannot write outside storage', stray.length === 0 && !storedName.includes('/'), 'high', [`upload ../../../../tmp/evals-traversal.txt -> ${traversalName.status}, stored name "${storedName}"`, `files named evals-traversal on disk: ${stray.length ? stray.join(', ') : 'none'}`]))
}

// --- Path traversal on static and file routes ---------------------------------------------------------------------
{
  const paths = [
    '/assets/../../../../etc/passwd',
    '/assets/..%2f..%2f..%2f..%2fetc%2fpasswd',
    '/%2e%2e/%2e%2e/%2e%2e/etc/passwd',
    '/assets/%2e%2e%2f%2e%2e%2fCargo.toml',
    '/..%5c..%5c..%5cCargo.toml',
    `${ws(a.workspaceId)}/pages/${a.pageId}/files/..%2f..%2f..%2f..%2fetc%2fpasswd/content`,
    `${ws(a.workspaceId)}/pages/${a.pageId}/files/${a.fileId}%2f..%2f..%2f..%2fetc%2fpasswd`,
    `/api/v1/admin/backups/..%2f..%2f..%2fetc%2fpasswd/download`,
    `${ws(a.workspaceId)}/exports/..%2f..%2f..%2fetc%2fpasswd/download`,
    `${ws(a.workspaceId)}/chat/emoji/..%2f..%2f..%2fetc%2fpasswd/image`,
  ]
  const leaks: string[] = []
  const evidence: string[] = []
  for (const path of paths) {
    const result = spawnSync('curl', ['-s', '--path-as-is', '--max-time', '10', '-w', '\n%{http_code}', '-H', `Cookie: ${server.cookie}=${sessionA}`, '-H', 'X-Forwarded-Proto: https', '-H', 'X-Forwarded-For: 198.51.100.70', `${server.base}${path}`], { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 })
    const status = result.stdout.trim().split('\n').pop()
    const body = result.stdout
    const shown = path.replace(a.workspaceId, '{A}').replace(a.pageId, '{page}').replace(a.fileId, '{file}')
    evidence.push(`GET ${shown} -> ${status}`)
    if (/root:x?:0:0|\[workspace\]|\[package\]/.test(body)) leaks.push(`GET ${shown} -> ${status} returned a file outside the app`)
  }
  add(check('path-traversal.routes', 'path-traversal', 'Encoded ../ paths never return files outside the app', leaks.length === 0, 'high', leaks.length ? leaks : evidence))
}

// --- SSRF through link previews and agent integrations -----------------------------------------------------------
{
  const hits: string[] = []
  const canary = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    fetch(request) {
      hits.push(new URL(request.url).pathname)
      return new Response('<html><head><title>EVALS CANARY</title><meta property="og:title" content="EVALS CANARY"></head></html>', { headers: { 'content-type': 'text/html' } })
    },
  })
  const port = canary.port
  const preview = (url: string) => call('GET', `${ws(a.workspaceId)}/chat/link-preview?url=${encodeURIComponent(url)}`, { session: sessionA, ip: '198.51.100.80', timeout: 20_000 })
  const targets = [
    `http://127.0.0.1:${port}/loopback`,
    `http://localhost:${port}/localhost`,
    `http://[::ffff:127.0.0.1]:${port}/mapped`,
    `http://2130706433:${port}/decimal`,
    `http://0x7f000001:${port}/hex`,
    `http://127.1:${port}/short`,
    `http://0.0.0.0:${port}/zero`,
    `http://127.0.0.1.nip.io:${port}/dns`,
    'http://169.254.169.254/latest/meta-data/',
    'http://metadata.google.internal/computeMetadata/v1/',
    'file:///etc/passwd',
    `gopher://127.0.0.1:${port}/_gopher`,
  ]
  const evidence: string[] = []
  const bad: string[] = []
  const control = await preview('https://example.com/')
  const external = control.status === 200 && !!control.json?.preview
  evidence.push(`control https://example.com/ -> ${control.status}, preview ${external ? 'yes' : 'no'}`)
  for (const target of targets) {
    const before = hits.length
    const reply = await preview(target)
    const leaked = hits.length > before || /EVALS CANARY|root:x:0:0|ami-id|computeMetadata/.test(reply.text)
    evidence.push(`${target.replace(String(port), '{canary}')} -> ${reply.status}, ${leaked ? 'FETCHED' : 'not fetched'}`)
    if (leaked) bad.push(`${target.replace(String(port), '{canary}')} reached an internal address`)
  }
  if (external) {
    // A public URL that redirects to the loopback canary.
    const before = hits.length
    const reply = await preview(`https://httpbin.org/redirect-to?url=${encodeURIComponent(`http://127.0.0.1:${port}/redirect`)}`)
    const leaked = hits.length > before || reply.text.includes('EVALS CANARY')
    evidence.push(`public redirect to loopback -> ${reply.status}, ${leaked ? 'FETCHED' : 'not fetched'}`)
    if (leaked) bad.push('a public redirect to loopback was followed')
  } else evidence.push('public redirect test skipped: the instance cannot reach the internet')
  add(check('ssrf.link-preview', 'ssrf', 'Link previews never fetch internal addresses (incl. redirects, DNS, odd IP forms)', control.status === 404 ? 'skip' : bad.length === 0, 'high', bad.length ? bad : evidence))

  // Agent integrations call a URL that a workspace admin enters. Their test call must also refuse internal targets.
  const integrations = `${ws(a.workspaceId)}/agent-integrations`
  const before = hits.length
  const created = await call('POST', integrations, { session: sessionA, ip: '198.51.100.81', body: { name: 'evals', url: `http://127.0.0.1:${port}/integration`, auth: { type: 'none' }, enabled: true } })
  const agentEvidence = [`POST ${integrations.replace(a.workspaceId, '{A}')} (url http://127.0.0.1:{canary}) -> ${created.status}`]
  if (created.status < 300 && created.json?.id) {
    const test = await call('POST', `${integrations}/${created.json.id}/test`, { session: sessionA, ip: '198.51.100.81', body: {} })
    agentEvidence.push(`POST …/agent-integrations/{id}/test -> ${test.status}`)
    await call('DELETE', `${integrations}/${created.json.id}`, { session: sessionA, ip: '198.51.100.81' })
  }
  const reached = hits.length > before
  agentEvidence.push(`canary hits: ${hits.length - before}`)
  add(check('ssrf.agent-integration', 'ssrf', 'Agent integration URLs cannot target internal addresses', created.status === 404 ? 'skip' : !reached, 'medium', agentEvidence))
  canary.stop(true)
}

// --- Sessions and account enumeration -----------------------------------------------------------------------------
{
  const unknown = await call('POST', '/api/v1/auth/login', { body: { email: 'nobody@example.test', password: 'Wrong-password-1' }, ip: '198.51.100.90' })
  const wrong = await call('POST', '/api/v1/auth/login', { body: { email: a.email, password: 'Wrong-password-1' }, ip: '198.51.100.91' })
  const same = unknown.status === wrong.status && unknown.json?.code === wrong.json?.code && unknown.json?.detail === wrong.json?.detail
  add(check('auth.login-enumeration', 'auth', 'Login answers the same for an unknown email and a wrong password', same, 'low', [`unknown email -> ${unknown.status} ${unknown.json?.code}`, `wrong password -> ${wrong.status} ${wrong.json?.code}`]))
  const recoveryKnown = await call('POST', '/api/v1/auth/recovery/request', { body: { email: a.email }, ip: '198.51.100.92' })
  const recoveryUnknown = await call('POST', '/api/v1/auth/recovery/request', { body: { email: 'nobody@example.test' }, ip: '198.51.100.93' })
  add(check('auth.recovery-enumeration', 'auth', 'Password reset answers the same for known and unknown emails', recoveryKnown.status === recoveryUnknown.status && recoveryKnown.text === recoveryUnknown.text, 'low', [`known -> ${recoveryKnown.status}`, `unknown -> ${recoveryUnknown.status}`, `same body: ${recoveryKnown.text === recoveryUnknown.text}`]))

  const session = sessionFrom(await call('POST', '/api/v1/auth/login', { body: { email: b.email, password: seed.password }, ip: '198.51.100.94' }))
  const logout = await call('POST', '/api/v1/auth/logout', { session, ip: '198.51.100.94' })
  const reuse = await call('GET', '/api/v1/auth/me', { session, ip: '198.51.100.94' })
  add(check('auth.logout', 'auth', 'A session cookie stops working after logout', reuse.status === 401, 'medium', [`POST /api/v1/auth/logout -> ${logout.status}`, `GET /api/v1/auth/me with the old cookie -> ${reuse.status}`]))
}

// --- Rate limits (last: they leave buckets full) -------------------------------------------------------------------
{
  const limit = async (label: string, attempts: number, request: () => Promise<Reply>) => {
    const statuses: number[] = []
    for (let index = 0; index < attempts; index++) {
      const reply = await request()
      statuses.push(reply.status)
      if (reply.status === 429) break
    }
    const at = statuses.indexOf(429)
    return { at, evidence: `${label}: ${statuses.length} requests, ${at >= 0 ? `first 429 at request ${at + 1}` : 'no 429'} (statuses ${[...new Set(statuses)].join(', ')})` }
  }
  const attempts = quick ? 40 : 60
  const loginLimit = await limit('POST /api/v1/auth/login wrong password', attempts, () =>
    call('POST', '/api/v1/auth/login', { body: { email: a.email, password: 'Wrong-password-1' }, ip: '203.0.113.10' }),
  )
  add(check('ratelimit.login', 'rate-limit', 'Login is rate limited per client', loginLimit.at >= 0, 'medium', [loginLimit.evidence]))
  const spread = await limit('POST /api/v1/auth/login, account B, a new IP each time', attempts, () =>
    call('POST', '/api/v1/auth/login', { body: { email: b.email, password: 'Wrong-password-1' }, ip: `203.0.113.${100 + Math.floor(Math.random() * 150)}` }),
  )
  add(check('ratelimit.login-account', 'rate-limit', 'Login attempts on one account are limited across IPs', spread.at >= 0 ? true : 'warn', 'low', [spread.evidence], 'Add a per-account failed-login limit or lockout (credential stuffing from many IPs).'))
  // The other side of a per-account limit: can strangers lock the real user out?
  const owner = await call('POST', '/api/v1/auth/login', { body: { email: b.email, password: seed.password }, ip: '203.0.113.99' })
  add(check('ratelimit.login-lockout', 'rate-limit', 'Failed attempts from other IPs do not lock the real user out', owner.status === 200 ? true : 'warn', 'low', [`after the attempts above, correct password from a new IP -> ${owner.status}`], 'Count failures per account and IP, or allow a known device / passkey past the account limit.'))
  const twoFactor = await limit('POST /api/v1/auth/login/two-factor', attempts, () =>
    call('POST', '/api/v1/auth/login/two-factor', { body: { challenge: 'evals', code: '000000' }, ip: '203.0.113.20' }),
  )
  add(check('ratelimit.two-factor', 'rate-limit', 'Two-factor code entry is rate limited', twoFactor.at >= 0, 'medium', [twoFactor.evidence]))
  const recovery = await limit('POST /api/v1/auth/recovery/request', 30, () =>
    call('POST', '/api/v1/auth/recovery/request', { body: { email: 'nobody@example.test' }, ip: '203.0.113.30' }),
  )
  add(check('ratelimit.recovery', 'rate-limit', 'Password reset requests are rate limited', recovery.at >= 0, 'medium', [recovery.evidence]))
}

writePart('security-local', { checks })
const failed = checks.filter((item) => item.result === 'fail').length
console.log(`security-local: ${checks.length} checks, ${failed} failed`)
