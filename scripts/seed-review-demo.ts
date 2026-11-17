#!/usr/bin/env bun
// Creates (or completes) a small, invented demo workspace for App Review through the HTTP API:
// 2 projects with 15 tasks (statuses, due dates, comments), 3 docs, a few Drive files and 2 chat channels.
// Safe to run again: the workspace and everything in it are matched by name/title and skipped when they exist.
// It never invites anyone, never assigns or mentions the agent (that would start a paid model run).
//
//   ORBIT_TOKEN_FILE=/path/to/session-token bun scripts/seed-review-demo.ts
//   ORBIT_URL=http://127.0.0.1:8000 ORBIT_HOST=orbit.easytomanage.com ...   production through the local nginx
//
// The token is a session token of the user who becomes the workspace owner, sent as a bearer token.

import { deflateSync } from 'node:zlib'

const BASE = process.env.ORBIT_URL ?? 'http://127.0.0.1:8000'
const HOST = process.env.ORBIT_HOST
const WORKSPACE = process.env.ORBIT_WORKSPACE ?? 'Orbit Demo'
const TOKEN = (await Bun.file(process.env.ORBIT_TOKEN_FILE ?? '').text()).trim()
if (!TOKEN) throw new Error('set ORBIT_TOKEN_FILE to a file with a session token')

async function api<T>(method: string, path: string, body?: unknown): Promise<T> {
  const headers: Record<string, string> = { authorization: `Bearer ${TOKEN}`, 'x-orbit-contract': 'orbit-api-v1' }
  if (HOST) Object.assign(headers, { host: HOST, 'x-forwarded-proto': 'https' })
  if (body !== undefined) headers['content-type'] = 'application/json'
  const response = await fetch(`${BASE}${path}`, { method, headers, body: body === undefined ? undefined : JSON.stringify(body) })
  if (!response.ok) throw new Error(`${method} ${path} → ${response.status}: ${await response.text()}`)
  return (response.status === 204 ? undefined : await response.json()) as T
}

async function all<T>(path: string): Promise<T[]> {
  const items: T[] = []
  let cursor: string | null | undefined
  do {
    const separator = path.includes('?') ? '&' : '?'
    const page = await api<{ items: T[]; next_cursor?: string | null }>('GET', `${path}${separator}limit=100${cursor ? `&cursor=${encodeURIComponent(cursor)}` : ''}`)
    items.push(...page.items)
    cursor = page.next_cursor
  } while (cursor)
  return items
}

const day = (offset: number) => {
  const date = new Date()
  date.setUTCHours(17, 0, 0, 0)
  date.setUTCDate(date.getUTCDate() + offset)
  return date.toISOString()
}

// ---------- data ----------
const PROJECTS = [
  { name: 'Website Relaunch', key: 'WEB', color: '#5e6ad2' },
  { name: 'Mobile App Launch', key: 'APP', color: '#e5488a' },
]
const LABELS = [
  { name: 'Design', color: '#bb87fc' },
  { name: 'Bug', color: '#eb5757' },
  { name: 'Marketing', color: '#f2994a' },
  { name: 'Content', color: '#26b5ce' },
]

