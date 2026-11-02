// Browser proof of Orbit Apps against a live test instance (see docs/apps.md): saves PNGs and
// fails on the first wrong result. Needs an empty Orbit on 127.0.0.1:4201 and these arguments:
//   node scripts/apps-demo/e2e.mjs <setup token> <output directory> <chromium executable> <start-demo script> <demo.env>
import { randomBytes } from 'node:crypto'
import { execFileSync } from 'node:child_process'
import { writeFileSync } from 'node:fs'
import { chromium } from '../../node_modules/playwright-core/index.mjs'

const [token, out, executablePath, startDemo, demoEnv] = process.argv.slice(2)
const ORBIT = 'http://127.0.0.1:4201'
const same = 'http://127.0.0.1:4202'
const cross = 'http://localhost:4203'
const passwords = { owner: randomBytes(18).toString('base64url'), member: randomBytes(18).toString('base64url') }
const emails = { owner: 'admin@orbit-apps.test', member: 'member@orbit-apps.test' }

function check(ok, what) {
  if (!ok) throw new Error(`FAILED: ${what}`)
  console.log(`ok - ${what}`)
}

async function api(path, { method = 'GET', body, cookie } = {}) {
  const response = await fetch(`${ORBIT}${path}`, {
    method,
    headers: { origin: ORBIT, 'content-type': 'application/json', ...(cookie ? { cookie } : {}) },
    body: body ? JSON.stringify(body) : undefined,
  })
  const text = await response.text()
  if (!response.ok) throw new Error(`${method} ${path}: ${response.status} ${text}`)
  return { data: text ? JSON.parse(text) : null, cookie: response.headers.get('set-cookie')?.split(';')[0] }
}

// The workspace and its two people.
const setup = await api('/api/v1/setup/complete', { method: 'POST', body: { token, email: emails.owner, display_name: 'Ada Admin', password: passwords.owner } })
const ownerCookie = setup.cookie
const workspace = (await api('/api/v1/workspaces', { method: 'POST', cookie: ownerCookie, body: { name: 'Wellrox' } })).data
const workspaceId = workspace.id ?? workspace.workspace?.id
const invitation = (await api(`/api/v1/workspaces/${workspaceId}/invitations`, { method: 'POST', cookie: ownerCookie, body: { email: emails.member, role: 'member', delivery: 'manual' } })).data
await api('/api/v1/workspaces/invitations/accept', { method: 'POST', body: { token: new URL(invitation.url).searchParams.get('token'), email: emails.member, display_name: 'Mia Member', password: passwords.member } })
writeFileSync(`${out}/../logins.txt`, `admin ${emails.owner} ${passwords.owner}\nmember ${emails.member} ${passwords.member}\n`, { mode: 0o600 })

const browser = await chromium.launch({ executablePath })
const shot = (page, name) => page.screenshot({ path: `${out}/${name}.png` })

async function signedIn(person) {
  const context = await browser.newContext({ viewport: { width: 1400, height: 900 } })
  const page = await context.newPage()
  await page.goto(`${ORBIT}/login`)
  await page.getByLabel('Email', { exact: true }).fill(emails[person])
  await page.getByLabel('Password', { exact: true }).fill(passwords[person])
  await page.getByRole('button', { name: 'Sign in', exact: true }).click()
  await page.waitForURL((url) => !url.pathname.startsWith('/login'))
  return { context, page }
}

// The owner adds the apps in the settings page.
const owner = await signedIn('owner')
const { page } = owner
await page.goto(`${ORBIT}/settings/apps`)
await page.getByRole('button', { name: 'Add app' }).waitFor()
await shot(page, '01-apps-settings-empty')

async function addApp({ name, url, redirect, adminsOnly, newTab }) {
  await page.goto(`${ORBIT}/settings/apps`)
  await page.getByRole('button', { name: 'Add app' }).click()
  const dialog = page.getByRole('dialog')
  await dialog.locator('#app-name').fill(name)
  await dialog.locator('#app-url').fill(url)
  if (adminsOnly) await dialog.getByRole('checkbox', { name: 'members' }).click()
  if (redirect) {
    await dialog.getByRole('switch').click()
    await dialog.locator('#app-redirects').fill(redirect)
  }
  await dialog.getByRole('button', { name: 'Add app' }).click()
  await dialog.waitFor({ state: 'hidden' })
  if (newTab) {
    await page.getByRole('link', { name: `Settings of ${name}` }).click()
    await page.locator('#app-mode').click()
    await page.getByRole('option', { name: 'In a new tab' }).click()
    await page.getByText('Saved').first().waitFor()
  }
}
async function secretShown() {
  const card = page.locator('section', { hasText: 'Orbit shows it only this once' }).last()
  await card.waitFor()
  const value = (await card.locator('code').textContent()).trim()
  return { card, value }
}
const clients = async () => (await api(`/api/v1/workspaces/${workspaceId}/workspace-apps`, { cookie: ownerCookie })).data.items

