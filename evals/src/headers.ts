// Checks of security response headers, the CSP and cookie flags. Used for production (read-only GETs) and for the
// local test instance.

export type Check = {
  id: string
  category: string
  title: string
  result: 'pass' | 'fail' | 'warn' | 'skip'
  severity?: 'critical' | 'high' | 'medium' | 'low' | 'info'
  evidence: string[]
  fix?: string
}

export function check(id: string, category: string, title: string, ok: boolean | 'warn' | 'skip', severity: Check['severity'], evidence: string[], fix?: string): Check {
  const result = ok === true ? 'pass' : ok === false ? 'fail' : ok
  return { id, category, title, result, ...(result === 'pass' ? {} : { severity, fix }), evidence }
}

/** Directive name -> sources of a CSP header. */
export function parseCsp(header: string): Map<string, string[]> {
  const directives = new Map<string, string[]>()
  for (const part of header.split(';')) {
    const [name, ...values] = part.trim().split(/\s+/)
    if (name) directives.set(name.toLowerCase(), values)
  }
  return directives
}

/** Checks of one HTML document response: `label` names the target in the check ids. */
export function documentHeaderChecks(label: string, url: string, headers: Headers, https: boolean): Check[] {
  const evidence = (name: string) => [`GET ${url} -> ${name}: ${headers.get(name) ?? '(missing)'}`]
  const checks: Check[] = []
  const csp = headers.get('content-security-policy')
  checks.push(check(`${label}.csp-present`, 'headers', 'Content-Security-Policy is set', !!csp, 'high', evidence('content-security-policy'), 'Send a CSP on every document.'))
  if (csp) {
    const directives = parseCsp(csp)
    const scriptSources = directives.get('script-src') ?? directives.get('default-src') ?? []
    const styleSources = directives.get('style-src') ?? directives.get('default-src') ?? []
    checks.push(check(`${label}.csp-script-unsafe`, 'csp', "script-src has no 'unsafe-inline' or 'unsafe-eval'", !scriptSources.some((s) => /unsafe-(inline|eval)/.test(s)), 'high', [`script-src ${scriptSources.join(' ')}`], 'Use hashes or nonces instead.'))
    // 'wasm-unsafe-eval' is narrower than 'unsafe-eval' (WebAssembly compile only); it is reported, not failed.
    if (scriptSources.includes("'wasm-unsafe-eval'")) checks.push(check(`${label}.csp-wasm-eval`, 'csp', "script-src allows 'wasm-unsafe-eval'", 'warn', 'info', [`script-src ${scriptSources.join(' ')}`], 'Keep it only if a WebAssembly feature needs it.'))
    checks.push(check(`${label}.csp-style-unsafe`, 'csp', "style-src has no 'unsafe-inline'", !styleSources.includes("'unsafe-inline'"), 'low', [`style-src ${styleSources.join(' ')}`], 'Move inline styles to the stylesheet.'))
    // A wildcard in a fetch or script directive is worse than one in img-src/media-src (tracking pixels only).
    const wide = (source: string) => source === '*' || /^(https?:|data:|blob:)?\*$/.test(source) || source === 'https:' || source === 'http:'
    const wildcards = [...directives].flatMap(([name, sources]) => sources.filter(wide).map((source) => `${name} ${source}`))
    const serious = wildcards.some((entry) => !/^(img-src|media-src) /.test(entry))
    checks.push(check(`${label}.csp-wildcards`, 'csp', 'CSP has no wildcard or scheme-only sources', wildcards.length === 0, serious ? 'medium' : 'low', [`wildcard sources: ${wildcards.join(', ') || 'none'}`], 'Name the hosts (or proxy remote images through the server).'))
    const frameAncestors = directives.get('frame-ancestors')
    const xfo = headers.get('x-frame-options')
    checks.push(check(`${label}.clickjacking`, 'headers', "frame-ancestors 'none'/'self' or X-Frame-Options DENY/SAMEORIGIN", !!frameAncestors?.every((s) => ["'none'", "'self'"].includes(s)) || /deny|sameorigin/i.test(xfo ?? ''), 'medium', [`frame-ancestors ${frameAncestors?.join(' ') ?? '(missing)'}`, `x-frame-options ${xfo ?? '(missing)'}`]))
    const objectSources = directives.get('object-src') ?? directives.get('default-src') ?? []
    checks.push(check(`${label}.csp-object`, 'csp', "object-src is 'none'", objectSources.join(' ') === "'none'", 'low', [`object-src ${objectSources.join(' ') || '(missing)'}`]))
    checks.push(check(`${label}.csp-base-uri`, 'csp', 'base-uri is restricted', !!directives.get('base-uri')?.every((s) => ["'none'", "'self'"].includes(s)), 'low', [`base-uri ${directives.get('base-uri')?.join(' ') ?? '(missing)'}`]))
  }
  checks.push(check(`${label}.nosniff`, 'headers', 'X-Content-Type-Options: nosniff', headers.get('x-content-type-options')?.toLowerCase() === 'nosniff', 'medium', evidence('x-content-type-options')))
  checks.push(check(`${label}.referrer-policy`, 'headers', 'Referrer-Policy is strict', /^(no-referrer|same-origin|strict-origin|strict-origin-when-cross-origin)$/i.test(headers.get('referrer-policy') ?? ''), 'low', evidence('referrer-policy')))
  checks.push(check(`${label}.permissions-policy`, 'headers', 'Permissions-Policy is set', !!headers.get('permissions-policy'), 'low', evidence('permissions-policy')))
  checks.push(check(`${label}.coop`, 'headers', 'Cross-Origin-Opener-Policy is set', /same-origin/.test(headers.get('cross-origin-opener-policy') ?? ''), 'low', evidence('cross-origin-opener-policy')))
  checks.push(check(`${label}.corp`, 'headers', 'Cross-Origin-Resource-Policy is set', /same-(origin|site)/.test(headers.get('cross-origin-resource-policy') ?? ''), 'low', evidence('cross-origin-resource-policy'), 'Add Cross-Origin-Resource-Policy: same-origin (or same-site) to responses.'))
  if (https) {
    const hsts = headers.get('strict-transport-security') ?? ''
    const maxAge = Number(hsts.match(/max-age=(\d+)/)?.[1] ?? 0)
    checks.push(check(`${label}.hsts`, 'headers', 'HSTS max-age >= 180 days', maxAge >= 15552000, 'medium', evidence('strict-transport-security')))
  }
  checks.push(check(`${label}.server-banner`, 'headers', 'No version in Server / X-Powered-By', !/\d/.test(headers.get('server') ?? '') && !headers.get('x-powered-by'), 'info', [`server: ${headers.get('server') ?? '(none)'}`, `x-powered-by: ${headers.get('x-powered-by') ?? '(none)'}`]))
  return checks
}

/** Flags of every cookie a response sets (names only, values never leave this function). */
export function cookieChecks(label: string, url: string, setCookies: string[], https: boolean): Check[] {
  return setCookies.map((cookie) => {
    const [pair, ...attributes] = cookie.split(';').map((part) => part.trim())
    const name = pair.split('=')[0]
    const flags = attributes.map((attribute) => attribute.toLowerCase())
    const sameSite = flags.find((flag) => flag.startsWith('samesite='))?.split('=')[1]
    const missing = [
      https && !flags.includes('secure') ? 'Secure' : '',
      !flags.includes('httponly') ? 'HttpOnly' : '',
      !sameSite || sameSite === 'none' ? 'SameSite=Lax/Strict' : '',
    ].filter(Boolean)
    return check(`${label}.cookie.${name}`, 'cookies', `Cookie ${name} has Secure, HttpOnly, SameSite`, missing.length === 0, 'medium', [`${url} sets ${name}; ${attributes.map((a) => a.split('=')[0] + (a.toLowerCase().startsWith('samesite') ? `=${a.split('=')[1]}` : '')).join('; ')}`, missing.length ? `missing: ${missing.join(', ')}` : 'all flags set'])
  })
}
