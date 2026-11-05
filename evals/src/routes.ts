// Lists every HTTP route of the server from the Rust source: each `.route(path, get(..).post(..))` call under
// apps/server/src. A path can be a string literal, a `let`/`const` string in the same file, or `&format!("{x}/..")`
// built from them. The OpenAPI contract adds nothing new today; it is merged in so a route that the scanner cannot
// read still gets tested. Usage as a module: `routes()`; as a script it prints the list.
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

export function routes(): Route[] {
  const found = new Map<string, Route>()
  const unresolved: string[] = []
  for (const file of rustFiles(join(root, 'apps/server/src'))) {
    const source = readFileSync(file, 'utf8').replace(/#\[cfg\(test\)\][\s\S]*$/, '')
    const strings = new Map<string, string>()
    for (const match of source.matchAll(/(?:let|const)\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*(?::\s*&(?:'static\s+)?str\s*)?=\s*"([^"]*)"/g)) strings.set(match[1], match[2])
    for (const match of source.matchAll(/\.route(_service)?\(/g)) {
      const [pathExpression, handler] = firstArgument(callArguments(source, match.index! + match[0].length))
      const path = resolvePath(pathExpression, strings)
      if (path === null) {
        unresolved.push(`${relative(root, file)}: ${pathExpression}`)
        continue
      }
      // A service route (`/mcp`) answers every method; test the ones it serves (POST, GET, DELETE).
      if (match[1]) for (const verb of ['GET', 'POST', 'DELETE']) found.set(`${verb} ${path}`, { method: verb, path, file: relative(root, file) })
      const verbs = new Set([...handler.matchAll(/\b(get|post|put|patch|delete|any)(?:_service)?\(/g)].map((m) => m[1]))
      for (const verb of verbs.has('any') ? methods : verbs) {
        const key = `${verb.toUpperCase()} ${path}`
        found.set(key, { method: verb.toUpperCase(), path, file: relative(root, file) })
      }
    }
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

if (import.meta.main) {
  const list = routes()
  for (const route of list) console.log(route.method.padEnd(6), route.path, ` (${route.file})`)
  console.log(`${list.length} routes`)
}
