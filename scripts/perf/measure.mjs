#!/usr/bin/env node
// Measures the CPU and memory use of the Orbit web app in headless Chromium (Playwright + CDP), on an instance that
// scripts/perf/seed.mjs filled. Run it unchanged before and after a change, then compare the two JSON files.
//
//   node scripts/perf/measure.mjs <web url> <logins.txt> <seed.json> <chromium executable> <output .json>
//   env: BUSY_S (10)  CPU window after each page load
//        SETTLE_S (30) wait before the idle window (the garbage collection after a load is not idle cost)
//        IDLE_S (60)  idle CPU window
//        SOAK_S (300) idle time before the last heap sample
//        SHOTS=<dir>  save a screenshot of each page
//        TABS_ONLY=1  only phase C (the 5 tabs), in a fresh browser
//
// Phases: (A) each page loads in its own browser context (own renderer process, empty cache except "warm load");
// the first BUSY_S seconds give load time, main-thread busy time and long tasks. (B) all pages of A stay open and
// idle together (in parallel, to save time): main-thread time over IDLE_S, JS heap after a forced GC at IDLE_S and
// SOAK_S, timer and animation-frame callbacks, running animations, and the CPU of each Chromium process type (the GPU
// process included). (C) 5 Orbit tabs in one context: renderer CPU and memory (PSS) while idle.
import { readFileSync, writeFileSync } from 'node:fs'
import { chromium } from '../../node_modules/playwright-core/index.mjs'

const [base, loginsFile, seedFile, executablePath, outFile] = process.argv.slice(2)
const BUSY_S = Number(process.env.BUSY_S ?? 10)
const SETTLE_S = Number(process.env.SETTLE_S ?? 30)
const IDLE_S = Number(process.env.IDLE_S ?? 60)
const SOAK_S = Number(process.env.SOAK_S ?? 300)
const SHOTS = process.env.SHOTS
const TABS_ONLY = process.env.TABS_ONLY === '1'
const [, email, password] = readFileSync(loginsFile, 'utf8').trim().split('\n')[0].split(' ')
const seed = JSON.parse(readFileSync(seedFile, 'utf8'))
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const viewport = { width: 1440, height: 900 }

// Counts timer and animation-frame callbacks and collects long tasks, from the first script of the page on.
const probe = () => {
  const perf = (window.__perf = { timers: 0, frames: 0, longTasks: [] })
  for (const name of ['setTimeout', 'setInterval']) {
    const original = window[name]
    window[name] = function (callback, ...rest) {
      if (typeof callback !== 'function') return original.call(this, callback, ...rest)
      return original.call(this, function (...args) { perf.timers++; return callback.apply(this, args) }, ...rest)
    }
  }
  const raf = window.requestAnimationFrame
  window.requestAnimationFrame = (callback) => raf((time) => { perf.frames++; callback(time) })
  new PerformanceObserver((list) => {
    for (const entry of list.getEntries()) perf.longTasks.push({ start: entry.startTime, duration: entry.duration })
  }).observe({ type: 'longtask', buffered: true })
}

// GPU-related state of the page: visible backdrop filters (and their area), running animations, canvas bitmaps.
const gpuState = () => {
  const view = { w: innerWidth, h: innerHeight }
  let blurArea = 0
  let blurCount = 0
  for (const element of document.querySelectorAll('*')) {
    const style = getComputedStyle(element)
    if (style.backdropFilter === 'none' || style.display === 'none' || style.visibility === 'hidden') continue
    const rect = element.getBoundingClientRect()
    const w = Math.max(0, Math.min(rect.right, view.w) - Math.max(rect.left, 0))
    const h = Math.max(0, Math.min(rect.bottom, view.h) - Math.max(rect.top, 0))
    if (w * h > 0) { blurCount++; blurArea += w * h }
  }
  let canvasPixels = 0
  let canvasCount = 0
  let canvasFarAway = 0
  for (const canvas of document.querySelectorAll('canvas')) {
    const pixels = canvas.width * canvas.height
    if (pixels === 0 || (canvas.width === 300 && canvas.height === 150)) continue
    canvasCount++
    canvasPixels += pixels
    const rect = canvas.getBoundingClientRect()
    if (rect.bottom < -2 * view.h || rect.top > 3 * view.h) canvasFarAway++
  }
  const running = document.getAnimations().filter((animation) => animation.playState === 'running')
  return {
    blurCount,
    blurViewportShare: Math.round((blurArea / (view.w * view.h)) * 100),
    runningAnimations: running.length,
    infiniteAnimations: running.filter((animation) => animation.effect?.getTiming().iterations === Infinity).length,
    canvasCount,
    canvasMegapixels: Math.round(canvasPixels / 1e5) / 10,
    canvasMB: Math.round((canvasPixels * 4) / 1e6),
    canvasFarAway,
    nodes: document.getElementsByTagName('*').length,
  }
}

