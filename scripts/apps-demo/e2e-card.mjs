// Browser proof of the card "Sign in to <app>" on the instance that e2e.mjs set up, with its demo apps running. Saves
// PNGs and fails on the first wrong result: node scripts/apps-demo/e2e-card.mjs <logins.txt> <output directory> <chromium executable>
import { readFileSync } from 'node:fs'
import { chromium } from '../../node_modules/playwright-core/index.mjs'

const [logins, out, executablePath] = process.argv.slice(2)
const ORBIT = 'http://127.0.0.1:4201'
const member = readFileSync(logins, 'utf8').split('\n')[1].split(' ')
const ANDROID = 'Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Mobile Safari/537.36'

function check(ok, what) {
  if (!ok) throw new Error(`FAILED: ${what}`)
  console.log(`ok - ${what}`)
}

const browser = await chromium.launch({ executablePath })

async function open(viewport, userAgent) {
  const context = await browser.newContext({ viewport, ...(userAgent ? { userAgent, isMobile: true, hasTouch: true, deviceScaleFactor: 2 } : {}) })
  const page = await context.newPage()
  await page.goto(`${ORBIT}/login`)
  await page.getByLabel('Email', { exact: true }).fill(member[1])
  await page.getByLabel('Password', { exact: true }).fill(member[2])
  await page.getByRole('button', { name: 'Sign in', exact: true }).click()
  await page.waitForURL((url) => !url.pathname.startsWith('/login'))
  return page
}

const card = (page) => page.locator('[data-slot=app-sign-in-card]')
const bar = (page) => page.locator('[data-slot=pane-header]')
const appFrame = (page) => page.frameLocator('iframe[data-slot=app-frame]').frameLocator('iframe')
const shot = (page, name) => page.screenshot({ path: `${out}/${name}.png` })
const quiet = (page) => page.waitForTimeout(1000)
const storage = (page, prefix, slug) => page.evaluate(([prefix, slug]) => Object.keys(localStorage).filter((key) => key.startsWith(prefix) && key.endsWith(`:${slug}`)).map((key) => localStorage.getItem(key)), [prefix, slug])

/** The card and every control of it are inside the screen, and nothing scrolls sideways. */
async function fits(page, label) {
  const result = await page.evaluate(() => {
    const dialog = document.querySelector('[data-slot=app-sign-in-card] [role=alertdialog]')
    const boxes = [dialog, ...dialog.querySelectorAll('a,button')].map((el) => el.getBoundingClientRect())
    return { left: Math.min(...boxes.map((box) => box.left)), right: Math.max(...boxes.map((box) => box.right)), width: window.innerWidth, scroll: document.documentElement.scrollWidth }
  })
  check(result.left >= 0 && result.right <= result.width && result.scroll <= result.width, `${label}: the card fits (${Math.round(result.left)}..${Math.round(result.right)} of ${result.width})`)
}

// Case 1: Orbit answers login_required in the frame. The card shows at once, for an app that never sent the signal.
const desktop = await open({ width: 1400, height: 900 })
const started = Date.now()
await desktop.goto(`${ORBIT}/apps/prompt-login`)
await card(desktop).waitFor({ timeout: 5000 })
check(Date.now() - started < 5000, `case 1: the card shows without the 8 s wait (${Date.now() - started} ms)`)
check(await card(desktop).getByRole('heading').textContent() === 'Sign in to Prompt Login', 'case 1: the title names the app')
check((await card(desktop).textContent()).includes('Prompt Login asked you to sign in, and its login cannot open inside Orbit.'), 'case 1: the reason line')
await appFrame(desktop).locator('#signin-error').waitFor({ timeout: 10000 })
check((await appFrame(desktop).locator('#signin-error').textContent()).includes('login_required'), "case 1: Orbit's page went on to the app's callback with login_required")
check(await bar(desktop).isVisible(), 'case 1: the bar is open')
check(await desktop.getByText('use Sign in to app').count() === 0, 'case 1: the card replaces the help line')
for (const name of ['Sign in to app', 'Reload', 'Show the app']) check(await card(desktop).getByRole('button', { name, exact: true }).isVisible(), `case 1: the card has ${name}`)
check(await card(desktop).getByText('Open in a new tab', { exact: true }).getAttribute('target') === '_blank', 'case 1: the card has Open in a new tab')
await shot(desktop, 'desktop-1-card-bar-open')

// Show the app: the card goes until the next failure, and the help line comes back.
await card(desktop).getByRole('button', { name: 'Show the app' }).click()
check(await card(desktop).count() === 0, 'Show the app hides the card')
check(await desktop.getByText('use Sign in to app').isVisible(), 'Show the app: the help line is back in the open bar')
// Forged messages: from the app's origin, and from Orbit's own top page. Neither is Orbit's page in the app frame.
const appWindow = desktop.frames().find((frame) => frame.url().startsWith('http://127.0.0.1:4218'))
await appWindow.evaluate((orbit) => window.top.postMessage({ type: 'orbit:app-login-required' }, orbit), ORBIT)
await desktop.evaluate(() => window.postMessage({ type: 'orbit:app-login-required' }, '*'))
await quiet(desktop)
check(await card(desktop).count() === 0, 'a forged login_required message (from the app, or from the top page) shows no card')

