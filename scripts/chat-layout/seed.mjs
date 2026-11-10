#!/usr/bin/env node
// Seeds a chat channel for the layout check of chat on a phone (scripts/chat-layout/shots.mjs): every agent card
// component and stress cases (long words, long numbers, many columns, charts with long labels) in one thread, and in
// the channel Markdown tables (open orders, long @mentions) and the agent's tool lines.
// Needs the development seed (`orbit migrate run --seed`) and a server with ORBIT_AGENT_URL and ORBIT_AGENT_SECRET set:
// the cards come from the sidecar's route, as the agent's do.
//
//   ORBIT_AGENT_SECRET=… node scripts/chat-layout/seed.mjs <orbit url> <email> <password> <output directory>
// Writes <output>/seed.json (workspace, channel and thread ids for shots.mjs).
import { randomUUID } from 'node:crypto'
import { writeFileSync } from 'node:fs'
import { groupSteps } from '../../apps/agent/src/steps.ts'

const [BASE, email, password, out] = process.argv.slice(2)
const secret = process.env.ORBIT_AGENT_SECRET
let cookie = ''

async function call(method, path, body, headers) {
  const response = await fetch(`${BASE}${path}`, { method, headers: { 'content-type': 'application/json', ...headers }, body: body === undefined ? undefined : JSON.stringify(body) })
  const text = await response.text()
  if (!response.ok) throw new Error(`${method} ${path} → ${response.status}: ${text}`)
  return { response, data: text ? JSON.parse(text) : undefined }
}
const api = async (method, path, body) => {
  const { response, data } = await call(method, path, body, { origin: BASE, 'x-orbit-contract': 'orbit-api-v1', ...(cookie ? { cookie } : {}) })
  const set = response.headers.getSetCookie?.() ?? []
  if (set.length) cookie = set.map((line) => line.split(';')[0]).join('; ')
  return data
}
const agent = async (method, path, body) => (await call(method, path, body, { authorization: `Bearer ${secret}` })).data
const items = (list) => (Array.isArray(list) ? list : list.items)

await api('POST', '/api/v1/auth/login', { email, password })
const me = await api('PATCH', '/api/v1/auth/me', { display_name: 'Bartholomew Featherstonehaugh' })
const workspaceId = items(await api('GET', '/api/v1/workspaces'))[0].id
const ws = `/api/v1/workspaces/${workspaceId}`
const { people } = await agent('GET', `/api/internal/agent/workspaces/${workspaceId}/people`)
// a long name for the @mention cases: the other member of the development seed
const other = people.find((person) => person.id !== me.id && person.name !== 'Orbit Agent') ?? people[0]
// chat writes answer { result, events, seq }
const channel = (await api('POST', `${ws}/chat/conversations`, { name: `layout-${randomUUID().slice(0, 6)}`, kind: 'public' })).result
const say = async (body, thread) => (await api('POST', `${ws}/chat/conversations/${channel.id}/messages`, { body, nonce: randomUUID(), ...(thread ? { thread_root_id: thread } : {}) })).result

// A card from the agent: a message as the bot, then the edit that adds its card (as the sidecar does).
let cards = 0
async function card(root, text, title, elements, rootId = 'card') {
  const posted = await agent('POST', '/api/internal/agent/messages', { workspace_id: workspaceId, conversation_id: channel.id, root_message_id: root, body: text, nonce: randomUUID() })
  const messageId = posted.id
  const spec = { root: rootId, elements: Object.fromEntries(Object.entries(elements).map(([id, element]) => [id, { children: [], props: {}, ...element }])) }
  await agent('PATCH', `/api/internal/agent/messages/${messageId}`, { workspace_id: workspaceId, body: text, blocks: [{ type: 'ui', version: 1, title, block_id: `b${++cards}`, spec }] })
}

