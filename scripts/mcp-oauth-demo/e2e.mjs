// Browser proof of "Connect" for remote MCP servers (docs/agent.md): saves PNGs and fails on the first wrong result.
// Needs an empty Orbit on 127.0.0.1:4320 (ORBIT_AGENT_INTEGRATION_TEST_HOSTS=127.0.0.1, the agent sidecar with
// AGENT_MOCK=1) and scripts/mcp-oauth-demo/server.mjs on 4310:
//   node scripts/mcp-oauth-demo/e2e.mjs <setup token> <output directory> <chromium executable>
import { randomBytes } from 'node:crypto'
import { writeFileSync } from 'node:fs'
import { chromium } from '../../node_modules/playwright-core/index.mjs'

const [token, out, executablePath] = process.argv.slice(2)
const ORBIT = 'http://127.0.0.1:4320'
const MOCK = 'http://127.0.0.1:4310'
const passwords = { owner: randomBytes(18).toString('base64url'), member: randomBytes(18).toString('base64url') }
const emails = { owner: 'admin@orbit-mcp.test', member: 'member@orbit-mcp.test' }

function check(ok, what) {
  if (!ok) throw new Error(`FAILED: ${what}`)
  console.log(`ok - ${what}`)
}

async function api(path, { method = 'GET', body, cookie } = {}) {
  const response = await fetch(`${ORBIT}${path}`, {
    method,
    headers: { origin: ORBIT, 'content-type': 'application/json', ...(cookie ? { cookie } : {}) },
    body: body ? JSON.stringify(body) : undefined,
  })
  const text = await response.text()
  if (!response.ok) throw new Error(`${method} ${path}: ${response.status} ${text}`)
  return { data: text ? JSON.parse(text) : null, cookie: response.headers.get('set-cookie')?.split(';')[0] }
}

// The workspace, its admin and a member.
const setup = await api('/api/v1/setup/complete', { method: 'POST', body: { token, email: emails.owner, display_name: 'Ada Admin', password: passwords.owner } })
const cookie = setup.cookie
const workspace = (await api('/api/v1/workspaces', { method: 'POST', cookie, body: { name: 'Wellrox' } })).data
const workspaceId = workspace.id ?? workspace.workspace?.id
const invitation = (await api(`/api/v1/workspaces/${workspaceId}/invitations`, { method: 'POST', cookie, body: { email: emails.member, role: 'member', delivery: 'manual' } })).data
await api('/api/v1/workspaces/invitations/accept', { method: 'POST', body: { token: new URL(invitation.url).searchParams.get('token'), email: emails.member, display_name: 'Mia Member', password: passwords.member } })
writeFileSync(`${out}/../logins.txt`, `admin ${emails.owner} ${passwords.owner}\nmember ${emails.member} ${passwords.member}\n`, { mode: 0o600 })

// The admin's ChatGPT connection (the mock model "signs in" after three seconds).
await api('/api/v1/agent/chatgpt/login', { method: 'POST', cookie })
for (let tries = 0; ; tries += 1) {
  const status = (await api('/api/v1/agent', { cookie })).data
  if (status.chatgpt?.connected) break
  if (tries > 30) throw new Error(`ChatGPT mock login did not finish: ${JSON.stringify(status)}`)
  await new Promise((resolve) => setTimeout(resolve, 500))
}
const agent = (await api('/api/v1/agent', { cookie })).data

const browser = await chromium.launch({ executablePath })
const context = await browser.newContext({ viewport: { width: 1400, height: 900 } })
const page = await context.newPage()
const shot = (name) => page.screenshot({ path: `${out}/${name}.png` })
await page.goto(`${ORBIT}/login`)
await page.getByLabel('Email', { exact: true }).fill(emails.owner)
await page.getByLabel('Password', { exact: true }).fill(passwords.owner)
await page.getByRole('button', { name: 'Sign in', exact: true }).click()
await page.waitForURL((url) => !url.pathname.startsWith('/login'))

