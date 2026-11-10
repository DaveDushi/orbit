#!/usr/bin/env node
// Screenshots and layout checks of chat on phones (360, 390, 430 px) and on desktop (the thread pane), on the channel
// of scripts/chat-layout/seed.mjs. Prints one line per check; it does not stop at a failure, so a "before" run shows
// every problem. Classic UI settings on a phone are checked too (the settings menu and its links).
//
//   node scripts/chat-layout/shots.mjs <web url> <email> <password> <seed.json> <chromium executable> <out dir> <prefix>
import { mkdirSync, readFileSync } from 'node:fs'
import { chromium } from '../../node_modules/playwright-core/index.mjs'

const [base, email, password, seedFile, executablePath, out, prefix] = process.argv.slice(2)
const seed = JSON.parse(readFileSync(seedFile, 'utf8'))
mkdirSync(out, { recursive: true })
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
let failures = 0
const check = (ok, what) => {
  if (!ok) failures += 1
  console.log(`${ok ? 'ok' : 'FAIL'} - ${what}`)
}

/** Layout facts of the page: horizontal page scroll, card parts wider than their card, small buttons, table columns. */
const facts = () => {
  const root = document.scrollingElement
  const cards = [...document.querySelectorAll('[data-slot=agent-card]')]
  const overflow = []
  for (const card of cards) {
    const box = card.getBoundingClientRect()
    for (const node of card.querySelectorAll('*')) {
      // a part inside a scroll box (a wide table) may be wider: the box scrolls
      if (node.closest('.overflow-x-auto') && node.closest('.overflow-x-auto') !== node) continue
      const r = node.getBoundingClientRect()
      if (r.width > 0 && r.right > box.right + 1) overflow.push(`${node.tagName.toLowerCase()}[${node.getAttribute('data-slot') ?? ''}] ${Math.round(r.right - box.right)}px`)
    }
  }
  const buttons = [...document.querySelectorAll('[data-slot=agent-card] button, [data-slot=agent-card] [role=combobox]')].map((b) => Math.round(b.getBoundingClientRect().height)).filter((h) => h > 0)
  const firstColumns = [...document.querySelectorAll('[data-slot=message-text] table')].map((table) => Math.round(table.querySelector('td')?.getBoundingClientRect().width ?? 0))
  const labels = [...document.querySelectorAll('[data-slot=agent-card-chart] text')].map((t) => t.getBoundingClientRect()).filter((r) => r.width > 0)
  let labelOverlaps = 0
  for (let i = 0; i < labels.length; i++) for (let j = i + 1; j < labels.length; j++) {
    const a = labels[i], b = labels[j]
    if (a.left < b.right - 1 && b.left < a.right - 1 && a.top < b.bottom - 1 && b.top < a.bottom - 1) labelOverlaps += 1
  }
  return { pageScroll: root.scrollWidth - root.clientWidth, overflow: overflow.slice(0, 5), overflowCount: overflow.length, minButton: Math.min(...buttons), firstColumns, labelOverlaps }
}

const browser = await chromium.launch({ executablePath })
async function context(viewport, phone, layout = 'new') {
  const made = await browser.newContext({ viewport, deviceScaleFactor: phone ? 2 : 1, isMobile: phone, hasTouch: phone, colorScheme: 'dark' })
  await made.addInitScript((value) => localStorage.setItem('orbit:layout', value), layout)
  const login = await made.request.post(`${base}/api/v1/auth/login`, { headers: { origin: base, 'x-orbit-contract': 'orbit-api-v1' }, data: { email, password } })
  if (!login.ok()) throw new Error(`login ${login.status()}`)
  return made
}
const scroller = (page) => page.locator('[data-slot=message-scroller]').last()
/** Scrolls the message list up (the list is virtual) until a message with `text` is there, and shows it. */
async function reveal(page, text) {
  const message = page.locator('[data-slot=message-text]', { hasText: text }).first()
  for (let n = 0; n < 40 && (await message.count()) === 0; n++) {
    await scroller(page).evaluate((el) => el.scrollBy(0, -400))
    await sleep(150)
  }
  await message.scrollIntoViewIfNeeded()
  await sleep(300)
}
const shot = (page, name) => page.screenshot({ path: `${out}/${prefix}-${name}.png` })