type DemoTask = { project: string; title: string; status: string; priority: string; labels?: string[]; mine?: boolean; due?: number; description?: string; comments?: string[] }
const TASKS: DemoTask[] = [
  { project: 'Website Relaunch', title: 'Write new homepage copy', status: 'Done', priority: 'high', labels: ['Content'], mine: true, due: -6,
    description: 'Short headline, three benefit blocks and one clear call to action.', comments: ['Final copy is in the "Homepage copy" doc. Approved by marketing.'] },
  { project: 'Website Relaunch', title: 'Choose a new color palette', status: 'Done', priority: 'medium', labels: ['Design'], due: -10 },
  { project: 'Website Relaunch', title: 'Design the pricing page', status: 'In Progress', priority: 'high', labels: ['Design'], mine: true, due: 3,
    description: 'Three plans side by side, yearly/monthly switch, FAQ below.\n\n- [x] Wireframe\n- [ ] High-fidelity mockup\n- [ ] Review with sales',
    comments: ['Wireframe is done, starting the mockup today.', 'Sales asked for a "Contact us" option on the largest plan.'] },
  { project: 'Website Relaunch', title: 'Set up redirects from the old URLs', status: 'In Progress', priority: 'medium', mine: true, due: 5 },
  { project: 'Website Relaunch', title: 'Contact form does not send on Safari', status: 'Todo', priority: 'urgent', labels: ['Bug'], mine: true, due: 1,
    description: 'Steps: open /contact in Safari 18, fill in all fields, press Send. Nothing happens and no error is shown.' },
  { project: 'Website Relaunch', title: 'Add customer logos section', status: 'Todo', priority: 'low', labels: ['Design', 'Marketing'], due: 12 },
  { project: 'Website Relaunch', title: 'Compress hero images', status: 'Todo', priority: 'medium', due: 8 },
  { project: 'Website Relaunch', title: 'Write three blog posts for launch week', status: 'Backlog', priority: 'low', labels: ['Content', 'Marketing'] },
  { project: 'Mobile App Launch', title: 'Prepare App Store screenshots', status: 'Done', priority: 'high', labels: ['Design', 'Marketing'], mine: true, due: -3,
    comments: ['Exported for 6.9" and 6.5" displays. Files are in Drive → Launch assets.'] },
  { project: 'Mobile App Launch', title: 'Write App Store description', status: 'Done', priority: 'medium', labels: ['Content'], due: -2 },
  { project: 'Mobile App Launch', title: 'Beta test with 20 customers', status: 'In Progress', priority: 'high', mine: true, due: 7,
    description: 'Collect feedback in #launch. Focus on sign-in, notifications and offline mode.', comments: ['14 of 20 testers have installed the beta so far.'] },
  { project: 'Mobile App Launch', title: 'Push notifications arrive twice', status: 'In Progress', priority: 'urgent', labels: ['Bug'], mine: true, due: 2 },
  { project: 'Mobile App Launch', title: 'Plan the launch announcement', status: 'Todo', priority: 'medium', labels: ['Marketing'], due: 10,
    description: 'Email to customers, social posts and a short video. See the "Launch plan" doc.' },
  { project: 'Mobile App Launch', title: 'Update the help center articles', status: 'Todo', priority: 'low', labels: ['Content'], due: 14 },
  { project: 'Mobile App Launch', title: 'Dark mode for settings screen', status: 'Backlog', priority: 'low', labels: ['Design'] },
]

type Block = Record<string, unknown>
const text = (value: string) => [{ type: 'text', text: value, styles: {} }]
const block = (type: string, value: string, props: Record<string, unknown> = {}): Block => ({ type, props, content: text(value), children: [] })
const p = (value: string) => block('paragraph', value)
const h2 = (value: string) => block('heading', value, { level: 2 })
const ul = (value: string) => block('bulletListItem', value)
const todo = (value: string, checked = false) => block('checkListItem', value, { checked })

const DOCS: { title: string; icon: string; content: Block[] }[] = [
  { title: 'Welcome to Orbit Demo', icon: '👋', content: [
    p('This workspace is a sample team that builds a website and a mobile app. All names and data are invented.'),
    h2('Where to find things'),
    ul('Projects: tasks for the website relaunch and the mobile app launch.'),
    ul('Docs: plans and meeting notes, like this page.'),
    ul('Drive: launch assets and the team handbook.'),
    ul('Chat: #general for the whole team, #launch for launch work.'),
  ] },
  { title: 'Launch plan', icon: '🚀', content: [
    p('Goal: release the mobile app and the new website in the same week.'),
    h2('Timeline'),
    ul('Week 1: finish beta test and fix the top bugs.'),
    ul('Week 2: submit to the App Store, publish the help center articles.'),
    ul('Week 3: launch announcement by email and social media.'),
    h2('Checklist'),
    todo('App Store screenshots', true),
    todo('App Store description', true),
    todo('Pricing page live'),
    todo('Announcement email scheduled'),
  ] },
  { title: 'Weekly sync – notes', icon: '📝', content: [
    p('Attendees: product, design, engineering, marketing.'),
    h2('Decisions'),
    ul('The pricing page shows three plans. The largest plan gets a "Contact us" button.'),
    ul('Beta testers get the app one week before the public launch.'),
    h2('Action items'),
    todo('Fix the duplicate push notifications before the next beta build'),
    todo('Share the screenshot set with marketing', true),
  ] },
]

const CHANNELS: { name: string; topic: string; messages: string[] }[] = [
  { name: 'general', topic: 'Team-wide news and questions', messages: [
    'Good morning everyone! Reminder: weekly sync is at 10:00 today.',
    'The notes from last week are in Docs → "Weekly sync – notes".',
    'Welcome to the new workspace. Projects, docs and files for the launch are all here now.',
  ] },
  { name: 'launch', topic: 'Website relaunch and mobile app launch', messages: [
    'Beta update: 14 of 20 testers have installed the app.',
    'Top feedback so far: sign-in is fast, but some testers get push notifications twice. Task is in progress.',
    'App Store screenshots are ready in Drive → Launch assets.',
    'Target: submit the app for review at the end of next week. 🚀',
  ] },
]

