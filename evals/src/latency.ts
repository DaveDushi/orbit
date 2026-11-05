// API latency on the test instance with oha: fixed concurrency and request count per endpoint, signed in as user A.
// Writes $EVALS_WORK/parts/latency.json (milliseconds).
import { spawnSync } from 'node:child_process'
import { call, instance, login, quick, readJson, round, writePart, type Seed } from './common'

const server = instance()
const seed = readJson<Seed>('seed.json')
const { a } = seed
const session = await login(a.email, seed.password)
const ws = `/api/v1/workspaces/${a.workspaceId}`
const requests = quick ? 100 : 500
const concurrency = 10

const endpoints: Record<string, string> = {
  health_ready: '/health/ready',
  index_html: '/',
  session_me: '/api/v1/auth/me',
  workspace_list: '/api/v1/workspaces',
  docs_list: `${ws}/pages`,
  docs_read: `${ws}/pages/${a.pageId}`,
  docs_search: `${ws}/pages/search?q=searchable`,
  app_list: `${ws}/apps`,
  tasks_list: `${ws}/tasks?project_id=${a.projectId}`,
  task_read: `${ws}/tasks/${a.taskId}`,
  file_read: `${ws}/pages/${a.pageId}/files/${a.fileId}`,
}

const results: Record<string, unknown> = {}
for (const [name, path] of Object.entries(endpoints)) {
  // Fail fast on an endpoint that does not answer 200, so a wrong path is not measured as fast errors.
  const probe = await call('GET', path, { session })
  if (probe.status !== 200) {
    results[name] = { error: `GET answered ${probe.status}` }
    continue
  }
  const args = ['--no-tui', '--output-format', 'json', '-c', String(concurrency), '-H', `cookie: ${server.cookie}=${session}`, '-H', 'x-orbit-contract: orbit-api-v1', server.base + path]
  spawnSync('oha', ['-n', '20', ...args]) // warm-up
  const run = spawnSync('oha', ['-n', String(requests), ...args], { encoding: 'utf8', maxBuffer: 16 * 1024 * 1024 })
  const report = JSON.parse(run.stdout)
  const percentile = (key: string) => round((report.latencyPercentiles[key] ?? 0) * 1000, 2)
  const statuses = report.statusCodeDistribution as Record<string, number>
  results[name] = {
    p50_ms: percentile('p50'),
    p95_ms: percentile('p95'),
    p99_ms: percentile('p99'),
    rps: round(report.summary.requestsPerSec, 0),
    non_200: Object.entries(statuses).filter(([code]) => code !== '200').reduce((sum, [, count]) => sum + count, 0),
  }
  console.log(`  ${name}: p50 ${percentile('p50')} ms, p95 ${percentile('p95')} ms`)
}
writePart('latency', { requests, concurrency, endpoints: results })