for (const width of [360, 390, 430]) {
  const ctx = await context({ width, height: 844 }, true)
  const page = await ctx.newPage()
  // the channel: the tables and tool lines at the bottom, then scrolled up a little for the floating day chip
  await page.goto(`${base}/chat/${seed.channelId}`)
  await page.locator('[data-slot=message-text] table').first().waitFor()
  await sleep(800)
  await reveal(page, 'Open orders')
  if (width === 390) await shot(page, `tables-${width}`)
  let f = await page.evaluate(facts)
  check(f.pageScroll <= 0, `${width}px channel: no page scroll sideways (${f.pageScroll}px)`)
  check(f.firstColumns.every((w) => w >= 90), `${width}px Markdown tables: first column at least 90px (${f.firstColumns.join(', ')}px)`)
  await reveal(page, 'Working on it')
  if (width === 390) await shot(page, `tool-lines-${width}`)
  await scroller(page).evaluate((el) => el.scrollBy(0, -260))
  await sleep(500)
  // the chip must not cut a line of text in two: some opaque band behind it
  if (width === 390) await shot(page, `day-chip-${width}`)
  const chip = await page.evaluate(() => {
    const floating = document.querySelector('[data-slot=day-chip-floating]')
    return floating ? getComputedStyle(floating).backgroundImage : null
  })
  check(chip !== null, `${width}px day chip floats while scrolled`)
  check(chip !== null && chip !== 'none', `${width}px floating day chip has a fade behind it (${chip?.slice(0, 40)})`)
  await ctx.close()

  // the thread with the cards, on a tall phone so one screenshot holds them all
  const tall = await context({ width, height: 6000 }, true)
  const thread = await tall.newPage()
  await thread.goto(`${base}/chat/${seed.channelId}/thread/${seed.threadId}`)
  await thread.locator('[data-slot=agent-card]').nth(4).waitFor()
  await sleep(1200)
  f = await thread.evaluate(facts)
  check(f.pageScroll <= 0, `${width}px thread: no page scroll sideways (${f.pageScroll}px)`)
  check(f.overflowCount === 0, `${width}px cards: nothing wider than its card (${f.overflowCount}: ${f.overflow.join('; ')})`)
  check(f.minButton >= 32, `${width}px cards: buttons and selects at least 32px tall (smallest ${f.minButton}px)`)
  check(f.labelOverlaps === 0, `${width}px charts: no labels on top of each other (${f.labelOverlaps})`)
  const cards = thread.locator('[data-slot=agent-card]')
  for (let i = 0; i < (await cards.count()); i++) if (width === 390 || i >= 1) await cards.nth(i).screenshot({ path: `${out}/${prefix}-card${i + 1}-${width}.png` })
  await tall.close()
}

// desktop: the thread pane beside the channel
{
  const ctx = await context({ width: 1440, height: 2400 }, false)
  const page = await ctx.newPage()
  await page.goto(`${base}/chat/${seed.channelId}?thread=${seed.threadId}`)
  await page.locator('[data-slot=agent-card]').nth(4).waitFor()
  await sleep(1200)
  const f = await page.evaluate(facts)
  check(f.pageScroll <= 0, `desktop thread pane: no page scroll sideways (${f.pageScroll}px)`)
  check(f.overflowCount === 0, `desktop thread pane: nothing wider than its card (${f.overflowCount}: ${f.overflow.join('; ')})`)
  await shot(page, 'desktop-thread')
  await ctx.close()
}

// classic UI on a phone: the settings menu has the area's links, and a page has Back to it
for (const area of ['settings', 'profile', 'admin']) {
  const ctx = await context({ width: 390, height: 844 }, true, 'classic')
  const page = await ctx.newPage()
  await page.goto(`${base}/${area}`)
  await page.locator('[data-slot=pane-header]').first().waitFor().catch(() => {})
  await sleep(800)
  const links = await page.locator('main a[href^="/"], section nav a[href^="/"]').evaluateAll((all) => all.map((a) => a.getAttribute('href')).filter((href) => href.split('/').length > 2))
  check(links.length >= 2, `classic UI 390px /${area}: sub-links shown (${links.length})`)
  await shot(page, `classic-${area}`)
  if (links[1]) {
    await page.goto(`${base}${links[1]}`)
    // the page loads lazily
    const back = await page.getByRole('link', { name: /^Back to/ }).waitFor({ timeout: 10_000 }).then(() => true, () => false)
    check(back, `classic UI 390px ${links[1]}: Back to the menu`)
  }
  await ctx.close()
}

await browser.close()
console.log(failures ? `${failures} check(s) failed` : 'all checks passed')
