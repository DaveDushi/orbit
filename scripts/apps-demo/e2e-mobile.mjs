// Browser proof of the app page on phones and desktop, on the instance that e2e.mjs set up. Saves PNGs and fails on the
// first wrong result: node scripts/apps-demo/e2e-mobile.mjs <logins.txt> <output directory> <chromium executable>
import { readFileSync } from 'node:fs'
import { chromium } from '../../node_modules/playwright-core/index.mjs'

const [logins, out, executablePath] = process.argv.slice(2)
const ORBIT = 'http://127.0.0.1:4201'
const member = readFileSync(logins, 'utf8').split('\n')[1].split(' ')
const IPHONE = 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1'
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

/** Nothing scrolls sideways, and every visible control of the header is inside the screen. */
async function fits(page, label) {
  const result = await page.evaluate(() => {
    const header = document.querySelector('[data-slot=pane-header]')
    const controls = [...header.querySelectorAll('a,button')].filter((el) => el.getClientRects().length > 0)
    return {
      scroll: document.documentElement.scrollWidth,
      width: window.innerWidth,
      right: Math.max(...controls.map((el) => el.getBoundingClientRect().right)),
      headers: [...document.querySelectorAll('header')].filter((el) => el.getClientRects().length > 0).length,
    }
  })
  check(result.scroll <= result.width, `${label}: no sideways scroll (${result.scroll} of ${result.width})`)
  check(result.right <= result.width, `${label}: every header control fits (right edge ${Math.round(result.right)} of ${result.width})`)
  check(result.headers === 0, `${label}: the global top bar is not on this page`)
}

const frameHeight = (page) => page.locator('iframe[data-slot=app-frame]').evaluate((el) => Math.round(el.getBoundingClientRect().height))

const devices = [['390', { width: 390, height: 844 }], ['320', { width: 320, height: 568 }]]
for (const [name, viewport] of devices) {
  for (const [kind, userAgent] of [['phone', ANDROID], ['iphone', IPHONE]]) {
    // Every browser: the frame first, with the ways out in the header.
    const page = await open(viewport, userAgent)
    await page.goto(`${ORBIT}/apps/demo-app`)
    const frame = page.frameLocator('iframe[data-slot=app-frame]').frameLocator('iframe')
    await frame.locator('#who').waitFor({ timeout: 15000 })
    await page.locator('[data-slot=pane-header]').waitFor({ timeout: 12000 }) // no signal from this app
    await page.getByRole('heading', { name: 'Demo App' }).waitFor()
    await fits(page, `${kind} ${name}, frame`)
    check(!(await page.getByText('This browser can block').isVisible()), `${kind} ${name}: no open-first screen`)
    check(await page.getByText('use Sign in to app').isVisible(), `${kind} ${name}: with no signal the help text shows`)
    const signInButton = page.getByRole('button', { name: 'Sign in to app' }).filter({ visible: true })
    check(await signInButton.count() === 1, `${kind} ${name}: Sign in is in the header`)
    check(await page.getByText('Open', { exact: true }).filter({ visible: true }).getAttribute('target') === '_blank', `${kind} ${name}: Open is in the header`)
    await page.screenshot({ path: `${out}/${kind}-${name}-app-in-frame.png` })
    await page.getByRole('button', { name: 'More actions' }).click()
    await page.getByRole('menuitem', { name: 'Reload' }).waitFor()
    await page.screenshot({ path: `${out}/${kind}-${name}-actions-menu.png` })
    await page.keyboard.press('Escape')

    // An app with the ready signal: no bar, the most height, and the actions and the drawer in the corner.
    await page.goto(`${ORBIT}/apps/ready-app`)
    const readyFrame = page.frameLocator('iframe[data-slot=app-frame]').frameLocator('iframe')
    await readyFrame.locator('#who').waitFor({ timeout: 15000 })
    await page.waitForTimeout(9000)
    check(await page.locator('[data-slot=pane-header]').count() === 0, `${kind} ${name}: with the signal there is no bar`)
    const height = await frameHeight(page)
    check(height >= viewport.height - 72, `${kind} ${name}: the frame has ${height} of ${viewport.height} px`)
    check(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), `${kind} ${name}: no sideways scroll without the bar`)
    const corner = await page.getByRole('button', { name: 'App actions' }).evaluate((el) => { const style = getComputedStyle(el); return { background: style.backgroundColor, opacity: style.opacity } })
    check(!/rgba\(.*, 0(\.\d+)?\)|transparent|\/ 0(\.\d+)?\)/.test(corner.background) && corner.opacity === '1', `${kind} ${name}: the corner button is solid on a white app (${corner.background})`)
    await page.screenshot({ path: `${out}/${kind}-${name}-ready-no-bar.png` })
    await page.getByRole('button', { name: 'App actions' }).click()
    await page.getByRole('menuitem', { name: 'Menu' }).waitFor({ timeout: 3000 }) // the menu opens with an animation
    for (const item of ['Menu', 'Sign in to app', 'Reload', 'Open in a new tab']) check(await page.getByRole('menuitem', { name: item }).isVisible(), `${kind} ${name}: ${item} is in the corner menu`)
    await page.screenshot({ path: `${out}/${kind}-${name}-ready-corner-menu.png` })
    await page.getByRole('menuitem', { name: 'Menu' }).click()
    await page.locator('[data-slot=sidebar-nav]').filter({ visible: true }).waitFor()
    check(true, `${kind} ${name}: Menu opens the navigation drawer`)
    await page.keyboard.press('Escape')

    // A refused login: a blank frame, Sign in in the popup, then the frame reloads.
    await page.goto(`${ORBIT}/apps/login-gate`)
    const gate = page.frameLocator('iframe[data-slot=app-frame]').frameLocator('iframe')
    await page.waitForTimeout(2000)
    check(await page.locator('iframe[data-slot=app-frame]').count() === 1 && await gate.locator('#who').count() === 0, `${kind} ${name}: the refused login stays blank in the frame`)
    check(await page.locator('[data-slot=pane-header]').count() === 0, `${kind} ${name}: the bar waits while the frame may still load`)
    await page.locator('[data-slot=pane-header]').waitFor({ timeout: 12000 })
    check(true, `${kind} ${name}: a refused frame gets the bar after the timeout`)
    await fits(page, `${kind} ${name}, blank frame`)
    await page.screenshot({ path: `${out}/${kind}-${name}-blank-frame.png` })
    const [popup] = await Promise.all([page.context().waitForEvent('page'), page.getByRole('button', { name: 'Sign in to app' }).filter({ visible: true }).click()])
    check(await page.getByText('use Sign in to app').isVisible(), `${kind} ${name}: the help text shows while the sign-in runs`)
    await fits(page, `${kind} ${name}, sign-in running`)
    await page.screenshot({ path: `${out}/${kind}-${name}-sign-in-help.png` })
    await popup.locator('#sign-in').click({ timeout: 15000 })
    await popup.locator('#who').waitFor({ timeout: 15000 })
    await popup.close()
    await gate.locator('#who').waitFor({ timeout: 15000 })
    check(!(await page.getByText('use Sign in to app').isVisible()), `${kind} ${name}: the help text goes away after the reload`)
    await page.screenshot({ path: `${out}/${kind}-${name}-signed-in-after-popup.png` })
    await page.context().close()
  }
}

