// Writes evals/noise.json from several results of one commit: per perf metric the median, min, max and noise
// (max - min), which compare.ts reads as the metric's run-to-run noise. `evals/run.sh --noise [N]` calls it.
//   bun evals/src/noise.ts <result.json> <result.json> [...]
import { readFileSync, writeFileSync } from 'node:fs'
import { NOISE_FILE, perfMetrics } from './compare'

const round = (value: number) => Math.round(value * 100) / 100
const median = (values: number[]) => {
  const sorted = [...values].sort((a, b) => a - b)
  const middle = Math.floor(sorted.length / 2)
  return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2
}

const results = process.argv.slice(2).map((path) => JSON.parse(readFileSync(path, 'utf8')))
if (results.length < 2) throw new Error('noise needs at least two results')
const commits = new Set(results.map((result) => result.commit))
if (commits.size > 1) throw new Error(`the results come from different commits: ${[...commits].join(', ')}`)
const quick = new Set(results.map((result) => Boolean(result.options?.quick)))
if (quick.size > 1) throw new Error('some results used --quick and some did not')

const runs = results.map(perfMetrics)
const metrics: Record<string, { median: number; min: number; max: number; noise: number }> = {}
// Only metrics every run measured.
for (const metric of Object.keys(runs[0]).filter((key) => runs.every((run) => run[key] !== undefined)).sort()) {
  const values = runs.map((run) => run[metric])
  const [min, max] = [Math.min(...values), Math.max(...values)]
  metrics[metric] = { median: round(median(values)), min, max, noise: round(max - min) }
}
const noise = {
  _about: 'Run-to-run noise per perf metric: max - min over `runs` runs of one commit on one machine. evals/src/compare.ts counts a metric as worse only when it moves by more than this. Re-measure with `evals/run.sh --noise` after a change to the VM or to the measurements.',
  commit: results[0].commit,
  runs: results.length,
  date: new Date().toISOString(),
  quick: [...quick][0],
  machine: { cpu: results[0].machine?.cpu, cpus: results[0].machine?.cpus, memory_gb: results[0].machine?.memory_gb },
  metrics,
}
writeFileSync(NOISE_FILE, `${JSON.stringify(noise, null, 2)}\n`)
const widest = Object.entries(metrics)
  .filter(([, value]) => value.median)
  .map(([metric, value]) => [metric, (value.noise / Math.abs(value.median)) * 100] as const)
  .sort((a, b) => b[1] - a[1])
  .slice(0, 10)
console.log(`evals/noise.json: ${Object.keys(metrics).length} metrics, ${results.length} runs of ${results[0].commit.slice(0, 7)}; noisiest (max - min, % of median):`)
for (const [metric, pct] of widest) console.log(`  ${metric.replace(/^perf\./, '').padEnd(48)} ±${metrics[metric].noise} (${Math.round(pct)}%)`)
