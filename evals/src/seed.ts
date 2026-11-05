// Seeds the test instance through its public API: user A owns workspace A, user B owns workspace B, and B is not a
// member of A. Each workspace gets a doc with text, a text file on that doc, a task, and two MCP API tokens: `read`
// only, and all scopes (read, write, docs:read, tasks:write).
// Workspace A also gets extra docs and tasks so lists are not trivially small. Writes $EVALS_WORK/seed.json.
// Usage: bun evals/src/seed.ts <setup token>
import { randomBytes } from 'node:crypto'
import { call, expectStatus, login, quick, sessionFrom, writeJson, type Account, type Seed } from './common'

const setupToken = process.argv[2]
if (!setupToken) throw new Error('usage: seed.ts <setup token>')
const password = `Ev-${randomBytes(18).toString('base64url')}`
const emailA = 'eval-a@example.test'
const emailB = 'eval-b@example.test'

const paragraph = (text: string) => ({ type: 'paragraph', content: [{ type: 'text', text, styles: {} }] })

async function fill(session: string, name: string, extra: number): Promise<Omit<Account, 'email' | 'userId'>> {
  const workspace = expectStatus(await call('POST', '/api/v1/workspaces', { session, body: { name } }), [201], 'create workspace').json
  const ws = `/api/v1/workspaces/${workspace.id}`
  let projects = expectStatus(await call('GET', `${ws}/projects`, { session }), [200], 'list projects').json
  projects = projects.items ?? projects
  const project = projects[0] ?? expectStatus(await call('POST', `${ws}/projects`, { session, body: { name: 'Eval' } }), [201], 'create project').json
  let statuses = expectStatus(await call('GET', `${ws}/projects/${project.id}/statuses`, { session }), [200], 'list statuses').json
  statuses = statuses.items ?? statuses

  const page = expectStatus(await call('POST', `${ws}/pages`, { session, body: { title: `${name} doc` } }), [201], 'create page').json
  const blocks = Array.from({ length: 40 }, (_, index) => paragraph(`${name} paragraph ${index}: searchable eval text about orbit performance.`))
  expectStatus(await call('PATCH', `${ws}/pages/${page.id}`, { session, body: { expected_version: page.version, content: blocks } }), [200], 'write page')

  const form = new FormData()
  form.append('file', new File([`${name} file\n`.repeat(200)], 'notes.txt', { type: 'text/plain' }))
  const file = expectStatus(await call('POST', `${ws}/pages/${page.id}/files`, { session, form }), [201], 'upload file').json

  const task = expectStatus(
    await call('POST', `${ws}/tasks`, { session, body: { project_id: project.id, status_id: statuses[0].id, title: `${name} task` } }),
    [201],
    'create task',
  ).json
  for (let index = 0; index < extra; index++) {
    expectStatus(await call('POST', `${ws}/pages`, { session, body: { title: `${name} doc ${index}` } }), [201], 'create page')
    expectStatus(
      await call('POST', `${ws}/tasks`, { session, body: { project_id: project.id, status_id: statuses[0].id, title: `${name} task ${index}` } }),
      [201],
      'create task',
    )
  }
  const token = async (scopes: string[]) =>
    expectStatus(await call('POST', `${ws}/api-tokens`, { session, body: { name: `eval ${scopes.join(' ')}`, scopes, project_ids: [project.id] } }), [201], 'create token').json.token as string
  return {
    workspaceId: workspace.id,
    pageId: page.id,
    fileId: file.id,
    projectId: project.id,
    statusId: statuses[0].id,
    taskId: task.id,
    readToken: await token(['read']),
    writeToken: await token(['read', 'write', 'docs:read', 'tasks:write']),
  }
}

const setup = expectStatus(
  await call('POST', '/api/v1/setup/complete', { body: { token: setupToken, email: emailA, display_name: 'Eval A', password } }),
  [201],
  'setup',
).json
const sessionA = await login(emailA, password)
const a = { email: emailA, userId: setup.user_id, ...(await fill(sessionA, 'Eval A', quick ? 10 : 50)) }

// B joins through an invitation to A (the only way to make an account without email), then leaves A.
const invitation = expectStatus(
  await call('POST', `/api/v1/workspaces/${a.workspaceId}/invitations`, { session: sessionA, body: { email: emailB, role: 'member', delivery: 'manual' } }),
  [201],
  'invite B',
).json
const invitationToken = new URL(invitation.url).searchParams.get('token')
const accepted = expectStatus(
  await call('POST', '/api/v1/workspaces/invitations/accept', { body: { token: invitationToken, email: emailB, display_name: 'Eval B', password } }),
  [200, 201],
  'accept invitation',
)
const sessionB = accepted.headers.getSetCookie().length ? sessionFrom(accepted) : await login(emailB, password)
const me = expectStatus(await call('GET', '/api/v1/auth/me', { session: sessionB }), [200], 'me B').json
const b = { email: emailB, userId: me.user?.id ?? me.id, ...(await fill(sessionB, 'Eval B', 0)) }
let members = expectStatus(await call('GET', `/api/v1/workspaces/${a.workspaceId}/members`, { session: sessionA }), [200], 'members').json
members = members.items ?? members
const membership = members.find((member: { email: string }) => member.email === emailB)
expectStatus(await call('DELETE', `/api/v1/workspaces/${a.workspaceId}/members/${membership.id}?expected_version=${membership.version}`, { session: sessionA }), [204, 200], 'remove B from A')

writeJson('seed.json', { password, a, b } satisfies Seed)
console.log(`seeded workspaces A and B (${quick ? 10 : 50} extra docs and tasks in A)`)
