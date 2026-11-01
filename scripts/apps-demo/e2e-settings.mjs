// Browser proof of Settings → Apps at phone and desktop sizes. Needs an empty Orbit on 127.0.0.1:4201. Saves PNGs and
// fails on the first wrong result: node scripts/apps-demo/e2e-settings.mjs <setup token> <output directory> <chromium executable>
import { randomBytes } from 'node:crypto'
import { chromium } from '../../node_modules/playwright-core/index.mjs'

const [token, out, executablePath] = process.argv.slice(2)
const ORBIT = 'http://127.0.0.1:4201'
const email = 'admin@orbit-apps.test'
const password = randomBytes(18).toString('base64url')

function check(ok, what) {
  if (!ok) throw new Error(`FAILED: ${what}`)
  console.log(`ok - ${what}`)
}
async function api(path, body, cookie) {
  const response = await fetch(`${ORBIT}${path}`, { method: 'POST', headers: { origin: ORBIT, 'content-type': 'application/json', ...(cookie ? { cookie } : {}) }, body: JSON.stringify(body) })
  if (!response.ok) throw new Error(`${path}: ${response.status}`)
  return response
}
const cookie = (await api('/api/v1/setup/complete', { token, email, display_name: 'Ada Admin', password })).headers.get('set-cookie').split(';')[0]
await api('/api/v1/workspaces', { name: 'Wellrox' }, cookie)

const browser = await chromium.launch({ executablePath })
const views = [['390', { width: 390, height: 844 }, true], ['320', { width: 320, height: 568 }, true], ['desktop', { width: 1400, height: 900 }, false]]

async function open([, viewport, phone]) {
  const context = await browser.newContext({ viewport, ...(phone ? { isMobile: true, hasTouch: true, deviceScaleFactor: 2 } : {}) })
  const page = await context.newPage()
  await page.goto(`${ORBIT}/login`)
  await page.getByLabel('Email', { exact: true }).fill(email)
  await page.getByLabel('Password', { exact: true }).fill(password)
  await page.getByRole('button', { name: 'Sign in', exact: true }).click()
  await page.waitForURL((url) => !url.pathname.startsWith('/login'))
  return page
}
async function fits(page, label) {
  const { scroll, width } = await page.evaluate(() => ({ scroll: document.documentElement.scrollWidth, width: window.innerWidth }))
  check(scroll <= width, `${label}: no sideways scroll (${scroll} of ${width})`)
  const wide = await page.evaluate(() => [...document.querySelectorAll('[data-slot=pane] section, [role=dialog]')].filter((el) => el.scrollWidth > el.clientWidth + 1).map((el) => `${el.tagName} ${el.scrollWidth}>${el.clientWidth}: ${[...el.querySelectorAll('*')].filter((child) => child.getBoundingClientRect().right > el.getBoundingClientRect().right + 1).map((child) => child.tagName + '.' + child.getAttribute('data-slot')).slice(0, 4)}`))
  check(wide.length === 0, `${label}: nothing is wider than its card ${wide}`)
}

// The list with no apps, on every size, and the add flow before anything exists.
for (const view of views) {
  const [name] = view
  const page = await open(view)
  await page.goto(`${ORBIT}/settings/apps`)
  await page.getByText('No apps yet.').waitFor()
  await page.waitForTimeout(300)
  await fits(page, `${name} empty list`)
  await page.screenshot({ path: `${out}/${name}-1-list-empty.png` })
  await page.getByRole('button', { name: 'Add app' }).click()
  const dialog = page.getByRole('dialog')
  await dialog.locator('#app-name').fill('Reports')
  await dialog.locator('#app-url').fill('https://reports.example.com')
  check(await dialog.locator('#app-redirects').count() === 0, `${name} add: no sign-in fields while sign-in is off`)
  await page.waitForTimeout(300)
  await fits(page, `${name} add, sign-in off`)
  await page.screenshot({ path: `${out}/${name}-3-add-sign-in-off.png` })
  await dialog.getByRole('switch').click()
  check(await dialog.getByRole('button', { name: 'Add app' }).isDisabled(), `${name} add: sign-in on needs a redirect URI`)
  await dialog.locator('#app-redirects').fill('https://reports.example.com/auth/callback')
  await fits(page, `${name} add, sign-in on`)
  await page.screenshot({ path: `${out}/${name}-4-add-sign-in-on.png` })
  await page.context().close()
}