// Reload (in the bar) starts again, and the card comes back.
await bar(desktop).getByRole('button', { name: 'Reload', exact: true }).click()
await card(desktop).waitFor({ timeout: 5000 })
check(true, 'Reload starts the rule again: the next failure shows the card')
// Sign in: the card goes while the popup is open, and comes back when the popup closes and the frame reloads.
const [popup] = await Promise.all([desktop.context().waitForEvent('page'), card(desktop).getByRole('button', { name: 'Sign in to app' }).click()])
check(await card(desktop).count() === 0, 'Sign in hides the card while the popup is open')
await popup.close()
await card(desktop).waitFor({ timeout: 8000 })
check(true, 'the frame reloads after the popup, and the failure shows the card again')
// A blocked popup: the card says so.
await desktop.evaluate(() => { window.open = () => null })
await card(desktop).getByRole('button', { name: 'Sign in to app' }).click()
await card(desktop).getByRole('alert').filter({ hasText: 'blocked the sign-in window' }).waitFor()
check(true, 'a blocked popup shows its message in the card')
await shot(desktop, 'desktop-2-card-popup-blocked')

// The closed bar: the card still shows, and the choice stays.
await desktop.goto(`${ORBIT}/apps/prompt-login`)
await card(desktop).waitFor({ timeout: 5000 })
await desktop.getByRole('button', { name: 'Close bar' }).click()
check(await bar(desktop).count() === 0 && await card(desktop).isVisible(), 'closing the bar keeps the card')
await desktop.reload()
await card(desktop).waitFor({ timeout: 5000 })
check(await bar(desktop).count() === 0, 'after a page reload the bar stays closed, and the card shows')
check(await desktop.getByRole('button', { name: 'App actions' }).isVisible(), 'the corner menu stays with the card')
await shot(desktop, 'desktop-3-card-bar-closed')
await card(desktop).getByRole('button', { name: 'Show the app' }).click()
check(JSON.stringify(await storage(desktop, 'orbit:app-bar-closed:', 'prompt-login')) === '["1"]', 'the card does not change the closed choice')

// Case 2: the app sends the frame to a login origin that frame-src refuses.
await desktop.goto(`${ORBIT}/apps/other-login`)
await card(desktop).waitFor({ timeout: 5000 })
check((await card(desktop).textContent()).includes('Other Login tried to open a login page that Orbit does not allow in the frame.'), 'case 2: a refused frame navigation shows the card at once')
await desktop.getByRole('button', { name: 'Close bar' }).click()
await shot(desktop, 'desktop-4-card-frame-blocked')

// Known ready: the first visit teaches this browser that the app sends the signal (5 s after its page).
await desktop.goto(`${ORBIT}/apps/slow-ready`)
await appFrame(desktop).locator('#who').waitFor({ timeout: 15000 })
await desktop.waitForFunction(() => Object.keys(localStorage).some((key) => key.startsWith('orbit:app-ready-seen:') && key.endsWith(':slow-ready')), null, { timeout: 10000 })
check(await card(desktop).count() === 0 && await bar(desktop).count() === 0, 'known ready: the first visit has no card, and the signal is remembered')
// Now it is silent 3 s after its load: a failure. The late signal hides the card at once.
const second = Date.now()
await desktop.goto(`${ORBIT}/apps/slow-ready`)
await card(desktop).waitFor({ timeout: 7000 })
const after = Date.now() - second
check(after >= 2500, `known ready: the card waits 3 s after the load (${after} ms)`)
check((await card(desktop).textContent()).includes('Slow Ready did not finish loading.'), 'known ready: the reason line')
await shot(desktop, 'desktop-5-known-ready-silent-card')
await card(desktop).waitFor({ state: 'detached', timeout: 6000 })
await quiet(desktop)
check(await bar(desktop).count() === 0, 'the ready signal hides the card and the bar')
await shot(desktop, 'desktop-6-card-hidden-after-ready')

// An app without the snippet that works: only today's bar after 8 s, no card.
await desktop.goto(`${ORBIT}/apps/demo-app`)
await appFrame(desktop).locator('#who').waitFor({ timeout: 15000 })
await bar(desktop).waitFor({ timeout: 12000 })
await quiet(desktop)
check(await card(desktop).count() === 0 && await desktop.getByText('use Sign in to app').isVisible(), 'an app without the snippet gets the bar and help line, no card')
check(JSON.stringify(await storage(desktop, 'orbit:app-ready-seen:', 'demo-app')) === '[]', 'an app without the snippet is not known ready')
await shot(desktop, 'desktop-7-no-snippet-bar-only')
await desktop.context().close()

// Phones: 390 with the bar closed, 320 with the bar open.
const wide = await open({ width: 390, height: 844 }, ANDROID)
await wide.goto(`${ORBIT}/apps/prompt-login`)
await card(wide).waitFor({ timeout: 5000 })
await wide.getByRole('button', { name: 'Close bar' }).click()
await fits(wide, 'phone 390, bar closed')
await shot(wide, 'phone-390-card-bar-closed')
await wide.context().close()
const narrow = await open({ width: 320, height: 568 }, ANDROID)
await narrow.goto(`${ORBIT}/apps/prompt-login`)
await card(narrow).waitFor({ timeout: 5000 })
check(await bar(narrow).isVisible(), 'phone 320: the bar is open')
await fits(narrow, 'phone 320, bar open')
await shot(narrow, 'phone-320-card-bar-open')
await narrow.context().close()

await browser.close()
console.log('all checks passed')
