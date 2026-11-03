#!/usr/bin/env node
// Browser check of the pages that the perf changes touch, on desktop (1440x900) and phone (390x844), with screenshots.
// Fails on the first wrong result. Needs the instance of scripts/perf/seed.mjs.
//
//   node scripts/perf/e2e.mjs <web url> <logins.txt> <seed.json> <chromium executable> <screenshot directory>
import { randomUUID } from 'node:crypto'
import { mkdirSync, readFileSync } from 'node:fs'
import { chromium } from '../../node_modules/playwright-core/index.mjs'

const [base, loginsFile, seedFile, executablePath, out] = process.argv.slice(2)
const [, email, password] = readFileSync(loginsFile, 'utf8').trim().split('\n')[0].split(' ')
const seed = JSON.parse(readFileSync(seedFile, 'utf8'))
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
mkdirSync(out, { recursive: true })

function check(ok, what) {
  if (!ok) throw new Error(`FAILED: ${what}`)
  console.log(`ok - ${what}`)
}

/** Number of drawn PDF canvases, and whether page `number` holds a drawing (not only white). */
const pdfState = (number) => {
  const canvases = [...document.querySelectorAll('[data-pdf-pages] canvas')]
  const drawn = canvases.filter((canvas) => canvas.width > 0 && !(canvas.width === 300 && canvas.height === 150))
  const canvas = canvases[number - 1]
  let inked = false
  if (drawn.includes(canvas)) {
    const data = canvas.getContext('2d').getImageData(0, 0, canvas.width, canvas.height).data
    for (let index = 0; index < data.length && !inked; index += 4 * 97) inked = data[index] < 128
  }
  return { pages: canvases.length, drawn: drawn.length, inked }
}

const browser = await chromium.launch({ executablePath })
for (const device of [
  { name: 'desktop', viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1 },
  { name: 'phone', viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true },
]) {
  const { name, ...options } = device
  const context = await browser.newContext(options)
  const page = await context.newPage()
  const shot = (label) => page.screenshot({ path: `${out}/${name}-${label}.png` })
  await page.goto(`${base}/login`)
  await page.getByLabel('Email', { exact: true }).fill(email)
  await page.getByLabel('Password', { exact: true }).fill(password)
  await page.getByRole('button', { name: 'Sign in', exact: true }).click()
  await page.waitForURL((url) => !url.pathname.startsWith('/login'))

  // PDF: the first pages draw, far pages give their bitmap back, and pages draw again on the way back.
  await page.goto(`${base}/docs/${seed.pdfPage}`)
  const file = page.locator('.bn-block-content[data-content-type="file"]').first()
  await file.waitFor({ timeout: 30000 })
  await file.click()
  await page.waitForFunction(() => document.querySelector('[data-pdf-page="1"]')?.width > 300, null, { timeout: 30000 })
  await sleep(1000)
  let state = await page.evaluate(pdfState, 1)
  check(state.pages === 150 && state.inked, `${name}: PDF shows 150 pages and page 1 is drawn`)
  await shot('01-pdf-top')
  // Read down to page 75 one screen at a time, as a person scrolls, so every page on the way comes into view.
  const scroller = page.locator('[data-pdf-pages]').locator('xpath=..')
  for (;;) {
    const past = await scroller.evaluate((element) => {
      element.scrollTop += element.clientHeight
      return element.querySelector('[data-pdf-page="75"]').getBoundingClientRect().top < element.getBoundingClientRect().bottom
    })
    await sleep(150)
    if (past) break
  }
  await page.waitForFunction(() => document.querySelector('[data-pdf-page="75"]')?.width > 300, null, { timeout: 15000 })
  await sleep(1000)
  state = await page.evaluate(pdfState, 75)
  check(state.inked && state.drawn <= 8, `${name}: page 75 is drawn and only ${state.drawn} pages hold a bitmap`)
  await shot('02-pdf-page-75')
  await scroller.evaluate((element) => { element.scrollTop = 0 })
  await page.waitForFunction(() => document.querySelector('[data-pdf-page="1"]')?.width > 300, null, { timeout: 15000 })
  await sleep(1000)
  state = await page.evaluate(pdfState, 1)
  check(state.inked, `${name}: back at the top, page 1 is drawn again`)
  await shot('03-pdf-back-to-top')

  // Realtime: a visible task renamed through the API shows its new title in the open list without a reload.
  await page.goto(`${base}/tasks`)
  const row = page.getByText(/^Task \d+: /).first()
  await row.waitFor({ timeout: 30000 })
  await sleep(1500)
  const title = (await row.textContent()).trim()
  const renamed = `Renamed ${name} ${randomUUID().slice(0, 8)}`
  const patched = await page.evaluate(async ({ workspaceId, title, renamed }) => {
    const headers = { 'content-type': 'application/json', 'x-orbit-contract': 'orbit-api-v1' }
    const found = await (await fetch(`/api/v1/workspaces/${workspaceId}/tasks?search=${encodeURIComponent(title)}&limit=50`, { headers })).json()
    const task = found.items.find((item) => item.title === title)
    const response = await fetch(`/api/v1/workspaces/${workspaceId}/tasks/${task.id}`, { method: 'PATCH', headers, body: JSON.stringify({ expected_version: task.version, title: renamed }) })
    return response.ok
  }, { workspaceId: seed.workspaceId, title, renamed })
  check(patched, `${name}: "${title}" renamed through the API`)
  await page.getByText(renamed).first().waitFor({ timeout: 10000 })
  check(true, `${name}: the open task list shows the new title without a reload`)
  await shot('04-tasks-realtime')

  // The other measured pages, for a look.
  for (const [label, path, ready] of [
    ['05-home', '/', (p) => p.locator('main, [data-slot=sidebar-nav]').first().waitFor()],
    ['06-docs-long', `/docs/${seed.longPage}`, (p) => p.getByText('Section 1', { exact: true }).first().waitFor({ timeout: 30000 })],
    ['07-chat', `/chat/${seed.channel}`, (p) => p.getByText('Message 1500 with code', { exact: false }).first().waitFor({ timeout: 30000 })],
    ['08-settings', '/settings', (p) => p.getByRole('heading').first().waitFor()],
  ]) {
    await page.goto(`${base}${path}`)
    await ready(page)
    await sleep(1500)
    await shot(label)
  }
  await context.close()
}
await browser.close()
console.log('all checks passed')
