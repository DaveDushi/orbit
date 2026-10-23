/** Writes src/theme/tokens.gen.ts from the web design tokens. Run: bun apps/mobile/scripts/generate-theme.ts */
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'

import { cssColorToRn } from './color'

const root = resolve(import.meta.dir, '../../..')
const css = readFileSync(resolve(root, 'apps/web/src/index.css'), 'utf8')
const output = resolve(root, 'apps/mobile/src/theme/tokens.gen.ts')

function block(selector: string): Map<string, string> {
  const start = css.search(new RegExp(`^${selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*\\{`, 'm'))
  if (start < 0) throw new Error(`No ${selector} block in index.css`)
  const body = css.slice(css.indexOf('{', start) + 1, css.indexOf('\n}', start))
  return new Map([...body.matchAll(/--([\w-]+):\s*([^;]+);/g)].map(([, name, value]) => [name, value.trim()]))
}

const camel = (name: string) => name.replace(/-(\w)/g, (_, char: string) => char.toUpperCase())
const REM = 16

const light = block(':root')
const dark = block('.dark')
const theme = block('@theme inline')

function colors(vars: Map<string, string>): Record<string, string> {
  const lookup = (name: string) => vars.get(name) ?? light.get(name) ?? (() => { throw new Error(`Unknown --${name}`) })()
  const entries = [...vars].filter(([name]) => name !== 'radius').map(([name, value]) => [camel(name), cssColorToRn(value, lookup)])
  return Object.fromEntries(entries.sort(([a], [b]) => a.localeCompare(b)))
}

const baseRadius = Number.parseFloat(light.get('radius')!) * REM
const radius: Record<string, number> = { lg: baseRadius }
for (const [name, value] of theme) {
  const scale = /^radius-([\w]+)$/.exec(name)?.[1]
  const factor = /^calc\(var\(--radius\)\s*\*\s*([\d.]+)\)$/.exec(value)?.[1]
  if (scale && factor) radius[scale] = Math.round(baseRadius * Number(factor) * 100) / 100
  else if (scale && value === 'var(--radius)') radius[scale] = baseRadius
}

const lightColors = colors(light)
const darkColors = colors(dark)
const keys = new Set([...Object.keys(lightColors), ...Object.keys(darkColors)])
const missing = [...keys].filter((key) => !(key in lightColors && key in darkColors))
if (missing.length) throw new Error(`Defined in only one theme: ${missing.join(', ')}`)

const source = `// Generated from apps/web/src/index.css. Do not edit.
// Regenerate: bun apps/mobile/scripts/generate-theme.ts

export const light = ${JSON.stringify(lightColors, null, 2)} as const

export const dark = ${JSON.stringify(darkColors, null, 2)} as const satisfies Record<keyof typeof light, string>

export const radius = ${JSON.stringify(Object.fromEntries(Object.entries(radius).sort(([, a], [, b]) => a - b)), null, 2)} as const
`

mkdirSync(dirname(output), { recursive: true })
writeFileSync(output, source)
console.log(`Wrote ${output}`)
for (const key of ['primary', 'background', 'foreground'] as const) console.log(`${key.padEnd(11)} light ${lightColors[key]}  dark ${darkColors[key]}`)
