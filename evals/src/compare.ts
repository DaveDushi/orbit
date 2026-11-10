// Noise-aware comparison of the perf numbers of two results, after Ember's perf/src/compare.ts
// (https://github.com/NYLLON-SOFTWARE/ember-cloudflare, MIT licence). A metric is worse only when it moves by more than
// its threshold percent, by more than its absolute floor, and by more than it moved between runs of one commit
// (evals/noise.json, written by `evals/run.sh --noise`). Improvements and smaller moves are reported and never fail.
//   bun evals/src/compare.ts <before.json> <after.json>    exits 1 when a metric got worse
// No import of ./common: this module also runs outside evals/run.sh (tests, the command above).
import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'

export const THRESHOLD_PCT = 15
export const NOISE_FILE = join(import.meta.dir, '../noise.json')
/** A paint lands on the next frame (60 Hz), so a smaller move is scheduling, not speed. */
const FRAME_MS = 16.7

export type Tolerance = {
  /** The smallest move that counts, in the metric's own unit. */
  floor: number
  /** Percent; THRESHOLD_PCT when not set. */
  threshold?: number
  better?: 'lower' | 'higher'
  /** Counts what should not happen at all (error responses): every increase counts, whatever the percent and noise. */
  anyChange?: boolean
}

// First match wins. Paths are the dotted paths of the result JSON.
const RULES: [RegExp, Tolerance][] = [
  [/\.non_200$/, { floor: 0, anyChange: true }],
  [/\.rps$/, { floor: 100, better: 'higher' }],
  [/^perf\.latency\..*_ms$/, { floor: 1 }],
  [/^perf\.pageload\..*\.(fcp|lcp)_ms$/, { floor: FRAME_MS }],
  // A task only adds to TBT past 50 ms, so one long task more or less moves it by tens of milliseconds.
  [/^perf\.pageload\..*\.tbt_ms$/, { floor: 50 }],
  [/\.cls$/, { floor: 0.02 }],
  [/^perf\.pageload\..*_kb$/, { floor: 5 }],
  [/^perf\.pageload\..*\.requests$/, { floor: 1 }],
  [/^perf\.bundle\..*_kb$/, { floor: 2 }],
  [/^perf\.server\..*_start_ms$/, { floor: 50 }],
  [/^perf\.server\..*_mb$/, { floor: 5 }],
  [/^perf\.build\..*_seconds$/, { floor: 30 }],
  [/^perf\.build\..*_rss_mb$/, { floor: 100 }],
  [/^perf\.build\..*_mb$/, { floor: 1 }],
]

export const tolerance = (metric: string): Required<Tolerance> => ({
  threshold: THRESHOLD_PCT,
  better: 'lower',
  anyChange: false,
  ...(RULES.find(([pattern]) => pattern.test(metric))?.[1] ?? { floor: 0 }),
})

/** Numbers by dotted path. */
export const flatten = (value: unknown, prefix = '', out: Record<string, number> = {}) => {
  if (typeof value === 'number') out[prefix] = value
  else if (value && typeof value === 'object' && !Array.isArray(value)) for (const [key, inner] of Object.entries(value)) flatten(inner, prefix ? `${prefix}.${key}` : key, out)
  return out
}
// Settings of the run, not measurements.
const ignored = (path: string) => /^perf\.(pageload\.(runs|presets)|latency\.(requests|concurrency)|build\.cargo_jobs|bundle\.chunk_count)/.test(path)
/** The perf metrics of a result JSON, by dotted path. */
export const perfMetrics = (result: { perf?: unknown } | null | undefined): Record<string, number> =>
  Object.fromEntries(Object.entries(flatten({ perf: result?.perf ?? {} })).filter(([path]) => !ignored(path)))

export type Noise = {
  commit: string
  runs: number
  date: string
  quick?: boolean
  /** Per metric, over `runs` runs of one commit: `noise` is max - min, in the metric's unit. */
  metrics: Record<string, { median: number; min: number; max: number; noise: number }>
}
export const readNoise = (path = NOISE_FILE): Noise | null => (existsSync(path) ? (JSON.parse(readFileSync(path, 'utf8')) as Noise) : null)

export type Verdict = 'worse' | 'better' | 'in noise' | 'under floor' | 'missing'
export type Row = {
  metric: string
  before: number
  /** Null when the second run did not measure it. */
  after: number | null
  delta: number | null
  /** Percent of before; null when before was 0 or the metric is missing. */
  change_pct: number | null
  /** Run-to-run noise of the metric; null when the noise file does not have it. */
  noise: number | null
  verdict: Verdict
}