/** Agentic → Workspace → Add tool: a remote MCP server with the Connect credential, then the mock's authorize page. */
async function addAndConnect(name, path, shots) {
  await page.goto(`${ORBIT}/settings/agentic/workspace`)
  await page.getByRole('button', { name: 'Add tool' }).click()
  await page.locator('#agent-tool-name').fill(name)
  await page.locator('#agent-tool-url').fill(`${MOCK}${path}`)
  await page.locator('#agent-tool-mcp-auth').click()
  await page.getByRole('option', { name: 'Sign in with OAuth (Connect)' }).click()
  if (shots) await shot('orbit-mcp-oauth-1-add-form')
  await page.getByRole('button', { name: 'Add and connect' }).click()
  await page.waitForURL((url) => url.href.startsWith(`${MOCK}/oauth/authorize`))
  const authorize = new URL(page.url())
  check(authorize.searchParams.get('code_challenge_method') === 'S256' && authorize.searchParams.get('resource') === `${MOCK}${path}`, `${name}: the authorize URL has PKCE S256 and the resource`)
  check(authorize.searchParams.get('redirect_uri') === `${ORBIT}/api/v1/agent-integrations/oauth/callback`, `${name}: the redirect URI is Orbit's callback`)
  if (shots) await shot('orbit-mcp-oauth-2-authorize-page')
  await page.getByRole('button', { name: 'Authorize' }).click()
  await page.waitForURL((url) => url.href.startsWith(`${ORBIT}/settings/agentic/workspace`))
  await page.getByText('Connected. Orbit read the server’s tools.').waitFor()
  check(!page.url().includes('agent_tool_connect'), `${name}: the outcome leaves the address after the toast`)
}

await addAndConnect('TikTok Social', '/mcp', true)
const row = page.locator('[data-slot=integration-row]').filter({ hasText: 'TikTok Social' })
await row.getByText(/Connected as @wellrox_brand · until /).waitFor()
check(await row.getByRole('button', { name: 'Reconnect' }).isVisible(), 'the connected row has Reconnect')
check((await row.textContent()).includes('1 of 2 tools on · OAuth sign-in'), 'the tools were read at the sign-in; only the read-only tool is on')
await shot('orbit-mcp-oauth-3-connected')

// The list answer to the browser has no token.
const listed = JSON.stringify((await api(`/api/v1/workspaces/${workspaceId}/agent-integrations`, { cookie })).data)
check(listed.includes('"account":"@wellrox_brand"') && !/access_token|refresh_token|client-/.test(listed), 'the API shows the account and no token or client')

// A server whose grant lasts 10 days: the row warns.
await addAndConnect('TikTok Short', '/mcp-short', false)
const short = page.locator('[data-slot=integration-row]').filter({ hasText: 'TikTok Short' })
await short.getByText(/sign-in ends in 10 days .* Reconnect before then\./).waitFor()
await shot('orbit-mcp-oauth-4-expiry-warning')

// Edit: the sign-in details, Disconnect, and the write tool turned on.
await row.getByRole('button', { name: 'Edit TikTok Social' }).click()
await row.getByRole('switch', { name: 'Allow reply_comment' }).click()
await page.waitForTimeout(800)
check(await row.getByRole('switch', { name: 'Allow reply_comment' }).getAttribute('aria-checked') === 'true', 'the write tool is on')
await shot('orbit-mcp-oauth-5-details')

// Donna: a read with the mock tool, then a write that needs the driver's yes.
const conversations = (await api(`/api/v1/workspaces/${workspaceId}/chat/conversations`, { cookie })).data
const general = conversations.find((conversation) => conversation.is_default).id
let nonce = 0
async function post(body, root) {
  nonce += 1
  const { data } = await api(`/api/v1/workspaces/${workspaceId}/chat/conversations/${general}/messages`, { method: 'POST', cookie, body: { body, thread_root_id: root ?? null, nonce: `e2e-${nonce}` } })
  return data.result.id
}
const readRoot = await post(`<@${agent.bot_user_id}> use tiktok_social__list_videos with {"limit": 3}`)
await page.goto(`${ORBIT}/chat/${general}/thread/${readRoot}`)
await page.getByText(/Behind the scenes at Wellrox/).last().waitFor({ timeout: 60000 })
await page.waitForTimeout(1500)
await shot('orbit-mcp-oauth-6-donna-read')

const writeRoot = await post(`<@${agent.bot_user_id}> use tiktok_social__reply_comment with {"comment_id": "c-42", "text": "Thank you!"}`)
await page.goto(`${ORBIT}/chat/${general}/thread/${writeRoot}`)
await page.getByText(/reply_comment/).last().waitFor({ timeout: 60000 })
await page.getByText(/yes/i).last().waitFor({ timeout: 60000 })
await page.waitForTimeout(1500)
check((await (await fetch(`${MOCK}/posts`)).json()).length === 0, 'nothing was posted before the yes')
await shot('orbit-mcp-oauth-7-write-asks-yes')
await post('yes', writeRoot)
await page.getByText(/Replied to c-42: Thank you!/).last().waitFor({ timeout: 60000 })
await page.waitForTimeout(1500)
check((await (await fetch(`${MOCK}/posts`)).json()).length === 1, 'the reply was posted once, after the yes')
await shot('orbit-mcp-oauth-8-write-done')

await browser.close()
console.log('done')