await addApp({ name: 'Demo App', url: same, redirect: `${same}/auth/callback` })
const first = await secretShown()
check(first.value.startsWith('orbit-app-s-'), 'the secret shows once after adding the app')
await shot(page, '02-app-added-secret-shown-once')
await page.reload()
await page.getByText('Demo App', { exact: true }).first().waitFor()
check((await page.locator('body').textContent()).includes(first.value) === false, 'the secret is gone after a reload')
await page.getByText('Client ID').first().scrollIntoViewIfNeeded()
await shot(page, '03-apps-settings-setup-values')

await addApp({ name: 'Cross Site', url: cross, redirect: `${cross}/auth/callback` })
const second = await secretShown()
await addApp({ name: 'Admins Only', url: 'http://127.0.0.1:4209', adminsOnly: true })
await addApp({ name: 'Own Tab', url: `${same}/`, newTab: true })
// Behind a stand-in for Cloudflare Access on its own origin, whose answers refuse frames.
const guarded = [
  { name: 'Behind Access', slug: 'behind-access', app: 4210, access: 4211, env: 'INSTANT' },
  { name: 'Login Gate', slug: 'login-gate', app: 4212, access: 4213, env: 'GATE' },
  { name: 'Gate With COOP', slug: 'gate-with-coop', app: 4214, access: 4215, env: 'COOP' },
  { name: 'Ready App', slug: 'ready-app', app: 4216, access: 4217, env: 'READY' },
]
const guardedSecrets = {}
for (const item of guarded) {
  await addApp({ name: item.name, url: `http://127.0.0.1:${item.app}`, redirect: `http://127.0.0.1:${item.access}/callback` })
  guardedSecrets[item.env] = (await secretShown()).value
}
// For the sign-in card (e2e-card.mjs): Orbit answers login_required in the frame (the app asks for prompt=login),
// a login on an origin that the frame does not allow (4220, no sign-in with Orbit), and a slow app with the signal.
await addApp({ name: 'Prompt Login', url: 'http://127.0.0.1:4218', redirect: 'http://127.0.0.1:4218/auth/callback' })
guardedSecrets.PROMPT = (await secretShown()).value
await addApp({ name: 'Other Login', url: 'http://127.0.0.1:4219' })
await addApp({ name: 'Slow Ready', url: 'http://127.0.0.1:4221', redirect: 'http://127.0.0.1:4221/auth/callback' })
guardedSecrets.SLOW = (await secretShown()).value
const signInApps = [...guarded, { name: 'Prompt Login', env: 'PROMPT' }, { name: 'Slow Ready', env: 'SLOW' }]
const items = await clients()
const idOf = (name) => items.find((item) => item.name === name).sign_in.client_id
writeFileSync(demoEnv, `SAME_ID=${idOf('Demo App')}\nSAME_SECRET=${first.value}\nCROSS_ID=${idOf('Cross Site')}\nCROSS_SECRET=${second.value}\n${signInApps.map((item) => `${item.env}_ID=${idOf(item.name)}\n${item.env}_SECRET=${guardedSecrets[item.env]}\n`).join('')}`, { mode: 0o600 })
execFileSync(startDemo, { stdio: 'ignore' })
await new Promise((resolve) => setTimeout(resolve, 1500))

// A member: sidebar, the app in its frame, signed in with their own name.
const member = await signedIn('member')
const mp = member.page
const nav = mp.locator('[data-slot=sidebar-nav]')
await nav.getByRole('link', { name: 'Demo App' }).waitFor()
check(await nav.getByRole('link', { name: 'Admins Only' }).count() === 0, 'a role that is not allowed gets no sidebar entry')
await shot(mp, '04-sidebar-entries-member')
await nav.getByRole('link', { name: 'Demo App' }).click()
await mp.waitForURL('**/apps/demo-app')
const demo = mp.frameLocator('iframe[data-slot=app-frame]').frameLocator('iframe')
await demo.locator('#who').waitFor({ timeout: 15000 })
const who = await demo.locator('#who').textContent()
check(who.includes('Mia Member'), `the app in the frame knows the member: "${who}"`)
check((await demo.locator('#role').textContent()) === 'member', 'the app knows the role')
await shot(mp, '05-app-in-frame-signed-in-as-member')

