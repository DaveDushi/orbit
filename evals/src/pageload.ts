// Page load on the test instance with Playwright (Chromium) and the browser's own performance entries: FCP, LCP, CLS,
// TBT, transfer size, and the JS/CSS each page loads. Lighthouse-like presets: desktop (40 ms RTT, 10 Mbit/s, no CPU
// slowdown) and phone (412x823, 150 ms RTT, 1.6 Mbit/s, CPU 4x slower). Every run uses a new browser context, so the
// cache is cold. Median of 3 runs (1 with --quick). Writes $EVALS_WORK/parts/pageload.json.
//
// TBT here is the sum of (long task - 50 ms) from FCP to the end of the observation window (Lighthouse stops at TTI),
// so it can be a little higher than Lighthouse's TBT.
import { chromium, type BrowserContext } from 'playwright-core'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { instance, login, median, quick, readJson, round, writePart, type Seed } from './common'
import { check, type Check } from './headers'

const server = instance()
const seed = readJson<Seed>('seed.json')
const session = await login(seed.a.email, seed.password)
const runs = quick ? 1 : 3

const pages = [
  { name: 'login', path: '/login', signedIn: false },
  { name: 'home', path: '/', signedIn: true },
  { name: 'app_tasks', path: '/tasks', signedIn: true },
  { name: 'docs', path: `/docs/${seed.a.pageId}`, signedIn: true },
]
const devices = {
  desktop: { viewport: { width: 1350, height: 940 }, isMobile: false, hasTouch: false, deviceScaleFactor: 1, latency: 40, down: 10_240, up: 10_240, cpu: 1, settle: 3000 },
  phone: { viewport: { width: 412, height: 823 }, isMobile: true, hasTouch: true, deviceScaleFactor: 1.75, latency: 150, down: 1_638, up: 750, cpu: 4, settle: 6000 },
}

// Collects LCP, layout shifts and long tasks from the first byte on.
const observe = () => {
  const state = { lcp: 0, cls: 0, windowValue: 0, windowStart: 0, lastShift: 0, longTasks: [] as { start: number; duration: number }[] }
  ;(window as any).__evals = state
  new PerformanceObserver((list) => {
    for (const entry of list.getEntries()) state.lcp = entry.startTime
  }).observe({ type: 'largest-contentful-paint', buffered: true })
  new PerformanceObserver((list) => {
    for (const entry of list.getEntries() as any[]) {
      if (entry.hadRecentInput) continue
      // CLS: the largest session window (shifts < 1 s apart, window < 5 s).
      if (entry.startTime - state.lastShift > 1000 || entry.startTime - state.windowStart > 5000) {
        state.windowStart = entry.startTime
        state.windowValue = 0
      }
      state.lastShift = entry.startTime
      state.windowValue += entry.value
      state.cls = Math.max(state.cls, state.windowValue)
    }
  }).observe({ type: 'layout-shift', buffered: true })
  new PerformanceObserver((list) => {
    for (const entry of list.getEntries()) state.longTasks.push({ start: entry.startTime, duration: entry.duration })
  }).observe({ type: 'longtask', buffered: true })
}

const collect = () => {
  const state = (window as any).__evals
  const fcp = performance.getEntriesByName('first-contentful-paint')[0]?.startTime ?? 0
  const navigation = performance.getEntriesByType('navigation')[0] as PerformanceNavigationTiming
  const resources = performance.getEntriesByType('resource') as PerformanceResourceTiming[]
  const bytes = (filter: (entry: PerformanceResourceTiming) => boolean) => resources.filter(filter).reduce((sum, entry) => sum + entry.transferSize, 0)
  return {
    fcp,
    lcp: state.lcp,
    cls: state.cls,
    tbt: state.longTasks.filter((task: { start: number }) => task.start >= fcp).reduce((sum: number, task: { duration: number }) => sum + Math.max(0, task.duration - 50), 0),
    transfer: navigation.transferSize + bytes(() => true),
    js: bytes((entry) => /\.m?js(\?|$)/.test(entry.name)),
    css: bytes((entry) => /\.css(\?|$)/.test(entry.name)),
    requests: resources.length + 1,
  }
}