// A long name must shrink, not push the actions off the screen. This app has no sign-in, so its redirect to Orbit is
// refused in the frame, and the card "Sign in to <app>" also shows.
const [, adminEmail, adminPassword] = readFileSync(logins, 'utf8').split('\n')[0].split(' ')
const login = await fetch(`${ORBIT}/api/v1/auth/login`, { method: 'POST', headers: { origin: ORBIT, 'content-type': 'application/json' }, body: JSON.stringify({ email: adminEmail, password: adminPassword }) })
const cookie = login.headers.get('set-cookie').split(';')[0]
const workspaces = await (await fetch(`${ORBIT}/api/v1/workspaces`, { headers: { cookie } })).json()
await fetch(`${ORBIT}/api/v1/workspaces/${(workspaces.items ?? workspaces)[0].id}/workspace-apps`, { method: 'POST', headers: { origin: ORBIT, cookie, 'content-type': 'application/json' }, body: JSON.stringify({ name: 'Product Development Workspace Analytics Dashboard', url: 'http://127.0.0.1:4202/' }) })
const long = await open({ width: 320, height: 568 }, ANDROID)
await long.goto(`${ORBIT}/apps/product-development-workspace-analytics`)
await long.getByRole('heading', { name: 'Product Development Workspace Analytics Dashboard', exact: true }).waitFor({ timeout: 12000 })
await fits(long, '320 wide, long name')
await long.screenshot({ path: `${out}/phone-320-long-name.png` })
await long.context().close()

// Desktop keeps its layout.
const desktop = await open({ width: 1400, height: 900 })
await desktop.goto(`${ORBIT}/apps/demo-app`)
const frame = desktop.frameLocator('iframe[data-slot=app-frame]').frameLocator('iframe')
await frame.locator('#who').waitFor({ timeout: 15000 })
await desktop.locator('[data-slot=pane-header]').waitFor({ timeout: 12000 }) // no signal from this app
for (const name of ['Sign in to app', 'Reload']) check(await desktop.getByRole('button', { name, exact: true }).isVisible(), `desktop: ${name} is in the header`)
check(await desktop.getByText('Open in a new tab', { exact: true }).isVisible(), 'desktop: Open in a new tab is in the header')
check(!(await desktop.getByRole('button', { name: 'More actions' }).isVisible()), 'desktop: no menu in the header')
check(await desktop.getByText('use Sign in to app').isVisible(), 'desktop: the help text stays')
await desktop.screenshot({ path: `${out}/desktop-no-signal-bar.png` })
await desktop.goto(`${ORBIT}/apps/ready-app`)
const readyDesktop = desktop.frameLocator('iframe[data-slot=app-frame]').frameLocator('iframe')
await readyDesktop.locator('#who').waitFor({ timeout: 15000 })
await desktop.waitForTimeout(9000)
check(await desktop.locator('[data-slot=pane-header]').count() === 0, 'desktop: with the signal there is no bar')
await desktop.screenshot({ path: `${out}/desktop-ready-no-bar.png` })
await desktop.getByRole('button', { name: 'App actions' }).click()
check(!(await desktop.getByRole('menuitem', { name: 'Menu' }).isVisible()), 'desktop: the corner menu has no drawer item')
await desktop.screenshot({ path: `${out}/desktop-ready-corner-menu.png` })
await desktop.keyboard.press('Escape')
await desktop.goto(`${ORBIT}/apps/login-gate`)
await desktop.locator('[data-slot=pane-header]').waitFor({ timeout: 12000 })
await desktop.screenshot({ path: `${out}/desktop-refused-frame-bar.png` })

await browser.close()
console.log('all checks passed')
