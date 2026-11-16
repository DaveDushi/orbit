// Lists every HTTP route of the server from the Rust source: each `.route(path, get(..).post(..))` call under
// apps/server/src. A path can be a string literal, a `let`/`const` string in the same file, or `&format!("{x}/..")`
// built from them. The OpenAPI contract adds nothing new today; it is merged in so a route that the scanner cannot
// read still gets tested. `.nest(prefix, router)` calls in the same file add their prefix. Usage as a module:
// `routes()`; as a script it prints the list.
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join, relative } from 'node:path'

export type Route = { method: string; path: string; file: string }

const root = join(import.meta.dir, '../..')
const methods = ['get', 'post', 'put', 'patch', 'delete']

function rustFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name)
    return statSync(path).isDirectory() ? rustFiles(path) : name.endsWith('.rs') ? [path] : []
  })
}

/** The text of the call's arguments: from after `.route(` to its closing parenthesis. */
function callArguments(source: string, start: number): string {
  let depth = 1
  let index = start
  let inString = false
  for (; index < source.length && depth > 0; index++) {
    const char = source[index]
    if (inString) {
      if (char === '\\') index++
      else if (char === '"') inString = false
    } else if (char === '"') inString = true
    else if (char === '(') depth++
    else if (char === ')') depth--
  }
  return source.slice(start, index - 1)
}

/** Splits at the first comma outside parentheses and strings. */
function firstArgument(args: string): [string, string] {
  let depth = 0
  let inString = false
  for (let index = 0; index < args.length; index++) {
    const char = args[index]
    if (inString) {
      if (char === '\\') index++
      else if (char === '"') inString = false
    } else if (char === '"') inString = true
    else if (char === '(' || char === '[') depth++
    else if (char === ')' || char === ']') depth--
    else if (char === ',' && depth === 0) return [args.slice(0, index).trim(), args.slice(index + 1)]
  }
  return [args.trim(), '']
}

function resolvePath(expression: string, strings: Map<string, string>): string | null {
  const literal = expression.match(/^"([^"]*)"$/)
  if (literal) return literal[1]
  const format = expression.match(/^&?format!\(\s*"([^"]*)"\s*\)$/)
  if (format) {
    let unresolved = false
    const path = format[1].replace(/\{\{|\}\}|\{([a-z_][a-z0-9_]*)\}/gi, (whole: string, name?: string) =>
      name ? (strings.get(name) ?? ((unresolved = true), '')) : whole[0],
    )
    return unresolved ? null : path
  }
  const name = expression.replace(/^&/, '')
  return strings.get(name) ?? null
}

/** The end of the statement that starts at `start`: the first `;` outside brackets and strings. */
function statementEnd(source: string, start: number): number {
  let depth = 0
  let inString = false
  for (let index = start; index < source.length; index++) {
    const char = source[index]
    if (inString) {
      if (char === '\\') index++
      else if (char === '"') inString = false
    } else if (char === '"') inString = true
    else if ('([{'.includes(char)) depth++
    else if (')]}'.includes(char)) depth--
    else if (char === ';' && depth === 0) return index
  }
  return source.length
}

/**
 * The prefix that `.nest(prefix, router)` calls put before the code at `offset`: the `let router = …;` statement
 * that holds it is mounted by a `.nest` in the same file, which can itself sit in another nested router
 * (`public_api.rs`: `BASE_PATH` + `/workspaces/{workspace_id}`).
 */