// The channel: Markdown tables and tool lines.
await say(
  'Open orders this week:\n\n| Order | Customer | Items | Total |\n| --- | --- | --- | ---: |\n' +
    '| #SO-2026-004512 | Nordwind Handelsgesellschaft mbH | 3 × Bluetooth speaker, 1 × charging cable | 1,249.90 € |\n' +
    '| #SO-2026-004513 | Ada Lovelace | 1 × Wireless keyboard | 89.00 € |\n' +
    '| #SO-2026-004514 | Schneider & Söhne Elektrotechnik | 12 × LED panel 60×60, 4 × driver | 3,480.00 € |\n' +
    '| #SO-2026-004515 | Li Wei | 2 × USB-C hub | 118.50 € |',
)
await say(
  `Who does what:\n\n| Owner | Task | Status |\n| --- | --- | --- |\n` +
    `| <@${other.id}> | Update the supplier contract for the Hamburg warehouse | In review |\n` +
    `| <@${me.id}> | Check invoices INV-2026-0000012345 and INV-2026-0000012346 | Blocked |\n\n` +
    'A long link still wraps: https://example.com/orders/2026/11/very-long-path-that-does-not-fit-on-a-phone-screen?with=query&and=more',
)
const steps = ['Using shopify → api request', 'Using shopify → api request', 'Using shopify → api request', 'Searching tasks for “invoice”', 'Using shopify → api request']
await agent('POST', '/api/internal/agent/messages', {
  workspace_id: workspaceId,
  conversation_id: channel.id,
  root_message_id: null,
  body: `⏳ Working on it…\n\n${groupSteps(steps).map((step) => `• ${step}`).join('\n')}`,
  nonce: randomUUID(),
})
await agent('POST', '/api/internal/agent/messages', {
  workspace_id: workspaceId,
  conversation_id: channel.id,
  root_message_id: null,
  body: `Four orders are open; the biggest is #SO-2026-004514.\n\n_${groupSteps(steps).join(' · ')}_`,
  nonce: randomUUID(),
})
for (let n = 1; n <= 12; n++) await say(`Filler message ${n} so the list scrolls and the day chip floats over the text of the messages below it. ${'Some more words to wrap. '.repeat(n % 3 + 1)}`)