const round = (value: number) => Math.round(value * 1000) / 1000

/**
 * How each metric of `before` moved in `after`. Rows only for metrics over their threshold (or, for error counts, any
 * increase) and for metrics `after` did not measure; quieter metrics are left out.
 */
export function compareMetrics(before: Record<string, number>, after: Record<string, number>, noise: Noise | null): Row[] {
  const rows: Row[] = []
  for (const [metric, previous] of Object.entries(before)) {
    const measured = noise?.metrics[metric]?.noise ?? null
    const value = after[metric]
    if (value === undefined) {
      rows.push({ metric, before: previous, after: null, delta: null, change_pct: null, noise: measured, verdict: 'missing' })
      continue
    }
    const rule = tolerance(metric)
    const delta = value - previous
    const change = previous === 0 ? (delta === 0 ? 0 : Infinity) : (delta / Math.abs(previous)) * 100
    if (delta === 0 || (!rule.anyChange && Math.abs(change) <= rule.threshold)) continue
    const improved = rule.better === 'lower' ? delta < 0 : delta > 0
    const verdict: Verdict =
      Math.abs(delta) <= rule.floor ? 'under floor' : !rule.anyChange && Math.abs(delta) <= (measured ?? 0) ? 'in noise' : improved ? 'better' : 'worse'
    rows.push({ metric, before: previous, after: value, delta: round(delta), change_pct: Number.isFinite(change) ? round(change) : null, noise: measured, verdict })
  }
  const order: Verdict[] = ['worse', 'missing', 'better', 'in noise', 'under floor']
  return rows.sort((a, b) => order.indexOf(a.verdict) - order.indexOf(b.verdict) || a.metric.localeCompare(b.metric))
}

/** Text table of the rows: metric, before, after, delta, noise, verdict. */
export function formatRows(rows: Row[]): string[] {
  const cells = rows.map((row) => [
    row.metric.replace(/^perf\./, ''),
    String(row.before),
    row.after === null ? '-' : String(row.after),
    row.delta === null ? '' : `${row.delta > 0 ? '+' : ''}${row.delta} (${row.change_pct === null ? 'from 0' : `${row.change_pct > 0 ? '+' : ''}${row.change_pct}%`})`,
    row.noise === null ? '-' : `±${row.noise}`,
    row.verdict === 'worse' ? 'WORSE' : row.verdict,
  ])
  const all = [['metric', 'before', 'after', 'delta', 'noise', 'verdict'], ...cells]
  const widths = all[0].map((_, column) => Math.max(...all.map((line) => line[column].length)))
  return all.map((line) => line.map((cell, column) => (column === 0 ? cell.padEnd(widths[column]) : cell.padStart(widths[column]))).join('  ').trimEnd())
}

/** Notes on why two results or the noise file do not compare well. */
export function caveats(before: any, after: any, noise: Noise | null): string[] {
  const notes: string[] = []
  if (!noise) notes.push('no evals/noise.json: verdicts use threshold and floor only (measure it with evals/run.sh --noise)')
  else if (noise.quick !== Boolean(after?.options?.quick)) notes.push(`noise was measured with${noise.quick ? '' : 'out'} --quick, this run ${after?.options?.quick ? 'used' : 'did not use'} it`)
  if (Boolean(before?.options?.quick) !== Boolean(after?.options?.quick)) notes.push('one run used --quick and the other did not: request counts differ')
  if (before?.machine?.cpu !== after?.machine?.cpu || before?.machine?.cpus !== after?.machine?.cpus) notes.push('the runs come from different machines')
  return notes
}

if (import.meta.main) {
  const [beforePath, afterPath] = process.argv.slice(2)
  if (!beforePath || !afterPath) {
    console.error('usage: bun evals/src/compare.ts <before.json> <after.json>')
    process.exit(64)
  }
  const before = JSON.parse(readFileSync(beforePath, 'utf8'))
  const after = JSON.parse(readFileSync(afterPath, 'utf8'))
  const noise = readNoise()
  const rows = compareMetrics(perfMetrics(before), perfMetrics(after), noise)
  console.log(`before ${before.short} (${before.date}), after ${after.short} (${after.date})`)
  for (const note of caveats(before, after, noise)) console.log(`note: ${note}`)
  console.log(rows.length ? formatRows(rows).join('\n') : `no metric moved by more than ${THRESHOLD_PCT}%`)
  process.exit(rows.some((row) => row.verdict === 'worse') ? 1 : 0)
}