// ---------- files ----------
function crc32(bytes: Uint8Array) {
  let crc = ~0
  for (const byte of bytes) {
    crc ^= byte
    for (let k = 0; k < 8; k++) crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1))
  }
  return ~crc >>> 0
}
/** A 640×360 PNG: a soft gradient with a few bars, like a small chart. */
function bannerPng(): Uint8Array<ArrayBuffer> {
  const width = 640, height = 360
  const raw = new Uint8Array((width * 3 + 1) * height)
  const bars = [[80, 200], [200, 140], [320, 90], [440, 60]]
  for (let y = 0; y < height; y++) {
    raw[y * (width * 3 + 1)] = 0
    for (let x = 0; x < width; x++) {
      const t = x / width
      let rgb = [Math.round(94 + 135 * t), Math.round(106 - 34 * t), Math.round(210 - 72 * t)]
      if (bars.some(([left, top]) => x >= left && x < left + 80 && y >= top && y < 320)) rgb = [255, 255, 255]
      raw.set(rgb, y * (width * 3 + 1) + 1 + x * 3)
    }
  }
  const chunk = (type: string, data: Uint8Array) => {
    const out = new Uint8Array(12 + data.length)
    const view = new DataView(out.buffer)
    view.setUint32(0, data.length)
    out.set(new TextEncoder().encode(type), 4)
    out.set(data, 8)
    view.setUint32(8 + data.length, crc32(out.subarray(4, 8 + data.length)))
    return out
  }
  const header = new Uint8Array(13)
  const view = new DataView(header.buffer)
  view.setUint32(0, width)
  view.setUint32(4, height)
  header.set([8, 2, 0, 0, 0], 8)
  const parts = [new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', header), chunk('IDAT', deflateSync(raw)), chunk('IEND', new Uint8Array())]
  return new Uint8Array(Buffer.concat(parts))
}

const md = (value: string) => new TextEncoder().encode(value) as Uint8Array<ArrayBuffer>
const FILES: { folder: string | null; name: string; mime: string; bytes: Uint8Array<ArrayBuffer> }[] = [
  { folder: 'Launch assets', name: 'launch-checklist.md', mime: 'text/markdown',
    bytes: md('# Launch checklist\n\n- [x] App Store screenshots\n- [x] App Store description\n- [ ] Pricing page live\n- [ ] Announcement email scheduled\n- [ ] Help center articles updated\n') },
  { folder: 'Launch assets', name: 'press-release-draft.txt', mime: 'text/plain',
    bytes: md('DRAFT – not for publication\n\nBrightside launches its new mobile app\n\nThe new app lets teams plan work, share files and chat in one place, on the go.\nIt is available on the App Store from launch day.\n') },
  { folder: 'Launch assets', name: 'launch-banner.png', mime: 'image/png', bytes: bannerPng() },
  { folder: null, name: 'Team handbook.md', mime: 'text/markdown',
    bytes: md('# Team handbook\n\n## How we work\n\n- Plan work as tasks in Projects.\n- Write decisions down in Docs.\n- Keep files in Drive, not in chat.\n\n## Meetings\n\nWeekly sync on Monday at 10:00.\n') },
]

// ---------- run ----------
const me = await api<{ id: string; email: string }>('GET', '/api/v1/auth/me')
console.log(`as: ${me.email}`)
const workspaces = await api<{ id: string; name: string }[]>('GET', '/api/v1/workspaces')
let workspace = workspaces.find((item) => item.name === WORKSPACE)
if (!workspace) {
  workspace = await api<{ id: string; name: string }>('POST', '/api/v1/workspaces', { name: WORKSPACE })
  console.log(`+ workspace ${WORKSPACE}`)
}
const ws = `/api/v1/workspaces/${workspace.id}`
console.log(`workspace: ${workspace.id}`)

const projects = new Map((await all<{ id: string; name: string }>(`${ws}/projects`)).map((item) => [item.name, item.id]))
for (const project of PROJECTS) {
  if (projects.has(project.name)) continue
  projects.set(project.name, (await api<{ id: string }>('POST', `${ws}/projects`, project)).id)
  console.log(`+ project ${project.name}`)
}

// Labels belong to a project.
const labels = new Map<string, Map<string, string>>()
const statuses = new Map<string, Map<string, string>>()
for (const { name } of PROJECTS) {
  const id = projects.get(name)!
  const own = new Map((await all<{ id: string; name: string }>(`${ws}/projects/${id}/labels`)).map((item) => [item.name, item.id]))
  for (const label of LABELS) {
    if (own.has(label.name)) continue
    own.set(label.name, (await api<{ id: string }>('POST', `${ws}/projects/${id}/labels`, label)).id)
    console.log(`+ label ${name}/${label.name}`)
  }
  labels.set(name, own)
  const list = await all<{ id: string; name: string }>(`${ws}/projects/${id}/statuses`)
  statuses.set(name, new Map(list.map((item) => [item.name, item.id])))
}

const existingTasks = new Set((await all<{ title: string }>(`${ws}/tasks`)).map((item) => item.title))
let tasks = 0
for (const task of TASKS) {
  if (existingTasks.has(task.title)) continue
  const status = statuses.get(task.project)!.get(task.status)
  if (!status) throw new Error(`no status "${task.status}" in ${task.project}`)
  const record = await api<{ id: string }>('POST', `${ws}/tasks`, {
    project_id: projects.get(task.project),
    status_id: status,
    title: task.title,
    description: task.description ?? '',
    priority: task.priority,
    label_ids: (task.labels ?? []).map((name) => labels.get(task.project)!.get(name)),
    assignee_ids: task.mine ? [me.id] : [],
    due_at: task.due === undefined ? null : day(task.due),
  })
  for (const body of task.comments ?? []) await api('POST', `${ws}/tasks/${record.id}/comments`, { body })
  tasks++
  console.log(`+ task ${task.title}`)
}

const existingPages = new Set((await api<{ items: { title: string }[] }>('GET', `${ws}/pages`)).items.map((item) => item.title))
for (const doc of DOCS) {
  if (existingPages.has(doc.title)) continue
  const created = await api<{ id: string }>('POST', `${ws}/pages`, { title: doc.title, icon: doc.icon })
  const page = await api<{ version: number }>('GET', `${ws}/pages/${created.id}`)
  await api('PATCH', `${ws}/pages/${created.id}`, { expected_version: page.version, content: doc.content })
  console.log(`+ doc ${doc.title}`)
}

type Node = { id: string; name: string; kind?: string }
const nodes = async (parent: string | null) =>
  all<Node>(`${ws}/drive/nodes${parent ? `?parent_id=${parent}` : ''}`)
const folders = new Map<string | null, string | null>([[null, null]])
for (const file of FILES) {
  if (folders.has(file.folder)) continue
  const found = (await nodes(null)).find((node) => node.name === file.folder)
  folders.set(file.folder, found?.id ?? (await api<{ id: string }>('POST', `${ws}/drive/folders`, { name: file.folder })).id)
}
for (const file of FILES) {
  const parent = folders.get(file.folder) ?? null
  if ((await nodes(parent)).some((node) => node.name === file.name)) continue
  const started = await api<{ upload: { id: string }; url?: string | null }>('POST', `${ws}/drive/uploads`, {
    name: file.name, size: file.bytes.length, mime: file.mime, parent_id: parent,
  })
  if (!started.url) throw new Error(`${file.name}: expected a single-PUT upload`)
  const put = await fetch(started.url, { method: 'PUT', body: file.bytes, headers: { 'content-type': file.mime } })
  if (!put.ok) throw new Error(`${file.name}: storage answered ${put.status}: ${await put.text()}`)
  await api('POST', `${ws}/drive/uploads/${started.upload.id}/complete`)
  console.log(`+ file ${file.folder ? `${file.folder}/` : ''}${file.name}`)
}

const conversations = await api<{ id: string; name?: string | null; kind: string }[]>('GET', `${ws}/chat/conversations`)
for (const channel of CHANNELS) {
  const conversation = conversations.find((item) => item.name === channel.name)
    ?? (await api<{ result: { id: string } }>('POST', `${ws}/chat/conversations`, { name: channel.name, kind: 'public', topic: channel.topic })).result
  // A send with a nonce that was used before returns the first message, so a second run adds nothing.
  for (const [index, body] of channel.messages.entries()) {
    await api('POST', `${ws}/chat/conversations/${conversation.id}/messages`, { body, nonce: `seed-${channel.name}-${index}` })
  }
  console.log(`channel #${channel.name}: ${channel.messages.length} messages (a repeated nonce adds nothing)`)
}

console.log(`done: workspace ${workspace.id}, ${tasks} new task(s)`)