async function metrics(session) {
  const { metrics: list } = await session.send('Performance.getMetrics')
  return Object.fromEntries(list.map((item) => [item.name, item.value]))
}

async function heapAfterGc(session) {
  await session.send('HeapProfiler.collectGarbage')
  return (await metrics(session)).JSHeapUsedSize
}

const browser = await chromium.launch({ executablePath, args: ['--enable-precise-memory-info'] })
const browserSession = await browser.newBrowserCDPSession()
const mb = (bytes) => Math.round(bytes / 1048576 * 10) / 10

async function processes() {
  const { processInfo } = await browserSession.send('SystemInfo.getProcessInfo')
  return processInfo.map((item) => {
    let pss = 0
    try {
      pss = Number(/Pss:\s+(\d+)/.exec(readFileSync(`/proc/${item.id}/smaps_rollup`, 'utf8'))[1]) * 1024
    } catch {}
    return { id: item.id, type: item.type, cpuTime: item.cpuTime, pss }
  })
}
/** CPU ms per process type. Not the renderers: headless Chromium reports no usable cpuTime for them (TaskDuration does). */
const cpuByType = (before, after) => {
  const out = {}
  for (const item of after.filter((process) => process.type !== 'renderer')) {
    const old = before.find((candidate) => candidate.id === item.id)
    out[item.type] = Math.round(((out[item.type] ?? 0) + (item.cpuTime - (old?.cpuTime ?? 0))) * 1000)
  }
  return out
}

// One login; every context starts with its cookies.
const loginContext = await browser.newContext({ viewport })
const loginPage = await loginContext.newPage()
await loginPage.goto(`${base}/login`)
await loginPage.getByLabel('Email', { exact: true }).fill(email)
await loginPage.getByLabel('Password', { exact: true }).fill(password)
await loginPage.getByRole('button', { name: 'Sign in', exact: true }).click()
await loginPage.waitForURL((url) => !url.pathname.startsWith('/login'))
const storageState = await loginContext.storageState()
await loginContext.close()

const openFile = async (page) => {
  const file = page.locator('.bn-block-content[data-content-type="file"]').first()
  await file.waitFor({ timeout: 30000 })
  await file.click()
}
/** Scrolls the PDF from the first to the last page like a reader, then back to page 10. */
const readPdf = async (page) => {
  await page.locator('[data-pdf-pages] canvas').first().waitFor()
  await page.waitForFunction(() => [...document.querySelectorAll('[data-pdf-pages] canvas')].some((canvas) => canvas.width !== 300))
  const scroller = page.locator('[data-pdf-pages]').locator('xpath=..')
  for (;;) {
    const done = await scroller.evaluate((element) => {
      element.scrollTop += element.clientHeight
      return element.scrollTop + element.clientHeight >= element.scrollHeight - 2
    })
    await sleep(120)
    if (done) break
  }
  await sleep(1500)
  await scroller.evaluate((element) => { element.querySelector('[data-pdf-page="10"]').scrollIntoView() })
  await sleep(1500)
}

const scenarios = [
  { name: 'first load (cold cache)', path: '/', ready: (page) => page.locator('[data-slot=sidebar-nav]').waitFor() },
  { name: 'first load (warm cache) = workspace home', path: '/', warm: true, ready: (page) => page.locator('[data-slot=sidebar-nav]').waitFor() },
  { name: 'tasks (300)', path: '/tasks', ready: (page) => page.getByText(/^Task \d+: /).first().waitFor() },
  { name: 'docs: long page', path: `/docs/${seed.longPage}`, ready: (page) => page.getByText('Section 60', { exact: true }).first().waitFor({ timeout: 30000 }) },
  { name: 'docs: PDF 150 pages (DPR 1)', path: `/docs/${seed.pdfPage}`, dpr: 1, ready: openFile, after: readPdf },
  { name: 'docs: PDF 150 pages (DPR 2)', path: `/docs/${seed.pdfPage}`, dpr: 2, ready: openFile, after: readPdf },
  { name: 'docs: spreadsheet', path: `/docs/${seed.sheetPage}`, ready: async (page) => { await openFile(page); await page.getByText('Region', { exact: true }).first().waitFor({ timeout: 30000 }) } },
  { name: 'docs: diagrams', path: `/docs/${seed.diagramPage}`, ready: (page) => page.waitForFunction(() => document.querySelectorAll('[data-slot=mermaid-block] svg').length >= 3, null, { timeout: 30000 }) },
  { name: 'chat: 1500 messages', path: `/chat/${seed.channel}`, ready: (page) => page.getByText('Message 1500 with code', { exact: false }).first().waitFor({ timeout: 30000 }) },
  { name: 'apps: Products frame', path: `/apps/${seed.app}`, ready: (page) => page.frameLocator('iframe[data-slot=app-frame]').frameLocator('iframe').locator('#who').waitFor({ timeout: 30000 }) },
  { name: 'settings', path: '/settings', ready: (page) => page.getByRole('heading').first().waitFor() },
]

