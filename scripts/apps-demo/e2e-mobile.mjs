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

for (const [name, viewport] of [['390', { width: 390, height: 844 }], ['320', { width: 320, height: 568 }]]) {
  // A phone with a browser that allows cookies in frames: the frame comes first.
  const page = await open(viewport, ANDROID)
  await page.goto(`${ORBIT}/apps/demo-app`)
  const frame = page.frameLocator('iframe[data-slot=app-frame]').frameLocator('iframe')
  await frame.locator('#who').waitFor({ timeout: 15000 })
  await page.getByRole('heading', { name: 'Demo App' }).waitFor()
  await fits(page, `${name} wide, frame`)
  check(!(await page.getByText('use Sign in to app').isVisible()), `${name} wide: the help text takes no space at rest`)
  const height = await frameHeight(page)
  check(height >= viewport.height - 48 - 72, `${name} wide: the frame has ${height} of ${viewport.height} px`)
  await page.screenshot({ path: `${out}/phone-${name}-app-in-frame.png` })
  await page.getByRole('button', { name: 'More actions' }).click()
  await page.getByRole('menuitem', { name: 'Reload' }).waitFor()
  check(await page.getByRole('menuitem', { name: 'Sign in to app' }).isVisible(), `${name} wide: Sign in to app is in the menu`)
  await page.screenshot({ path: `${out}/phone-${name}-actions-menu.png` })
  await page.keyboard.press('Escape')

  // The refused login: the popup from the menu signs in, and the frame reloads.
  await page.goto(`${ORBIT}/apps/login-gate`)
  const gate = page.frameLocator('iframe[data-slot=app-frame]').frameLocator('iframe')
  await page.getByRole('button', { name: 'More actions' }).click()
  const [popup] = await Promise.all([page.context().waitForEvent('page'), page.getByRole('menuitem', { name: 'Sign in to app' }).click()])
  check(await page.getByText('use Sign in to app').isVisible(), `${name} wide: the help text shows while the sign-in runs`)
  await fits(page, `${name} wide, sign-in running`)
  await page.screenshot({ path: `${out}/phone-${name}-sign-in-help.png` })
  await popup.locator('#sign-in').click({ timeout: 15000 })
  await popup.locator('#who').waitFor({ timeout: 15000 })
  await popup.close()
  await gate.locator('#who').waitFor({ timeout: 15000 })
  check(!(await page.getByText('use Sign in to app').isVisible()), `${name} wide: the help text goes away after the reload`)
  await page.context().close()

  // An iPhone: the app opens first, and no blank frame shows.
  const iphone = await open(viewport, IPHONE)
  await iphone.goto(`${ORBIT}/apps/behind-access`)
  await iphone.getByText('This browser can block sign-in inside Orbit').waitFor()
  check(await iphone.locator('iframe[data-slot=app-frame]').count() === 0, `${name} iPhone: no frame before the person asks for it`)
  await fits(iphone, `${name} iPhone`)
  const first = iphone.getByText('Open Behind Access', { exact: true })
  check(await first.getAttribute('target') === '_blank', `${name} iPhone: the first action opens the app in its own tab`)
  await iphone.screenshot({ path: `${out}/iphone-${name}-open-first.png` })
  await iphone.getByRole('button', { name: 'Try inside Orbit' }).click()
  await iphone.locator('iframe[data-slot=app-frame]').waitFor()
  await iphone.reload()
  await iphone.locator('iframe[data-slot=app-frame]').waitFor()
  check(true, `${name} iPhone: the choice to try the frame stays after a reload`)
  await iphone.context().close()
}

// A long name must shrink, not push the actions off the screen.
const [, adminEmail, adminPassword] = readFileSync(logins, 'utf8').split('\n')[0].split(' ')
const login = await fetch(`${ORBIT}/api/v1/auth/login`, { method: 'POST', headers: { origin: ORBIT, 'content-type': 'application/json' }, body: JSON.stringify({ email: adminEmail, password: adminPassword }) })
const cookie = login.headers.get('set-cookie').split(';')[0]
const workspaces = await (await fetch(`${ORBIT}/api/v1/workspaces`, { headers: { cookie } })).json()
await fetch(`${ORBIT}/api/v1/workspaces/${(workspaces.items ?? workspaces)[0].id}/workspace-apps`, { method: 'POST', headers: { origin: ORBIT, cookie, 'content-type': 'application/json' }, body: JSON.stringify({ name: 'Product Development Workspace Analytics Dashboard', url: 'http://127.0.0.1:4202/' }) })
const long = await open({ width: 320, height: 568 }, ANDROID)
await long.goto(`${ORBIT}/apps/product-development-workspace-analytics`)
await long.getByRole('heading', { name: /Product Development Workspace/ }).waitFor()
await fits(long, '320 wide, long name')
await long.screenshot({ path: `${out}/phone-320-long-name.png` })
await long.context().close()

// Desktop keeps its layout.
const desktop = await open({ width: 1400, height: 900 })
await desktop.goto(`${ORBIT}/apps/demo-app`)
const frame = desktop.frameLocator('iframe[data-slot=app-frame]').frameLocator('iframe')
await frame.locator('#who').waitFor({ timeout: 15000 })
for (const name of ['Sign in to app', 'Reload']) check(await desktop.getByRole('button', { name, exact: true }).isVisible(), `desktop: ${name} is in the header`)
check(await desktop.getByText('Open in a new tab', { exact: true }).isVisible(), 'desktop: Open in a new tab is in the header')
check(!(await desktop.getByRole('button', { name: 'More actions' }).isVisible()), 'desktop: no menu in the header')
check(await desktop.getByText('use Sign in to app').isVisible(), 'desktop: the help text stays')
await desktop.screenshot({ path: `${out}/desktop-app-in-frame.png` })

await browser.close()
console.log('all checks passed')
