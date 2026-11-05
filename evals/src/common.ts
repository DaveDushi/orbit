// Shared helpers of the eval scripts. Every script reads the test instance from $EVALS_WORK/instance.json and writes
// one result part to $EVALS_WORK/parts/<name>.json. Secrets (passwords, cookies, tokens) stay in $EVALS_WORK, which
// run.sh deletes; result parts never contain them.
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

export const work = process.env.EVALS_WORK ?? ''
if (!work) throw new Error('EVALS_WORK is not set: run the evals through evals/run.sh')
export const quick = process.env.EVALS_QUICK === '1'

export type Instance = {
  base: string // http://127.0.0.1:<port>, what the client connects to
  origin: string // the public origin the server expects in Origin
  mode: 'development' | 'production'
  cookie: string // session cookie name
  pid: number
  dataDir: string
}

export type Seed = {
  password: string
  a: Account
  b: Account
}

export type Account = {
  email: string
  userId: string
  workspaceId: string
  pageId: string
  fileId: string
  projectId: string
  statusId: string
  taskId: string
  readToken: string
  writeToken: string
}

export const readJson = <T>(name: string): T => JSON.parse(readFileSync(join(work, name), 'utf8')) as T
export const writeJson = (name: string, value: unknown) => writeFileSync(join(work, name), `${JSON.stringify(value, null, 2)}\n`)
export const writePart = (name: string, value: unknown) => {
  mkdirSync(join(work, 'parts'), { recursive: true })
  writeJson(join('parts', `${name}.json`), value)
}

export const instance = (): Instance => readJson<Instance>('instance.json')

export type Reply = { status: number; headers: Headers; text: string; json: any }

/** One request to the test instance. In production mode it acts as the trusted proxy (X-Forwarded-Proto: https). */
export async function call(
  method: string,
  path: string,
  options: {
    session?: string
    bearer?: string
    origin?: string | null
    body?: unknown
    form?: FormData
    headers?: Record<string, string>
    /** The client address the trusted proxy reports (production mode), so each check has its own rate limit bucket. */
    ip?: string
    timeout?: number
  } = {},
): Promise<Reply> {
  const server = instance()
  const headers: Record<string, string> = { 'x-orbit-contract': 'orbit-api-v1', ...options.headers }
  if (server.mode === 'production') {
    headers['x-forwarded-proto'] = 'https'
    if (options.ip) headers['x-forwarded-for'] = options.ip
  }
  // Browsers send Origin on every write; `origin: null` leaves it out on purpose.
  const origin = options.origin === undefined ? server.origin : options.origin
  if (origin !== null && !['GET', 'HEAD'].includes(method)) headers.origin = origin
  if (options.session) headers.cookie = `${server.cookie}=${options.session}`
  if (options.bearer) headers.authorization = `Bearer ${options.bearer}`
  let body: BodyInit | undefined
  if (options.form) body = options.form
  else if (options.body !== undefined) {
    headers['content-type'] = 'application/json'
    body = JSON.stringify(options.body)
  }
  const response = await fetch(server.base + path, { method, headers, body, redirect: 'manual', signal: AbortSignal.timeout(options.timeout ?? 30_000) })
  const text = await response.text()
  let json: any = null
  try {
    json = JSON.parse(text)
  } catch {}
  return { status: response.status, headers: response.headers, text, json }
}

/** The session cookie value from a login response. */
export function sessionFrom(reply: Reply): string {
  const name = instance().cookie
  for (const cookie of reply.headers.getSetCookie()) {
    const [pair] = cookie.split(';')
    const [key, value] = pair.split('=')
    if (key.trim() === name && value) return value
  }
  throw new Error(`login returned ${reply.status} without a session cookie`)
}

export async function login(email: string, password: string): Promise<string> {
  const reply = await call('POST', '/api/v1/auth/login', { body: { email, password } })
  if (reply.status !== 200) throw new Error(`login failed with ${reply.status}`)
  return sessionFrom(reply)
}

export function expectStatus(reply: Reply, expected: number[], what: string) {
  if (!expected.includes(reply.status)) throw new Error(`${what}: ${reply.status} ${reply.text.slice(0, 300)}`)
  return reply
}

export const median = (values: number[]) => {
  const sorted = [...values].sort((a, b) => a - b)
  const middle = Math.floor(sorted.length / 2)
  return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2
}

export const round = (value: number, digits = 1) => Math.round(value * 10 ** digits) / 10 ** digits
