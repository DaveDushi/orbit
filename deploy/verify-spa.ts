// Loads the live web app in headless Chromium and fails on a page error, a console error or a blank page at load.
// Health checks and curl do not run the app's JavaScript, so a bundle that breaks at load passes them.
// Run from the repo root: bun deploy/verify-spa.ts (verify-production.sh runs it when ORBIT_VERIFY_SPA=1).
//
// ORBIT_VERIFY_ORIGIN               public origin, e.g. https://orbit.example.com
// ORBIT_VERIFY_SESSION_TOKEN_FILE   optional: a session token; also checks signed-in pages, then logs out (revokes it)
// ORBIT_VERIFY_CONNECT              optional host:port to connect to instead of DNS (a local TLS hop); accepts its cert
// ORBIT_VERIFY_HEADER               optional extra request header "Name: value" (e.g. for a proxy in front)
// PLAYWRIGHT_CHROMIUM_EXECUTABLE    optional Chromium binary
import { chromium, type Page } from 'playwright-core'

const origin = process.env.ORBIT_VERIFY_ORIGIN?.replace(/\/$/, '')
if (!origin) throw new Error('set ORBIT_VERIFY_ORIGIN')
const host = new URL(origin).hostname
const connect = process.env.ORBIT_VERIFY_CONNECT
const tokenFile = process.env.ORBIT_VERIFY_SESSION_TOKEN_FILE
const header = process.env.ORBIT_VERIFY_HEADER?.match(/^([^:]+):\s*(.*)$/)

const browser = await chromium.launch({
  executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE || undefined,
  args: connect ? [`--host-resolver-rules=MAP ${host}:443 ${connect}`] : [],
})
const context = await browser.newContext({
  ignoreHTTPSErrors: Boolean(connect),
  // A service worker fetch does not get the extra header, and the check is about the app's own code.
  serviceWorkers: 'block',
  extraHTTPHeaders: header ? { [header[1]!]: header[2]! } : undefined,
})

let failed = 0
async function check(page: Page, path: string, signedIn: boolean) {
  // Signed out, the app asks for the current session and gets the expected 401.
  const allowed = signedIn ? null : /status of 401/
  const errors: string[] = []
  const onPageError = (e: Error) => errors.push(`page error: ${e.message}`)
  const onConsole = (m: { type(): string; text(): string }) => {
    if (m.type() === 'error' && !allowed?.test(m.text())) errors.push(`console error: ${m.text()}`)
  }
  page.on('pageerror', onPageError)
  page.on('console', onConsole)
  try {
    await page.goto(origin + path, { waitUntil: 'networkidle', timeout: 30_000 })
    await page.waitForTimeout(1_000)
    if (!(await page.locator('body').innerText()).trim()) errors.push('blank page')
    if (signedIn && new URL(page.url()).pathname.startsWith('/login')) errors.push('sent to /login: the session is not valid')
  } catch (e) {
    errors.push(`load: ${(e as Error).message.split('\n')[0]}`)
  }
  page.off('pageerror', onPageError)
  page.off('console', onConsole)
  const label = `${signedIn ? 'signed in' : 'signed out'} ${path} -> ${new URL(page.url()).pathname}`
  if (errors.length) {
    failed++
    console.error(`FAIL: ${label}\n  ${errors.join('\n  ')}`)
  } else {
    console.log(`ok  ${label}`)
  }
}

try {
  await check(await context.newPage(), '/', false)
  if (tokenFile) {
    const token = (await Bun.file(tokenFile).text()).trim()
    await context.clearCookies()
    await context.addCookies([
      { name: '__Host-orbit_session', value: token, domain: host, path: '/', secure: true, httpOnly: true, sameSite: 'Lax' },
    ])
    const page = await context.newPage()
    try {
      for (const path of ['/', '/tasks']) await check(page, path, true)
    } finally {
      const status = await page.evaluate(async () => (await fetch('/api/v1/auth/logout', { method: 'POST' })).status)
      if (status !== 204) {
        failed++
        console.error(`FAIL: logout returned ${status}; revoke the session by hand`)
      } else {
        console.log('ok  test session logged out (revoked)')
      }
    }
  }
} finally {
  await browser.close()
}
process.exit(failed ? 1 : 0)
