import { describe, expect, test } from 'bun:test'
import { join } from 'node:path'
import { caveats, compareMetrics, type Noise, perfMetrics, readNoise, tolerance } from './compare'

const LATENCY = 'perf.latency.endpoints.docs_list.p95_ms' // floor 1 ms
const LCP = 'perf.pageload.pages.home.desktop.lcp_ms' // floor 16.7 ms
const noiseOf = (metrics: Record<string, number>): Noise => ({
  commit: 'abc',
  runs: 5,
  date: '2026-11-10',
  metrics: Object.fromEntries(Object.entries(metrics).map(([metric, noise]) => [metric, { median: 0, min: 0, max: noise, noise }])),
})
const verdict = (metric: string, before: number, after: number, noise: Noise | null = null): string =>
  compareMetrics({ [metric]: before }, { [metric]: after }, noise)[0]?.verdict ?? 'quiet'

describe('compareMetrics', () => {
  test('a move under the threshold is quiet, even past floor and noise', () => {
    expect(verdict(LCP, 1000, 1140, noiseOf({ [LCP]: 10 }))).toBe('quiet') // +14%
  })

  test('over the threshold but not over the floor is under floor', () => {
    expect(verdict(LATENCY, 2, 2.9)).toBe('under floor') // +45%, +0.9 ms
    expect(verdict(LCP, 50, 66)).toBe('under floor') // +32%, under one frame
  })

  test('over threshold and floor but inside the noise is in noise', () => {
    expect(verdict(LCP, 1000, 1300, noiseOf({ [LCP]: 400 }))).toBe('in noise')
  })

  test('over threshold, floor and noise is worse', () => {
    expect(verdict(LCP, 1000, 1300, noiseOf({ [LCP]: 100 }))).toBe('worse')
    expect(verdict(LATENCY, 2, 4, noiseOf({ [LATENCY]: 0.5 }))).toBe('worse')
  })

  test('the same move the other way is better', () => {
    expect(verdict(LCP, 1300, 1000, noiseOf({ [LCP]: 100 }))).toBe('better')
  })

  test('throughput is better when higher', () => {
    const rps = 'perf.latency.endpoints.docs_list.rps'
    expect(tolerance(rps).better).toBe('higher')
    expect(verdict(rps, 4000, 3000)).toBe('worse')
    expect(verdict(rps, 3000, 4000)).toBe('better')
  })

  test('error responses count from the first one, whatever the noise', () => {
    const errors = 'perf.latency.endpoints.docs_list.non_200'
    expect(verdict(errors, 0, 1, noiseOf({ [errors]: 5 }))).toBe('worse')
    expect(verdict(errors, 0, 0)).toBe('quiet')
  })

  test('a metric the new run did not measure is missing, a new metric is ignored', () => {
    const rows = compareMetrics({ [LCP]: 1000 }, { [LATENCY]: 2 }, null)
    expect(rows).toEqual([{ metric: LCP, before: 1000, after: null, delta: null, change_pct: null, noise: null, verdict: 'missing' }])
  })

  test('no baseline gives no rows', () => {
    expect(compareMetrics(perfMetrics(null), { [LCP]: 1000 }, null)).toEqual([])
  })

  test('without a noise file, or without the metric in it, threshold and floor decide', () => {
    expect(verdict(LCP, 1000, 1300, null)).toBe('worse')
    expect(verdict(LCP, 1000, 1300, noiseOf({ [LATENCY]: 1000 }))).toBe('worse')
    expect(readNoise(join(import.meta.dir, 'no-such-noise.json'))).toBeNull()
    expect(caveats({}, {}, null)[0]).toContain('no evals/noise.json')
  })

  test('a rise from zero has no percent and still needs the floor', () => {
    const tbt = 'perf.pageload.pages.home.desktop.tbt_ms' // floor 50 ms
    expect(compareMetrics({ [tbt]: 0 }, { [tbt]: 30 }, null)[0]).toMatchObject({ verdict: 'under floor', change_pct: null })
    expect(verdict(tbt, 0, 80)).toBe('worse')
  })

  test('worse rows come first', () => {
    const rows = compareMetrics({ [LCP]: 1300, [LATENCY]: 2 }, { [LCP]: 1000, [LATENCY]: 4 }, null)
    expect(rows.map((row) => row.verdict)).toEqual(['worse', 'better'])
  })
})

describe('perfMetrics', () => {
  test('keeps measurements, drops run settings', () => {
    const metrics = perfMetrics({ perf: { latency: { requests: 500, endpoints: { a: { p95_ms: 3 } } }, pageload: { runs: 3, pages: { 'home.desktop': { lcp_ms: 900 } } }, bundle: { chunk_count: 100 } } })
    expect(metrics).toEqual({ 'perf.latency.endpoints.a.p95_ms': 3, 'perf.pageload.pages.home.desktop.lcp_ms': 900 })
  })
})
