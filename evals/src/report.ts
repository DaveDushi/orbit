// Merges the result parts of a run into evals/results/<date>-<sha>.json, compares it with the latest earlier result
// and with evals/budgets.json, prints a summary and sets the exit code:
//   1 = a metric is over budget, a perf metric got worse beyond threshold, floor and noise (compare.ts), or a
//       high/critical security finding that the previous result did not have;
//   2 = a step of the run failed.
import { execSync } from 'node:child_process'
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import { join } from 'node:path'
import { parseArgs } from 'node:util'
import { readJson, work } from './common'
import { caveats, compareMetrics, formatRows, perfMetrics, readNoise, THRESHOLD_PCT } from './compare'
import type { Check } from './headers'

const { values: options } = parseArgs({ options: { duration: { type: 'string' }, perf: { type: 'string' }, security: { type: 'string' }, quick: { type: 'string' }, build: { type: 'string' }, prod: { type: 'string' } } })
const root = join(import.meta.dir, '../..')
// EVALS_RESULTS_DIR: `run.sh --noise` keeps its runs out of evals/results.
const resultsDir = process.env.EVALS_RESULTS_DIR || join(root, 'evals/results')
const shell = (command: string) => {
  try {
    return execSync(command, { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim()
  } catch {
    return 'unknown'
  }
}
const part = <T = any>(name: string): T | undefined => (existsSync(join(work, 'parts', `${name}.json`)) ? readJson<T>(join('parts', `${name}.json`)) : undefined)
const errors = existsSync(join(work, 'errors.txt')) ? readFileSync(join(work, 'errors.txt'), 'utf8').trim().split('\n') : []

const commit = shell('git rev-parse HEAD')
const now = new Date()
const stamp = now.toISOString().slice(0, 16).replace('T', '-').replace(':', '')
const checks: Check[] = ['security-local', 'security-prod', 'supply-chain', 'pageload'].flatMap((name) => part<{ checks: Check[] }>(name)?.checks ?? [])
const findings = part<{ findings: any[] }>('supply-chain')?.findings ?? []

const result = {
  schema: 1,
  commit,
  short: commit.slice(0, 7),
  branch: shell('git rev-parse --abbrev-ref HEAD'),
  dirty: shell('git status --porcelain --untracked-files=no') !== '',
  date: now.toISOString(),
  duration_s: Number(options.duration ?? 0),
  options: { perf: options.perf === '1', security: options.security === '1', quick: options.quick === '1', build: options.build === '1', prod: options.prod === '1' },
  machine: {
    hostname_hash: Bun.hash(os.hostname()).toString(16).slice(0, 8),
    cpu: os.cpus()[0]?.model,
    cpus: os.cpus().length,
    memory_gb: Math.round(os.totalmem() / 2 ** 30),
    kernel: os.release(),
    bun: Bun.version,
    rustc: shell('rustc --version'),
    cargo_build_jobs: Number(process.env.CARGO_BUILD_JOBS ?? 0) || undefined,
    build_slice: existsSync('/etc/systemd/system/orbit-builds.slice'),
  },
  perf: {
    bundle: part('bundle'),
    pageload: part('pageload'),
    latency: part('latency'),
    server: part('server'),
    build: part('build'),
  },
  security: {
    summary: {
      checks: checks.length,
      pass: checks.filter((item) => item.result === 'pass').length,
      fail: checks.filter((item) => item.result === 'fail').length,
      warn: checks.filter((item) => item.result === 'warn').length,
      skip: checks.filter((item) => item.result === 'skip').length,
      findings_by_severity: Object.fromEntries(['critical', 'high', 'medium', 'low', 'info'].map((severity) => [severity, findings.filter((item) => item.severity === severity).length])),
    },
    prod: part('security-prod') ? { site: part('security-prod').site, gated: part('security-prod').gated, tls: part('security-prod').tls_protocol } : undefined,
    checks,
    findings,
  },
  errors,
  comparison: {} as Record<string, unknown>,
}

const metrics = perfMetrics(result)

// The latest earlier result.
mkdirSync(resultsDir, { recursive: true })
const name = `${stamp}-${result.short}.json`
const earlier = readdirSync(resultsDir).filter((file) => /^\d{4}-\d{2}-\d{2}-\d{4}-[0-9a-f]{7}\.json$/.test(file) && file < name).sort()
const previous = earlier.length ? JSON.parse(readFileSync(join(resultsDir, earlier.at(-1)!), 'utf8')) : null
const serious = (data: any): Set<string> =>
  new Set([
    ...(data?.security?.checks ?? []).filter((item: Check) => item.result === 'fail' && ['critical', 'high'].includes(item.severity ?? '')).map((item: Check) => `check:${item.id}`),
    ...(data?.security?.findings ?? []).filter((item: any) => ['critical', 'high'].includes(item.severity)).map((item: any) => `${item.tool}:${item.id}`),
  ])
const before = serious(previous)
const newSerious = [...serious(result)].filter((key) => !before.has(key))
const fixedSerious = [...before].filter((key) => !serious(result).has(key))

const budgets: Record<string, number> = JSON.parse(readFileSync(join(root, 'evals/budgets.json'), 'utf8')).max
const overBudget = Object.entries(budgets)
  .filter(([path, max]) => metrics[path] !== undefined && metrics[path] > max)
  .map(([path, max]) => ({ metric: path, value: metrics[path], budget: max }))
const missingBudgets = Object.keys(budgets).filter((path) => metrics[path] === undefined)

// Noise-aware comparison with the previous result (compare.ts): only `worse` fails the run.
const noise = readNoise()
const compared = previous && result.options.perf
const rows = compared ? compareMetrics(perfMetrics(previous), metrics, noise) : []
const notes = compared ? caveats(previous, result, noise) : []
const worse = rows.filter((row) => row.verdict === 'worse')

result.comparison = {
  previous: earlier.at(-1) ?? null,
  over_budget: overBudget,
  budgets_not_measured: missingBudgets,
  new_high_or_critical: newSerious,
  fixed_high_or_critical: fixedSerious,
  noise: noise ? { commit: noise.commit, runs: noise.runs, date: noise.date } : null,
  notes,
  perf: rows,
}
writeFileSync(join(resultsDir, name), `${JSON.stringify(result, null, 2)}\n`)

// Summary.
console.log(`\nResult: evals/results/${name} (${result.duration_s} s)`)
const show = (label: string, path: string, unit = '') => metrics[path] !== undefined && console.log(`  ${label.padEnd(36)} ${metrics[path]}${unit}`)
show('entry JS gzip', 'perf.bundle.entry_js.gzip_kb', ' kB')
show('entry CSS gzip', 'perf.bundle.entry_css.gzip_kb', ' kB')
show('largest lazy chunk gzip', 'perf.bundle.largest_lazy_gzip_kb', ' kB')
for (const page of Object.keys(result.perf.pageload?.pages ?? {})) show(`LCP ${page}`, `perf.pageload.pages.${page}.lcp_ms`, ' ms')
for (const endpoint of Object.keys(result.perf.latency?.endpoints ?? {})) show(`p95 ${endpoint}`, `perf.latency.endpoints.${endpoint}.p95_ms`, ' ms')
show('cold start', 'perf.server.cold_start_ms', ' ms')
show('RSS idle', 'perf.server.rss_idle_mb', ' MB')
show('release build', 'perf.build.release_build_seconds', ' s')
const s = result.security.summary
console.log(`  security checks: ${s.pass} pass, ${s.fail} fail, ${s.warn} warn, ${s.skip} skip; findings ${JSON.stringify(s.findings_by_severity)}`)
for (const item of checks.filter((check) => check.result === 'fail')) console.log(`  FAIL [${item.severity}] ${item.id}: ${item.title}`)
if (overBudget.length) console.log(`  over budget: ${overBudget.map((item) => `${item.metric} ${item.value} > ${item.budget}`).join('; ')}`)
if (compared) {
  console.log(`\nAgainst ${earlier.at(-1)} (${noise ? `noise of ${noise.runs} runs of ${noise.commit.slice(0, 7)}` : 'no noise file'}):`)
  for (const note of notes) console.log(`  note: ${note}`)
  if (rows.length) for (const line of formatRows(rows)) console.log(`  ${line}`)
  else console.log(`  no metric moved by more than ${THRESHOLD_PCT}%`)
  console.log(`  ${worse.length} worse (fails the run), ${rows.filter((row) => row.verdict === 'better').length} better, ${rows.filter((row) => ['in noise', 'under floor'].includes(row.verdict)).length} inside noise or floor, ${rows.filter((row) => row.verdict === 'missing').length} missing`)
}
if (newSerious.length) console.log(`  new high/critical: ${newSerious.join(', ')}`)
if (errors.length) console.log(`  failed steps: ${errors.join(', ')}`)
process.exit(errors.length ? 2 : overBudget.length || newSerious.length || worse.length ? 1 : 0)