let idle, idleProcesses, dialogBlur
const loads = []
if (!TABS_ONLY) {
  // Phase A: load each page and watch its first seconds.
  const open = []
  let warmContext
  for (const scenario of scenarios) {
    const context = scenario.warm ? warmContext : await browser.newContext({ viewport, deviceScaleFactor: scenario.dpr ?? 1, storageState })
    if (!scenario.warm) await context.addInitScript(probe)
    const page = scenario.warm ? context.pages()[0] : await context.newPage()
    const session = await context.newCDPSession(page)
    await session.send('Performance.enable')
    const before = await metrics(session)
    const start = Date.now()
    if (scenario.warm) await page.reload()
    else await page.goto(`${base}${scenario.path}`)
    await scenario.ready(page)
    const loadMs = Date.now() - start
    if (scenario.after) await scenario.after(page)
    await sleep(Math.max(0, BUSY_S * 1000 - (Date.now() - start)))
    const after = await metrics(session)
    const pageInfo = await page.evaluate(() => {
      const resources = performance.getEntriesByType('resource')
      const scripts = resources.filter((entry) => entry.name.endsWith('.js') || entry.initiatorType === 'script')
      const navigation = performance.getEntriesByType('navigation')[0]
      return {
        jsTransferKB: Math.round(scripts.reduce((sum, entry) => sum + entry.transferSize, 0) / 1024),
        jsDecodedKB: Math.round(scripts.reduce((sum, entry) => sum + entry.decodedBodySize, 0) / 1024),
        jsFiles: scripts.length,
        domContentLoadedMs: Math.round(navigation.domContentLoadedEventEnd),
        longTasks: window.__perf.longTasks.length,
        longTaskMs: Math.round(window.__perf.longTasks.reduce((sum, task) => sum + task.duration, 0)),
      }
    })
    const result = {
      name: scenario.name,
      loadMs,
      busyMs: Math.round((after.TaskDuration - (scenario.warm ? before.TaskDuration : 0)) * 1000),
      scriptMs: Math.round((after.ScriptDuration - (scenario.warm ? before.ScriptDuration : 0)) * 1000),
      layoutMs: Math.round((after.LayoutDuration - (scenario.warm ? before.LayoutDuration : 0)) * 1000),
      ...pageInfo,
    }
    loads.push(result)
    // The cold page's context holds the warm page now: phase B measures it under the warm name.
    if (scenario.warm) open.find((item) => item.context === context).result = result
    else open.push({ scenario, context, page, session, result })
    if (!warmContext && scenario.path === '/') warmContext = context
    if (SHOTS) await page.screenshot({ path: `${SHOTS}/${scenarios.indexOf(scenario).toString().padStart(2, '0')}-${scenario.name.replace(/[^a-z0-9]+/gi, '-')}.png` })
    console.log(JSON.stringify(result))
  }
  // Phase B: everything idle at once.
  const gcHeap = async () => Promise.all(open.map(async (item) => heapAfterGc(item.session)))
  const counters = async () => Promise.all(open.map(async (item) => ({ ...(await metrics(item.session)), ...(await item.page.evaluate(() => ({ timers: window.__perf.timers, frames: window.__perf.frames }))) })))
  await sleep(SETTLE_S * 1000)
  const heap0 = await gcHeap()
  const gpu = await Promise.all(open.map((item) => item.page.evaluate(gpuState)))
  const c0 = await counters()
  const p0 = await processes()
  await sleep(IDLE_S * 1000)
  const c1 = await counters()
  const p1 = await processes()
  const heap1 = await gcHeap()
  await sleep(Math.max(0, SOAK_S - SETTLE_S - IDLE_S) * 1000)
  const heap5 = await gcHeap()
  const p5 = await processes()
  const nodes5 = await Promise.all(open.map((item) => item.page.evaluate(() => document.getElementsByTagName('*').length)))
  idle = open.map((item, index) => ({
    name: item.result.name,
    idleCpuMsPerS: Math.round(((c1[index].TaskDuration - c0[index].TaskDuration) / IDLE_S) * 10000) / 10,
    timersPerS: Math.round(((c1[index].timers - c0[index].timers) / IDLE_S) * 10) / 10,
    framesPerS: Math.round(((c1[index].frames - c0[index].frames) / IDLE_S) * 10) / 10,
    layoutsPerMin: Math.round(((c1[index].LayoutCount - c0[index].LayoutCount) / IDLE_S) * 60),
    heapMB0: mb(heap0[index]),
    heapMB1: mb(heap1[index]),
    heapMB5: mb(heap5[index]),
    heapGrowthMB: mb(heap5[index] - heap1[index]),
    nodes5: nodes5[index],
    ...gpu[index],
  }))
  idleProcesses = { cpuMsOverIdle: cpuByType(p0, p1), cpuMsOverSoak: cpuByType(p0, p5), pssMB: Object.fromEntries(Object.entries(p5.reduce((sum, item) => ({ ...sum, [item.type]: (sum[item.type] ?? 0) + item.pss }), {})).map(([type, bytes]) => [type, mb(bytes)])) }
  for (const item of open) await item.context.close()

  // A dialog over the app: how much of the screen gets a backdrop blur.
  const dialogContext = await browser.newContext({ viewport, storageState })
  const dialogPage = await dialogContext.newPage()
  await dialogPage.goto(`${base}/`)
  await dialogPage.locator('[data-slot=sidebar-nav]').waitFor()
  await dialogPage.keyboard.press('Control+k')
  await dialogPage.getByRole('dialog').first().waitFor()
  await sleep(500)
  dialogBlur = await dialogPage.evaluate(gpuState)
  await dialogContext.close()
}