let number = 0
for (const view of views) {
  const [name] = view
  number += 1
  const app = `Reports ${number}`
  const page = await open(view)
  await page.goto(`${ORBIT}/settings/apps`)

  // Without sign-in: back on the list, with no secret.
  await page.getByRole('button', { name: 'Add app' }).click()
  let dialog = page.getByRole('dialog')
  await dialog.locator('#app-name').fill(`${app} plain`)
  await dialog.locator('#app-url').fill(`https://plain${number}.example.com/a-long-path/that-does-not-fit-on-a-small-phone-screen`)
  await dialog.getByRole('button', { name: 'Add app' }).click()
  await dialog.waitFor({ state: 'hidden' })
  await page.getByRole('link', { name: `Settings of ${app} plain` }).waitFor()
  check(await page.getByText('Orbit shows it only this once').count() === 0, `${name}: no secret for an app without sign-in`)

  // With sign-in: its settings, with the secret once.
  await page.getByRole('button', { name: 'Add app' }).click()
  dialog = page.getByRole('dialog')
  await dialog.locator('#app-name').fill(app)
  await dialog.locator('#app-url').fill(`https://reports${number}.example.com`)
  await dialog.getByRole('switch').click()
  await dialog.locator('#app-redirects').fill(`https://reports${number}.example.com/auth/callback`)
  await dialog.getByRole('button', { name: 'Add app' }).click()
  await page.getByText('Orbit shows it only this once').waitFor()
  await page.waitForURL('**/settings/apps/*')
  const secret = (await page.locator('section', { hasText: 'Orbit shows it only this once' }).last().locator('code').first().textContent()).trim()
  check(secret.startsWith('orbit-app-s-'), `${name}: the secret shows once`)
  await page.waitForTimeout(300)
  await fits(page, `${name} secret`)
  await page.screenshot({ path: `${out}/${name}-5-secret-once.png` })
  await page.reload()
  await page.getByText('Client ID').first().waitFor()
  check(!(await page.locator('body').textContent()).includes(secret), `${name}: the secret is gone after a reload`)
  await fits(page, `${name} settings`)
  await page.screenshot({ path: `${out}/${name}-6-app-settings-top.png` })
  await page.getByText('Client ID').first().scrollIntoViewIfNeeded()
  await page.waitForTimeout(200)
  await page.screenshot({ path: `${out}/${name}-7-app-settings-sign-in.png` })
  await page.getByRole('button', { name: 'Remove', exact: true }).scrollIntoViewIfNeeded()
  await page.waitForTimeout(200)
  await page.screenshot({ path: `${out}/${name}-8-app-settings-remove.png` })

  // Back to the list.
  await page.getByRole('link', { name: 'All apps' }).click()
  await page.getByRole('link', { name: `Settings of ${app}`, exact: true }).waitFor()
  await page.waitForTimeout(300)
  await fits(page, `${name} list`)
  check(await page.getByText('Open', { exact: true }).first().isVisible(), `${name}: each row has Open`)
  await page.screenshot({ path: `${out}/${name}-2-list-with-apps.png` })

  // Rename, new secret, sign-in off, remove.
  await page.getByRole('link', { name: `Settings of ${app}`, exact: true }).click()
  await page.locator('#app-name').fill(`${app} renamed`)
  await page.getByRole('button', { name: 'Save', exact: true }).first().click()
  await page.getByRole('heading', { name: `${app} renamed` }).waitFor()
  check(true, `${name}: the name can change`)
  await page.getByRole('button', { name: 'Make a new secret' }).click()
  await page.getByRole('dialog').getByRole('button', { name: 'Make a new secret' }).click()
  await page.getByText('Orbit shows it only this once').waitFor()
  const next = (await page.locator('section', { hasText: 'Orbit shows it only this once' }).last().locator('code').first().textContent()).trim()
  check(next !== secret, `${name}: a new secret shows once`)
  await page.getByRole('button', { name: 'Done' }).click()
  await page.locator('#app-sign-in').click()
  await page.getByRole('dialog').getByRole('button', { name: 'Turn off' }).click()
  await page.getByText('Sign-in turned off').waitFor()
  check(await page.getByText('Client ID').count() === 0, `${name}: turning sign-in off removes the client`)
  await page.getByRole('button', { name: 'Remove', exact: true }).click()
  await page.getByRole('dialog').getByRole('button', { name: 'Remove' }).click()
  await page.waitForURL('**/settings/apps')
  await page.getByRole('link', { name: `Settings of ${app} plain` }).waitFor()
  check(await page.getByRole('link', { name: `Settings of ${app} renamed` }).count() === 0, `${name}: the app is removed`)
  await page.context().close()
}

await browser.close()
console.log('all checks passed')