// Reload and the help text.
await mp.getByRole('button', { name: 'Reload' }).click()
await demo.locator('#who').waitFor()
check(await mp.getByText('Open in a new tab', { exact: true }).getAttribute('href') === `${same}/`, 'Open in a new tab goes to the app')
await mp.getByText('use Sign in to app').waitFor()
check(true, 'without a signal from the app, the help text shows after the timeout')
await mp.getByText('use Sign in to app').evaluate((el) => { el.style.outline = '2px solid #f2458f' })
await shot(mp, '06-sign-in-control-and-help')

// The real framing: the wrapper's own CSP, in the browser.
const frameResponse = await mp.request.get(`${ORBIT}/api/v1/workspaces/${workspaceId}/apps/demo-app/frame`)
const policy = frameResponse.headers()['content-security-policy']
check(policy.includes('frame-ancestors \'self\'') && policy.includes(`frame-src ${same}`), `the wrapper has its own CSP: ${policy}`)

// An app that sends the ready signal: no bar, the whole height, and the actions in the corner.
await mp.goto(`${ORBIT}/apps/ready-app`)
const readyFrame = mp.frameLocator('iframe[data-slot=app-frame]').frameLocator('iframe')
await readyFrame.locator('#who').waitFor({ timeout: 15000 })
await mp.waitForTimeout(9000) // past the timeout
check(await mp.locator('[data-slot=pane-header]').count() === 0, 'an app with the signal shows no bar')
check(await mp.locator('[data-slot=app-frame]').evaluate((el) => el.getBoundingClientRect().height === el.parentElement.getBoundingClientRect().height), 'the frame takes the full height')
await shot(mp, '17-ready-app-without-bar')
await mp.getByRole('button', { name: 'App actions' }).click()
await mp.getByRole('menuitem', { name: 'Reload' }).click()
await readyFrame.locator('#who').waitFor({ timeout: 15000 })
await mp.waitForTimeout(9000)
check(await mp.locator('[data-slot=pane-header]').count() === 0, 'after a Reload the same rule holds')
// A message from Orbit's own page is not the app's signal.
await mp.goto(`${ORBIT}/apps/demo-app`)
await mp.evaluate(() => window.postMessage({ type: 'orbit:app-ready' }, '*'))
await mp.locator('[data-slot=pane-header]').waitFor({ timeout: 12000 })
check(true, 'a forged message does not hide the bar')

// A role that is not allowed: a 404 at the API, and the same page as an unknown app.
const denied = await mp.request.get(`${ORBIT}/api/v1/workspaces/${workspaceId}/apps/admins-only/frame`)
check(denied.status() === 404, 'the frame of an app that is not allowed is a 404')
const list = await (await mp.request.get(`${ORBIT}/api/v1/workspaces/${workspaceId}/apps`)).json()
check(!list.items.some((item) => item.slug === 'admins-only'), 'the list has no app that is not allowed')
await mp.goto(`${ORBIT}/apps/admins-only`)
await mp.getByText('App not found').waitFor()
await shot(mp, '07-role-not-allowed-404')

// The app's new_tab page.
await mp.goto(`${ORBIT}/apps/own-tab`)
await mp.getByText('This app opens in its own tab.').waitFor()
check(await nav.getByRole('link', { name: /Own Tab, opens in a new tab/ }).getAttribute('target') === '_blank', 'a new_tab app opens directly from the sidebar')
await shot(mp, '08-new-tab-app')

// Another site: Orbit is the top page, so its session cookie is sent and sign-in works. The browser
// blocks the app's own Lax cookie in the frame, so the app must not loop, and offers a new tab.
await mp.goto(`${ORBIT}/apps/cross-site`)
const crossFrame = mp.frameLocator('iframe[data-slot=app-frame]').frameLocator('iframe')
await crossFrame.locator('#cookie-blocked').waitFor({ timeout: 15000 })
check(true, "in a cross-site frame the app's own cookie is blocked, and the demo says so")
await shot(mp, '09-cross-site-frame-cookie-blocked')
const [popup] = await Promise.all([mp.context().waitForEvent('page'), crossFrame.getByRole('link', { name: /new tab/ }).click()])
await popup.locator('#who').waitFor({ timeout: 15000 })
check((await popup.locator('#who').textContent()).includes('Mia Member'), 'a top-level sign-in in a new tab works')
await popup.close()