const browser = await chromium.launch({ executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE || undefined })
const results: Record<string, Record<string, unknown>> = {}
const consoleErrors: Record<string, string[]> = {}
for (const [deviceName, device] of Object.entries(devices)) {
  for (const target of pages) {
    const samples: ReturnType<typeof collect>[] = []
    for (let run = 0; run < runs; run++) {
      const context: BrowserContext = await browser.newContext({ viewport: device.viewport, isMobile: device.isMobile, hasTouch: device.hasTouch, deviceScaleFactor: device.deviceScaleFactor })
      if (target.signedIn) await context.addCookies([{ name: server.cookie, value: session, url: server.base }])
      await context.addInitScript(observe)
      const page = await context.newPage()
      page.on('console', (message) => {
        if (message.type() !== 'error') return
        const list = (consoleErrors[target.name] ??= [])
        const text = message.text().slice(0, 200)
        if (!list.includes(text)) list.push(text)
      })
      const cdp = await context.newCDPSession(page)
      await cdp.send('Network.enable')
      await cdp.send('Network.emulateNetworkConditions', { offline: false, latency: device.latency, downloadThroughput: (device.down * 1024) / 8, uploadThroughput: (device.up * 1024) / 8 })
      await cdp.send('Emulation.setCPUThrottlingRate', { rate: device.cpu })
      await page.goto(server.base + target.path, { waitUntil: 'load', timeout: 120_000 })
      await page.waitForTimeout(device.settle)
      if (target.signedIn && new URL(page.url()).pathname.startsWith('/login')) throw new Error(`${target.name}: redirected to login, the session cookie did not work`)
      samples.push(await page.evaluate(collect))
      await context.close()
    }
    const pick = (key: keyof ReturnType<typeof collect>) => median(samples.map((sample) => sample[key]))
    results[`${target.name}.${deviceName}`] = {
      fcp_ms: round(pick('fcp'), 0),
      lcp_ms: round(pick('lcp'), 0),
      tbt_ms: round(pick('tbt'), 0),
      cls: round(pick('cls'), 3),
      transfer_kb: round(pick('transfer') / 1024),
      js_kb: round(pick('js') / 1024),
      css_kb: round(pick('css') / 1024),
      requests: pick('requests'),
    }
    console.log(`  ${target.name} ${deviceName}: LCP ${results[`${target.name}.${deviceName}`].lcp_ms} ms, TBT ${results[`${target.name}.${deviceName}`].tbt_ms} ms`)
  }
}
await browser.close()

// Does the server compress JS/CSS? Without it every cold load sends the raw bundle.
const html = readFileSync(join(import.meta.dir, '../../apps/web/dist/index.html'), 'utf8')
const entry = html.match(/src="(\/assets\/[^"]+\.js)"/)![1]
const asset = await fetch(server.base + entry, { headers: { 'accept-encoding': 'br, gzip, zstd' } })
const encoding = asset.headers.get('content-encoding') ?? 'none'
const wire = (await asset.arrayBuffer()).byteLength // Bun decodes; the size on the wire is in content-length
const checks: Check[] = [
  check('perf.asset-compression', 'performance', 'JS/CSS are served compressed (br/gzip/zstd)', encoding !== 'none', 'high', [`GET ${entry.replace(/-[\w-]{8}\.js$/, '.js')} with Accept-Encoding: br, gzip, zstd -> content-encoding ${encoding}, content-length ${asset.headers.get('content-length') ?? '-'} (${round(wire / 1024)} kB decoded)`], 'Serve precompressed .br/.gz files from the build, or add tower-http CompressionLayer for /assets and /.'),
  ...pages.map((target) => {
    const csp = (consoleErrors[target.name] ?? []).filter((text) => /Content Security Policy/i.test(text))
    return check(`console.csp.${target.name}`, 'csp', `${target.name}: no CSP console errors`, csp.length === 0, 'low', csp.length ? csp : ['none'], 'Move the inline style to a stylesheet or a constructed style sheet.')
  }),
]
writePart('pageload', { runs, presets: devices, pages: results, console_errors: consoleErrors, asset_encoding: encoding, checks })