// The thread: every component, then the stress cases.
const thread = (await say('@agent show me every card component')).id
const long = 'Donaudampfschifffahrtsgesellschaftskapitänsmützenabzeichen'
await card(thread, 'Here is every component.', 'All components', {
  card: { type: 'Card', props: { title: 'All components', description: 'A card with each component of the catalog.' }, children: ['text', 'row', 'table', 'chart', 'tasks', 'form'] },
  text: { type: 'Text', props: { text: `**Bold**, a [link](https://example.com), a mention <@${other.id}> and a task id.`, tone: 'default' } },
  row: { type: 'Stack', props: { direction: 'row' }, children: ['b1', 'b2', 'b3'] },
  b1: { type: 'Button', props: { label: 'Approve', style: 'primary', action: 'reply' } },
  b2: { type: 'Button', props: { label: 'Later', style: 'secondary', action: 'reply' } },
  b3: { type: 'Button', props: { label: 'Reject', style: 'danger', action: 'reply' } },
  table: { type: 'Table', props: { columns: [{ key: 'name', label: 'Name' }, { key: 'qty', label: 'Qty' }, { key: 'ok', label: 'In stock' }], rows: [{ name: 'Speaker', qty: 3, ok: true }, { name: 'Cable', qty: 120, ok: false }] } },
  chart: { type: 'Chart', props: { kind: 'bar', title: 'Orders per day', series: [{ name: 'Orders', points: ['Mon', 'Tue', 'Wed', 'Thu', 'Fri'].map((x, i) => ({ x, y: [12, 18, 9, 22, 15][i] })) }] } },
  tasks: { type: 'TaskList', props: { tasks: [{ id: 'GEN-1', title: 'Write the release notes', status: 'Todo', assignee: 'Ada' }, { id: 'GEN-2', title: 'Fix the import', status: 'In progress', due: 'Nov 12' }] } },
  form: {
    type: 'Form',
    props: {
      fields: [
        { name: 'note', label: 'Note', kind: 'text', required: true },
        { name: 'details', label: 'Details', kind: 'textarea' },
        { name: 'size', label: 'Size', kind: 'select', options: ['Small', 'Medium', 'Large'] },
        { name: 'urgent', label: 'Urgent', kind: 'checkbox' },
        { name: 'due', label: 'Due', kind: 'date' },
      ],
      submit_label: 'Send',
    },
  },
})
await card(thread, 'Long words and many buttons.', 'Long words', {
  card: { type: 'Card', props: { title: `Title with a long word ${long}`, description: `https://example.com/a/really/long/link/${long.toLowerCase()}/that/goes/on` }, children: ['text', 'row'] },
  text: { type: 'Text', props: { text: `A long word: ${long}${long}. A long number: 12345678901234567890123.`, tone: 'warning' } },
  row: { type: 'Stack', props: { direction: 'row' }, children: ['b1', 'b2', 'b3', 'b4', 'b5', 'b6'] },
  ...Object.fromEntries(['Approve the whole order now', 'Ask the customer about the delivery date', 'Split', 'Cancel the order and refund it', long, 'OK'].map((label, i) => [`b${i + 1}`, { type: 'Button', props: { label, style: i === 0 ? 'primary' : i === 3 ? 'danger' : 'secondary', action: 'reply' } }])),
})
await card(thread, 'A wide table.', 'Wide table', {
  card: { type: 'Card', props: { title: 'Eight columns' }, children: ['table'] },
  table: {
    type: 'Table',
    props: {
      columns: ['Order', 'Customer', 'SKU', 'Quantity', 'Unit price', 'Total', 'Warehouse', 'Note'].map((label, i) => ({ key: `c${i}`, label, ...(i >= 3 && i <= 5 ? { align: 'right' } : {}) })),
      rows: [
        { c0: '#SO-2026-004512', c1: 'Nordwind Handelsgesellschaft mbH', c2: 'BT-SPK-0001-BLK-EU', c3: 1234567, c4: 12345.678, c5: 15234567890.12, c6: 'Hamburg', c7: long },
        { c0: '#SO-2026-004513', c1: 'Ada', c2: 'KB-01', c3: 1, c4: 89, c5: 89, c6: 'Berlin', c7: 'Gift wrap' },
        { c0: 'GEN-1', c1: 'Schneider & Söhne', c2: 'LED-60x60', c3: 12, c4: 290, c5: 3480, c6: 'München', c7: null },
      ],
    },
  },
})
await card(thread, 'Charts with long labels.', 'Charts', {
  card: { type: 'Card', props: { title: 'Revenue by customer' }, children: ['bar', 'line'] },
  bar: {
    type: 'Chart',
    props: {
      kind: 'bar',
      title: 'Revenue by customer (EUR)',
      x_label: 'Customer (the twelve largest this quarter)',
      y_label: 'Revenue in euros',
      series: [{ name: 'Revenue', points: ['Nordwind Handelsgesellschaft', 'Schneider & Söhne Elektrotechnik', 'Ada Lovelace', 'Li Wei', 'Acme Corporation International', 'Globex', 'Initech', 'Umbrella', 'Hooli', 'Vandelay Industries', 'Stark', 'Wayne Enterprises'].map((x, i) => ({ x, y: 12345678 - i * 812345 })) }],
    },
  },
  line: {
    type: 'Chart',
    props: {
      kind: 'line',
      title: 'Orders per week',
      series: ['Online shop (Shopify, all regions)', 'Wholesale partners in the DACH region', 'Marketplace'].map((name, s) => ({ name, points: Array.from({ length: 20 }, (_, i) => ({ x: `Week ${i + 1}`, y: Math.round(1000 + s * 400 + Math.sin(i / 2 + s) * 300) })) })),
    },
  },
})
await card(thread, 'Tasks and a form with long text.', 'Long tasks and form', {
  card: { type: 'Card', props: { title: 'Follow-ups' }, children: ['tasks', 'form'] },
  tasks: {
    type: 'TaskList',
    props: {
      tasks: [
        { id: 'GEN-1', title: `Call the customer about ${long} and the delivery of the remaining 12 LED panels`, status: 'In progress', assignee: 'Bartholomew Featherstonehaugh', due: 'Friday, November 13, 2026' },
        { id: 'GEN-123456', title: 'Short', status: 'Todo' },
      ],
    },
  },
  form: {
    type: 'Form',
    props: {
      fields: [
        { name: 'reason', label: `Why split the order for ${long}?`, kind: 'textarea', required: true },
        { name: 'carrier', label: 'Carrier', kind: 'select', options: ['DHL Express Worldwide (next business day before 12:00)', 'UPS', long] },
        { name: 'notify', label: 'Tell the customer by e-mail and by text message after the change', kind: 'checkbox' },
      ],
      submit_label: 'Send the change',
    },
  },
})

writeFileSync(`${out}/seed.json`, JSON.stringify({ workspaceId, channelId: channel.id, threadId: thread }, null, 2))
console.log(`seeded ${BASE}/chat/${channel.id} (thread ${thread})`)