// Phase C: 5 Orbit tabs in one browser context, all idle.
const tabsContext = await browser.newContext({ viewport, storageState })
await tabsContext.addInitScript(probe)
const tabPaths = ['/', '/tasks', `/docs/${seed.longPage}`, `/chat/${seed.channel}`, '/settings']
const tabs = []
for (const path of tabPaths) {
  const page = await tabsContext.newPage()
  await page.goto(`${base}${path}`)
  await page.locator('[data-slot=sidebar-nav]').waitFor()
  tabs.push(page)
}
const tabSessions = await Promise.all(tabs.map((page) => tabsContext.newCDPSession(page)))
for (const session of tabSessions) await session.send('Performance.enable')
await sleep(SETTLE_S * 1000)
const t0 = await processes()
const tabBusy0 = await Promise.all(tabSessions.map(async (session) => (await metrics(session)).TaskDuration))
await sleep(IDLE_S * 1000)
const t1 = await processes()
const tabBusy1 = await Promise.all(tabSessions.map(async (session) => (await metrics(session)).TaskDuration))
const tabHeaps = await Promise.all(tabSessions.map((session) => heapAfterGc(session)))
const renderers = t1.filter((item) => item.type === 'renderer')
const fiveTabs = {
  rendererProcesses: renderers.length,
  mainThreadMsPerS: Math.round((tabBusy1.reduce((sum, value) => sum + value, 0) - tabBusy0.reduce((sum, value) => sum + value, 0)) / IDLE_S * 10000) / 10,
  cpuMsOverIdle: cpuByType(t0, t1),
  rendererPssMB: mb(renderers.reduce((sum, item) => sum + item.pss, 0)),
  gpuPssMB: mb(t1.filter((item) => item.type === 'GPU').reduce((sum, item) => sum + item.pss, 0)),
  jsHeapMB: tabHeaps.map(mb),
}
await browser.close()

const report = { at: new Date().toISOString(), base, BUSY_S, SETTLE_S, IDLE_S, SOAK_S, load: loads, idle, idleProcesses, dialogBlur, fiveTabs }
writeFileSync(outFile, JSON.stringify(report, null, 2))
console.log(JSON.stringify({ idle, idleProcesses, dialogBlur, fiveTabs }, null, 1))
