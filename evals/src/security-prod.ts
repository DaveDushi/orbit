// Read-only checks of the production site: headers, CSP, cookie flags, TLS, and console errors of the login page.
// Sends only GET/HEAD requests and a TLS handshake; the browser aborts any other method. No login.
// If an access gate (exe.dev login proxy) answers instead of Orbit, Orbit's own page checks are skipped: the local
// production-mode instance covers them (security-local.ts). Usage: bun evals/src/security-prod.ts [site].
// Writes $EVALS_WORK/parts/security-prod.json.
import { spawnSync } from 'node:child_process'
import { chromium } from 'playwright-core'
import { writePart } from './common'
import { check, cookieChecks, documentHeaderChecks, type Check } from './headers'

const site = (process.argv[2] ?? 'https://orbit.wellrox.com').replace(/\/$/, '')
const host = new URL(site).host
const checks: Check[] = []

const get = async (url: string, redirect: RequestRedirect = 'manual') => {
  const response = await fetch(url.startsWith('http') ? url : site + url, { method: 'GET', redirect, headers: { 'user-agent': 'orbit-evals (read-only header check)' } })
  await response.arrayBuffer()
  return response
}

const first = await get('/')
const location = first.headers.get('location') ?? ''
const gate = first.status >= 300 && first.status < 400 && /\/__[a-z.]+\/login/.test(location) ? new URL(location, site).pathname : null

// The edge: HSTS on the first answer, http -> https, TLS.
{
  const hsts = first.headers.get('strict-transport-security') ?? ''
  const maxAge = Number(hsts.match(/max-age=(\d+)/)?.[1] ?? 0)
  checks.push(check('prod.hsts', 'headers', 'HSTS max-age >= 180 days on the first answer', maxAge >= 15552000, 'medium', [`GET / -> ${first.status}, strict-transport-security: ${hsts || '(missing)'}`]))
  const response = await fetch(`http://${host}/`, { method: 'HEAD', redirect: 'manual' }).catch(() => null)
  const target = response?.headers.get('location') ?? ''
  checks.push(check('prod.http-redirect', 'tls', 'http:// redirects to https://', !response || (response.status >= 300 && response.status < 400 && target.startsWith('https://')), 'medium', [`HEAD http://${host}/ -> ${response?.status ?? 'no answer (port 80 closed)'} ${target}`]))
}
const handshake = (flag: string) =>
  spawnSync('openssl', ['s_client', '-connect', `${host}:443`, '-servername', host, flag, '-brief'], { input: '', encoding: 'utf8', timeout: 15_000 })
const modern = handshake('-tls1_3')
const protocol = `${modern.stdout}${modern.stderr}`.match(/Protocol version: (\S+)/)?.[1] ?? 'unknown'
checks.push(check('prod.tls13', 'tls', 'TLS 1.3 is offered', protocol === 'TLSv1.3', 'low', [`openssl s_client -tls1_3 -> ${protocol}`]))
for (const flag of ['-tls1', '-tls1_1']) {
  const old = handshake(flag)
  const accepted = /Protocol version: TLSv1(\.1)?\b/.test(`${old.stdout}${old.stderr}`)
  checks.push(check(`prod.tls-old${flag}`, 'tls', `${flag.slice(1)} is refused`, !accepted, 'medium', [`openssl s_client ${flag} -> ${accepted ? 'accepted' : 'refused'}`]))
}

if (gate) {
  // Orbit is not reachable without signing in at the gate, which these checks must not do.
  const gatePage = await get(location.startsWith('http') ? location : site + location, 'follow')
  const missing = documentHeaderChecks('prod.gate', gate, gatePage.headers, true).filter((item) => item.result !== 'pass')
  checks.push(check('prod.gated', 'headers', 'Orbit pages are behind an access gate; Orbit header checks are skipped (see local.*)', 'skip', 'info', [`GET / -> ${first.status} to ${gate}`]))
  checks.push(check('prod.gate-headers', 'headers', `Access gate page (${gate}) sends the security headers`, missing.length === 0 ? true : 'warn', 'info', missing.length ? missing.map((item) => `${item.title}: ${item.evidence.join('; ')}`) : ['all present'], 'Gate is the hosting proxy, not Orbit: raise it with the host if the gate stays public.'))
  checks.push(...cookieChecks('prod.gate', `GET ${gate}`, [...first.headers.getSetCookie(), ...gatePage.headers.getSetCookie()], true))
} else {
  for (const path of ['/', '/login']) {
    const response = await get(path)
    checks.push(...documentHeaderChecks(`prod${path === '/' ? '.index' : path.replaceAll('/', '.')}`, path, response.headers, true))
    checks.push(...cookieChecks(`prod${path.replaceAll('/', '.')}`, `GET ${path}`, response.headers.getSetCookie(), true))
  }
  for (const path of ['/api/v1/auth/options', '/api/v1/setup/status', '/health/live']) {
    const response = await get(path)
    checks.push(check(`prod.api${path.replaceAll('/', '.')}.nosniff`, 'headers', `${path} sends nosniff`, response.headers.get('x-content-type-options') === 'nosniff', 'low', [`GET ${path} -> ${response.status}`]))
    checks.push(...cookieChecks(`prod.api${path.replaceAll('/', '.')}`, `GET ${path}`, response.headers.getSetCookie(), true))
  }
  // Console errors (CSP and others) on the login page, with every non-GET/HEAD request aborted.
  const browser = await chromium.launch({ executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE || undefined })
  const page = await browser.newPage()
  const errors: string[] = []
  const blocked: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text().slice(0, 240))
  })
  page.on('pageerror', (error) => errors.push(`pageerror: ${error.message.slice(0, 240)}`))
  await page.route('**/*', (route) => {
    const method = route.request().method()
    if (method === 'GET' || method === 'HEAD') return route.continue()
    blocked.push(`${method} ${new URL(route.request().url()).pathname}`)
    return route.abort()
  })
  await page.goto(`${site}/login`, { waitUntil: 'networkidle', timeout: 60_000 })
  await page.waitForTimeout(2000)
  await browser.close()
  const csp = errors.filter((text) => /Content Security Policy|Refused to/i.test(text))
  checks.push(check('prod.login-console-csp', 'csp', 'Login page has no CSP console errors', csp.length === 0, 'medium', csp.length ? csp : ['none']))
  checks.push(check('prod.login-console', 'errors', 'Login page has no other console errors', errors.length === csp.length ? true : 'warn', 'low', errors.filter((text) => !csp.includes(text)).concat(blocked.length ? [`blocked non-GET requests: ${blocked.join(', ')}`] : [])))
}

writePart('security-prod', { site, gated: gate, tls_protocol: protocol, checks })
console.log(`security-prod: ${checks.length} checks, ${checks.filter((item) => item.result === 'fail').length} failed${gate ? ' (behind an access gate)' : ''}`)
