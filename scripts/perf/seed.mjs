#!/usr/bin/env node
// Seeds an empty Orbit for the perf measurements (scripts/perf/measure.mjs) through the HTTP API: the first account
// (a strong random password), a workspace, 300 tasks, a long docs page, docs pages with a 150-page PDF, an .xlsx sheet
// and Mermaid diagrams, a chat channel with a long history, and an embedded app with sign-in.
//
//   node scripts/perf/seed.mjs <orbit url> <setup token> <fixtures directory> <output directory> <app url>
// Writes <output>/logins.txt (mode 600), <output>/seed.json (ids for measure.mjs) and <output>/demo.env (the app's
// client id and secret for scripts/apps-demo/server.mjs).
import { randomBytes, randomUUID } from 'node:crypto'
import { readFileSync, writeFileSync } from 'node:fs'

const [BASE, token, fixtures, out, appUrl] = process.argv.slice(2)
const email = 'perf-admin@orbit-perf.test'
const password = randomBytes(24).toString('base64url')
let cookie = ''

async function api(method, path, body) {
  const response = await fetch(`${BASE}${path}`, {
    method,
    headers: { 'content-type': 'application/json', origin: BASE, 'x-orbit-contract': 'orbit-api-v1', ...(cookie ? { cookie } : {}) },
    body: body === undefined ? undefined : JSON.stringify(body),
  })
  const setCookie = response.headers.get('set-cookie')
  if (setCookie) cookie = setCookie.split(';')[0]
  const text = await response.text()
  if (!response.ok) throw new Error(`${method} ${path} → ${response.status}: ${text}`)
  return text ? JSON.parse(text) : undefined
}

async function upload(path, name, type) {
  const form = new FormData()
  form.append('file', new Blob([readFileSync(`${fixtures}/${name}`)], { type }), name)
  const response = await fetch(`${BASE}${path}`, { method: 'POST', headers: { origin: BASE, 'x-orbit-contract': 'orbit-api-v1', cookie }, body: form })
  if (!response.ok) throw new Error(`POST ${path} → ${response.status}: ${await response.text()}`)
  return response.json()
}

const items = (list) => (Array.isArray(list) ? list : list.items)
const text = (value) => [{ type: 'text', text: value, styles: {} }]
const block = (type, value, props = {}) => ({ type, props, content: text(value), children: [] })
const lorem = (n) => `Orbit keeps tasks, docs and chat in one place. Paragraph ${n} explains the plan, the open risks and the next steps for the team, with enough words to wrap over a few lines in the editor.`

await api('POST', '/api/v1/setup/complete', { token, email, display_name: 'Perf Admin', password })
writeFileSync(`${out}/logins.txt`, `admin ${email} ${password}\n`, { mode: 0o600 })
const created = await api('POST', '/api/v1/workspaces', { name: 'Perf Workspace' })
const workspaceId = created.id ?? created.workspace?.id
const ws = `/api/v1/workspaces/${workspaceId}`

// Tasks: one project, 300 tasks over its statuses.
const project = await api('POST', `${ws}/projects`, { name: 'Product', key: 'PRD', color: '#e5488a' })
// New tasks start open: only the unstarted and started statuses take them.
const statuses = items(await api('GET', `${ws}/projects/${project.id}/statuses`)).filter((status) => ['unstarted', 'started'].includes(status.category))
for (let n = 1; n <= 300; n++) {
  await api('POST', `${ws}/tasks`, {
    project_id: project.id,
    status_id: statuses[n % statuses.length].id,
    title: `Task ${n}: ${['Fix the import of large files', 'Design the billing page', 'Write the release notes', 'Speed up the search', 'Review the API limits'][n % 5]}`,
    priority: ['none', 'low', 'medium', 'high', 'urgent'][n % 5],
  })
}
console.log('+ 300 tasks')