// Instant Auth: the frame passes through the login's origin and Orbit, and each of those answers
// refuses frames on its redirect. The browser checks frame refusal only on the final page, so it works.
await mp.goto(`${ORBIT}/apps/behind-access`)
const instantFrame = mp.frameLocator('iframe[data-slot=app-frame]').frameLocator('iframe')
await instantFrame.locator('#who').waitFor({ timeout: 15000 })
check((await instantFrame.locator('#who').textContent()).includes('Mia Member'), 'Instant Auth signs in inside the frame, through redirects that refuse frames')
await shot(mp, '11-instant-auth-signed-in-in-frame')

// A login page that refuses frames: the frame fails, the popup signs in, the frame reloads.
await mp.goto(`${ORBIT}/apps/login-gate`)
const gateFrame = mp.frameLocator('iframe[data-slot=app-frame]').frameLocator('iframe')
await mp.waitForTimeout(2500)
check(await gateFrame.locator('#who').count() === 0 && await gateFrame.locator('#sign-in').count() === 0, 'the browser refuses the login page in the frame')
await shot(mp, '12-login-refused-in-frame')
const [gatePopup] = await Promise.all([mp.context().waitForEvent('page'), mp.getByRole('button', { name: 'Sign in to app' }).click()])
await gatePopup.locator('#sign-in').waitFor({ timeout: 15000 })
check(await gatePopup.evaluate(() => window.opener === null), 'the login page in the popup cannot reach Orbit through opener')
await shot(gatePopup, '13-popup-login-page')
await gatePopup.locator('#sign-in').click()
await gatePopup.locator('#who').waitFor({ timeout: 15000 })
await gatePopup.close()
await gateFrame.locator('#who').waitFor({ timeout: 15000 })
check((await gateFrame.locator('#who').textContent()).includes('Mia Member'), 'after the popup closes, the frame reloads and shows the signed-in person')
await shot(mp, '14-frame-signed-in-after-popup')

// COOP on the login page: the popup looks closed at once. The frame must still get there.
await mp.goto(`${ORBIT}/apps/gate-with-coop`)
const coopFrame = mp.frameLocator('iframe[data-slot=app-frame]').frameLocator('iframe')
const [coopPopup] = await Promise.all([mp.context().waitForEvent('page'), mp.getByRole('button', { name: 'Sign in to app' }).click()])
await coopPopup.locator('#sign-in').waitFor({ timeout: 15000 })
await mp.waitForTimeout(1500) // the early reload, if the browser reports the popup as closed
await coopPopup.locator('#sign-in').click()
await coopPopup.locator('#who').waitFor({ timeout: 15000 })
await coopPopup.close()
await mp.evaluate(() => window.dispatchEvent(new Event('focus'))) // back on the Orbit page
await coopFrame.locator('#who').waitFor({ timeout: 15000 })
check((await coopFrame.locator('#who').textContent()).includes('Mia Member'), 'with COOP on the login page, the return to Orbit reloads the frame')
await shot(mp, '15-frame-signed-in-after-coop-popup')
await mp.getByRole('button', { name: 'Reload' }).click()
await coopFrame.locator('#who').waitFor({ timeout: 15000 })
check(true, 'the manual Reload stays as the way out')

// A blocked popup.
await mp.evaluate(() => { window.open = () => null })
await mp.getByRole('button', { name: 'Sign in to app' }).click()
await mp.getByRole('alert').filter({ hasText: 'blocked the sign-in window' }).waitFor()
check(true, 'a blocked popup says what to do')
await shot(mp, '16-popup-blocked')

// Not signed in at all: to Orbit's login and back to the app.
const fresh = await browser.newContext({ viewport: { width: 1400, height: 900 } })
const fp = await fresh.newPage()
await fp.goto(`${same}/`)
await fp.waitForURL('**/login?**')
await fp.getByLabel('Email', { exact: true }).fill(emails.member)
await fp.getByLabel('Password', { exact: true }).fill(passwords.member)
await fp.getByRole('button', { name: 'Sign in', exact: true }).click()
await fp.locator('#who').waitFor({ timeout: 15000 })
check(new URL(fp.url()).origin === same, 'after the login Orbit returns to the app')
await shot(fp, '10-login-then-back-to-the-app')

// Agent tools link (only when the agent runs, so here only the owner's settings page shows it is there).
await browser.close()
console.log('all checks passed')