function nestPrefix(source: string, offset: number, strings: Map<string, string>, seen = new Set<string>()): string | null {
  let owner: { name: string; start: number } | null = null
  for (const match of source.matchAll(/\blet\s+(?:mut\s+)?([a-zA-Z_][a-zA-Z0-9_]*)\s*=/g)) {
    if (match.index! < offset && offset < statementEnd(source, match.index!) && (!owner || match.index! > owner.start)) owner = { name: match[1], start: match.index! }
  }
  if (!owner || seen.has(owner.name)) return ''
  for (const match of source.matchAll(/\.nest\(/g)) {
    const [prefixExpression, router] = firstArgument(callArguments(source, match.index! + match[0].length))
    if (router.trim() !== owner.name) continue
    const outer = nestPrefix(source, match.index!, strings, new Set([...seen, owner.name]))
    const prefix = resolvePath(prefixExpression, strings)
    return outer === null || prefix === null ? null : outer + prefix
  }
  return ''
}

/** The routes of one Rust source file, with the prefixes of the `.nest` calls that mount them. */
export function fileRoutes(source: string, file: string): { routes: Route[]; unresolved: string[] } {
  const found: Route[] = []
  const unresolved: string[] = []
  source = source.replace(/#\[cfg\(test\)\][\s\S]*$/, '')
  const strings = new Map<string, string>()
  for (const match of source.matchAll(/(?:let|const)\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*(?::\s*&(?:'static\s+)?str\s*)?=\s*"([^"]*)"/g)) strings.set(match[1], match[2])
  for (const match of source.matchAll(/\.route(_service)?\(/g)) {
    const [pathExpression, handler] = firstArgument(callArguments(source, match.index! + match[0].length))
    const route = resolvePath(pathExpression, strings)
    const prefix = nestPrefix(source, match.index!, strings)
    if (route === null || prefix === null) {
      unresolved.push(`${file}: ${pathExpression}`)
      continue
    }
    const path = prefix + route
    // A service route (`/mcp`) answers every method; test the ones it serves (POST, GET, DELETE).
    if (match[1]) for (const verb of ['GET', 'POST', 'DELETE']) found.push({ method: verb, path, file })
    const verbs = new Set([...handler.matchAll(/\b(get|post|put|patch|delete|any)(?:_service)?\(/g)].map((m) => m[1]))
    for (const verb of verbs.has('any') ? methods : verbs) found.push({ method: verb.toUpperCase(), path, file })
  }
  return { routes: found, unresolved }
}

export function routes(): Route[] {
  const found = new Map<string, Route>()
  const unresolved: string[] = []
  for (const file of rustFiles(join(root, 'apps/server/src'))) {
    const scanned = fileRoutes(readFileSync(file, 'utf8'), relative(root, file))
    for (const route of scanned.routes) found.set(`${route.method} ${route.path}`, route)
    unresolved.push(...scanned.unresolved)
  }
  const openapi = JSON.parse(readFileSync(join(root, 'packages/api/src/generated/openapi.json'), 'utf8'))
  for (const [path, operations] of Object.entries<Record<string, unknown>>(openapi.paths)) {
    for (const verb of Object.keys(operations).filter((key) => methods.includes(key))) {
      const key = `${verb.toUpperCase()} ${path}`
      if (!found.has(key)) found.set(key, { method: verb.toUpperCase(), path, file: 'openapi.json' })
    }
  }
  if (unresolved.length) console.error(`routes: could not resolve ${unresolved.length} path expressions:\n  ${unresolved.join('\n  ')}`)
  return [...found.values()].sort((x, y) => x.path.localeCompare(y.path) || x.method.localeCompare(y.method))
}

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

/** How the unauthenticated sweep counts the answer of a route to a request without a session. */
export function sweepVerdict(route: Pick<Route, 'method' | 'path'>, status: number): 'rate-limited' | 'error' | 'public' | 'open' | 'other-4xx' | 'refused' {
  if (status === 429) return 'rate-limited'
  if (status >= 500) return 'error'
  if (publicRoutes.some((pattern) => pattern.test(`${route.method} ${route.path}`))) return 'public'
  if (status < 400) return 'open'
  return status === 401 || status === 403 ? 'refused' : 'other-4xx'
}

if (import.meta.main) {
  const list = routes()
  for (const route of list) console.log(route.method.padEnd(6), route.path, ` (${route.file})`)
  console.log(`${list.length} routes`)
}