// Docs: a long page, a page with a PDF, a page with a sheet, a page with diagrams.
const page = async (title, content) => {
  const made = await api('POST', `${ws}/pages`, { title })
  if (content) {
    const current = await api('GET', `${ws}/pages/${made.id}`)
    await api('PATCH', `${ws}/pages/${made.id}`, { expected_version: current.version, content })
  }
  return made.id
}
const long = []
for (let section = 1; section <= 60; section++) {
  long.push(block('heading', `Section ${section}`, { level: 2 }))
  for (let n = 0; n < 6; n++) long.push(block('paragraph', lorem(section * 10 + n)))
  long.push(block('bulletListItem', `Point A of section ${section}`), block('bulletListItem', `Point B of section ${section}`), block('checkListItem', `Check section ${section}`, { checked: section % 2 === 0 }))
  if (section % 10 === 0) long.push({ type: 'codeBlock', props: { language: 'typescript' }, content: text(`export function section${section}(input: number): number {\n  return input * ${section}\n}`), children: [] })
}
const longPage = await page('Long handbook', long)
const fileBlock = (file, name) => ({ type: 'file', props: { url: file.url, name, caption: '' }, children: [] })
const pdfPage = await page('Report with PDF')
const pdfFile = await upload(`${ws}/pages/${pdfPage}/files`, 'report.pdf', 'application/pdf')
await api('PATCH', `${ws}/pages/${pdfPage}`, { expected_version: (await api('GET', `${ws}/pages/${pdfPage}`)).version, content: [block('paragraph', 'The quarterly report:'), fileBlock(pdfFile, 'report.pdf')] })
const sheetPage = await page('Sales sheet')
const sheetFile = await upload(`${ws}/pages/${sheetPage}/files`, 'sales.xlsx', 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet')
await api('PATCH', `${ws}/pages/${sheetPage}`, { expected_version: (await api('GET', `${ws}/pages/${sheetPage}`)).version, content: [block('paragraph', 'The sales sheet:'), fileBlock(sheetFile, 'sales.xlsx')] })
const mermaid = (source) => ({ type: 'codeBlock', props: { language: 'mermaid' }, content: text(source), children: [] })
const diagramPage = await page('Architecture diagrams', [
  block('paragraph', 'How a request flows through Orbit.'),
  mermaid('flowchart LR\n  Browser --> Proxy --> Server\n  Server --> DB[(SQLite)]\n  Server --> Files[(Storage)]\n  Server --> Agent'),
  mermaid('sequenceDiagram\n  participant B as Browser\n  participant S as Server\n  B->>S: GET /api/v1/tasks\n  S-->>B: 200 tasks\n  B->>S: WebSocket events\n  S-->>B: change'),
  mermaid('classDiagram\n  class Task { title status priority }\n  class Project { name key }\n  Project "1" --> "*" Task'),
])
console.log('+ docs pages')

// Chat: a channel with a long history (markdown, code and links).
const channel = await api('POST', `${ws}/chat/conversations`, { name: 'engineering', kind: 'public' })
for (let n = 1; n <= 1500; n++) {
  const body = n % 25 === 0
    ? `Message ${n} with code:\n\`\`\`ts\nconst total = items.reduce((sum, item) => sum + item.price, 0) // ${n}\n\`\`\``
    : n % 7 === 0 ? `Message ${n}: see **the plan** in _docs_ and https://example.com/item/${n}` : `Message ${n}: ${lorem(n).slice(0, 40 + (n % 90))}`
  await api('POST', `${ws}/chat/conversations/${channel.result.id}/messages`, { body, nonce: randomUUID() })
}
console.log('+ 1500 chat messages')

// An embedded app with sign-in (scripts/apps-demo/server.mjs runs it).
const app = await api('POST', `${ws}/workspace-apps`, { name: 'Products', url: appUrl, redirect_uris: [`${appUrl}/auth/callback`] })
writeFileSync(`${out}/demo.env`, `CLIENT_ID=${app.sign_in.client_id}\nCLIENT_SECRET=${app.client_secret}\n`, { mode: 0o600 })

writeFileSync(`${out}/seed.json`, JSON.stringify({ workspaceId, longPage, pdfPage, sheetPage, diagramPage, channel: channel.result.id, app: app.slug }, null, 2))
console.log('seed done')
