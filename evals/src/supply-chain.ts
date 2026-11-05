// Dependency advisories, licenses and a secret scan: cargo audit, cargo deny, bun audit (root workspace), npm audit
// (apps/agent), gitleaks (working tree, and git history unless --quick). Secrets are redacted by gitleaks and never
// copied: findings keep rule, file, line and commit only. Writes $EVALS_WORK/parts/supply-chain.json.
import { spawnSync } from 'node:child_process'
import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { quick, writePart } from './common'
import { check, type Check } from './headers'

const root = join(import.meta.dir, '../..')
const run = (command: string, args: string[], cwd = root) => spawnSync(command, args, { cwd, encoding: 'utf8', maxBuffer: 256 * 1024 * 1024, timeout: 15 * 60_000 })
type Severity = NonNullable<Check['severity']>
const findings: { tool: string; id: string; package?: string; title: string; severity: Severity; url?: string }[] = []
const checks: Check[] = []

/** Severity from a CVSS 3.x vector (base score), or `null` without one. */
function cvssSeverity(vector: string | null | undefined): Severity | null {
  if (!vector?.startsWith('CVSS:3')) return null
  const m = Object.fromEntries(vector.split('/').slice(1).map((part) => part.split(':')))
  const scopeChanged = m.S === 'C'
  const av = { N: 0.85, A: 0.62, L: 0.55, P: 0.2 }[m.AV as 'N'] ?? 0.85
  const ac = { L: 0.77, H: 0.44 }[m.AC as 'L'] ?? 0.77
  const pr = (scopeChanged ? { N: 0.85, L: 0.68, H: 0.5 } : { N: 0.85, L: 0.62, H: 0.27 })[m.PR as 'N'] ?? 0.85
  const ui = { N: 0.85, R: 0.62 }[m.UI as 'N'] ?? 0.85
  const cia = (key: string) => ({ H: 0.56, L: 0.22, N: 0 })[m[key] as 'H'] ?? 0
  const iss = 1 - (1 - cia('C')) * (1 - cia('I')) * (1 - cia('A'))
  const impact = scopeChanged ? 7.52 * (iss - 0.029) - 3.25 * (iss - 0.02) ** 15 : 6.42 * iss
  const exploitability = 8.22 * av * ac * pr * ui
  const roundUp = (value: number) => Math.ceil(value * 10) / 10
  const score = impact <= 0 ? 0 : roundUp(Math.min(scopeChanged ? 1.08 * (impact + exploitability) : impact + exploitability, 10))
  return score >= 9 ? 'critical' : score >= 7 ? 'high' : score >= 4 ? 'medium' : score > 0 ? 'low' : 'info'
}
const npmSeverity = (value: string): Severity => (value === 'moderate' ? 'medium' : (value as Severity))

// cargo audit
{
  const result = run('cargo', ['audit', '--json'])
  try {
    const report = JSON.parse(result.stdout)
    for (const item of report.vulnerabilities?.list ?? []) {
      findings.push({ tool: 'cargo-audit', id: item.advisory.id, package: `${item.package.name}@${item.package.version}`, title: item.advisory.title, severity: cvssSeverity(item.advisory.cvss) ?? 'medium', url: `https://rustsec.org/advisories/${item.advisory.id}` })
    }
    for (const [kind, list] of Object.entries<any[]>(report.warnings ?? {})) {
      for (const item of list) findings.push({ tool: 'cargo-audit', id: item.advisory?.id ?? kind, package: `${item.package.name}@${item.package.version}`, title: `${kind}: ${item.advisory?.title ?? ''}`.trim(), severity: kind === 'unsound' ? 'low' : 'info', url: item.advisory ? `https://rustsec.org/advisories/${item.advisory.id}` : undefined })
    }
    checks.push(check('deps.cargo-audit', 'dependencies', 'cargo audit: no vulnerable Rust crates', (report.vulnerabilities?.count ?? 0) === 0, 'high', [`${report.vulnerabilities?.count ?? 0} vulnerabilities, ${Object.values<any[]>(report.warnings ?? {}).flat().length} warnings, ${report.lockfile?.['dependency-count']} crates`]))
  } catch {
    checks.push(check('deps.cargo-audit', 'dependencies', 'cargo audit ran', 'skip', 'info', [result.stderr.trim().split('\n').slice(-3).join(' ')]))
  }
}

// cargo deny (advisories are covered above; licenses, bans, sources here, and advisories as a cross-check)
{
  const result = run('cargo', ['deny', '--format', 'json', '--config', 'evals/deny.toml', 'check', 'advisories', 'licenses', 'bans', 'sources'])
  const counts: Record<string, { error: number; warning: number }> = {}
  const errors: string[] = []
  for (const text of result.stderr.split('\n')) {
    let entry: any
    try {
      entry = JSON.parse(text)
    } catch {
      continue
    }
    if (entry.type !== 'diagnostic') continue
    const { severity, code, message } = entry.fields
    const kind = String(code ?? 'other')
    const bucket = (counts[kind] ??= { error: 0, warning: 0 })
    if (severity === 'error') {
      bucket.error++
      const crate = entry.fields.graphs?.[0]?.Krate
      errors.push(`${kind}: ${message}${crate ? ` (${crate.name}@${crate.version})` : ''}`)
    } else if (severity === 'warning') bucket.warning++
  }
  const total = Object.values(counts).reduce((sum, bucket) => sum + bucket.error, 0)
  const licenseErrors = errors.filter((text) => /license|rejected|unlicensed/i.test(text))
  const otherErrors = errors.filter((text) => !licenseErrors.includes(text) && !/^(vulnerability|unmaintained|unsound|notice|yanked)/.test(text))
  checks.push(check('deps.cargo-deny-licenses', 'dependencies', 'cargo deny: all crate licenses on the allow list', licenseErrors.length === 0, 'low', licenseErrors.length ? licenseErrors.slice(0, 15) : ['ok'], 'Review the license, then allow it in evals/deny.toml or replace the crate.'))
  checks.push(check('deps.cargo-deny-bans-sources', 'dependencies', 'cargo deny: no banned crates or unknown sources', otherErrors.length === 0, 'medium', otherErrors.length ? otherErrors.slice(0, 15) : ['ok']))
  checks.push(check('deps.cargo-deny-summary', 'dependencies', 'cargo deny ran', result.status === null ? 'skip' : true, 'info', [`errors ${total}; by code: ${Object.entries(counts).map(([code, bucket]) => `${code} ${bucket.error}e/${bucket.warning}w`).join(', ') || 'none'}`]))
}

// bun audit (root workspace: web, mobile, packages)
{
  const result = run('bun', ['audit', '--json'])
  try {
    const report = JSON.parse(result.stdout || '{}')
    for (const [name, advisories] of Object.entries<any[]>(report)) {
      for (const advisory of advisories) findings.push({ tool: 'bun-audit', id: advisory.url?.split('/').pop() ?? String(advisory.id), package: name, title: advisory.title, severity: npmSeverity(advisory.severity), url: advisory.url })
    }
    const high = findings.filter((item) => item.tool === 'bun-audit' && ['high', 'critical'].includes(item.severity))
    checks.push(check('deps.bun-audit', 'dependencies', 'bun audit: no high/critical advisories in the web/mobile workspace', high.length === 0, 'high', [`${findings.filter((item) => item.tool === 'bun-audit').length} advisories, ${high.length} high/critical`, ...high.map((item) => `${item.package}: ${item.title}`)]))
  } catch {
    checks.push(check('deps.bun-audit', 'dependencies', 'bun audit ran', 'skip', 'info', [result.stderr.trim().slice(0, 200)]))
  }
}

// npm audit (apps/agent has its own package-lock.json)
{
  const result = run('npm', ['audit', '--package-lock-only', '--json'], join(root, 'apps/agent'))
  try {
    const report = JSON.parse(result.stdout)
    for (const [name, item] of Object.entries<any>(report.vulnerabilities ?? {})) {
      const advisory = item.via.find((via: unknown) => typeof via === 'object')
      findings.push({ tool: 'npm-audit-agent', id: advisory?.url?.split('/').pop() ?? name, package: name, title: advisory?.title ?? `via ${item.via.join(', ')}`, severity: npmSeverity(item.severity), url: advisory?.url })
    }
    const counts = report.metadata?.vulnerabilities ?? {}
    checks.push(check('deps.npm-audit-agent', 'dependencies', 'npm audit (apps/agent): no high/critical advisories', (counts.high ?? 0) + (counts.critical ?? 0) === 0, 'high', [JSON.stringify(counts)]))
  } catch {
    checks.push(check('deps.npm-audit-agent', 'dependencies', 'npm audit (apps/agent) ran', 'skip', 'info', [result.stderr.trim().slice(0, 200) || 'npm not found']))
  }
}

// gitleaks: working tree, and the whole history unless --quick
{
  const dir = mkdtempSync(join(tmpdir(), 'evals-gitleaks-'))
  const scans: [string, string[]][] = [['tree', ['dir', '--config', 'evals/gitleaks.toml', '--redact', '--no-banner', '--report-format', 'json', '--report-path', join(dir, 'tree.json'), '.']]]
  if (!quick) scans.push(['history', ['git', '--config', 'evals/gitleaks.toml', '--redact', '--no-banner', '--report-format', 'json', '--report-path', join(dir, 'history.json'), '.']])
  for (const [scope, args] of scans) {
    const result = run('gitleaks', args)
    let leaks: any[] = []
    try {
      leaks = JSON.parse(readFileSync(join(dir, `${scope}.json`), 'utf8'))
    } catch {
      checks.push(check(`secrets.${scope}`, 'secrets', `gitleaks ${scope} scan ran`, 'skip', 'info', [result.stderr.trim().split('\n').slice(-2).join(' ')]))
      continue
    }
    // Ignore what is not ours to fix: vendored references and local scratch.
    leaks = leaks.filter((leak) => !/^(references|shadow|node_modules|target)\//.test(leak.File))
    const unique = new Map<string, any>()
    for (const leak of leaks) unique.set(`${leak.RuleID} ${leak.File}:${leak.StartLine}`, leak)
    const listed = [...unique.values()].map((leak) => {
      // Test fixtures, docs and examples hold sample values; real code or config is what matters.
      const sample = /(^|\/)(tests?|fixtures?|e2e|docs?|examples?|__tests__)\/|\.test\.|\.spec\.|\.md$/.test(leak.File)
      return { rule: leak.RuleID, file: leak.File, line: leak.StartLine, commit: leak.Commit ? leak.Commit.slice(0, 10) : undefined, severity: (sample ? 'low' : 'high') as Severity }
    })
    for (const leak of listed) findings.push({ tool: `gitleaks-${scope}`, id: `${leak.rule}:${leak.file}:${leak.line}`, title: `possible secret (${leak.rule}) in ${leak.file}:${leak.line}${leak.commit ? ` @${leak.commit}` : ''}`, severity: leak.severity })
    const serious = listed.filter((leak) => leak.severity === 'high')
    checks.push(check(`secrets.${scope}`, 'secrets', `gitleaks ${scope}: no secrets outside tests and docs`, serious.length === 0, 'high', [`${listed.length} findings (${serious.length} outside tests/docs), secrets redacted`, ...serious.slice(0, 15).map((leak) => `${leak.rule} ${leak.file}:${leak.line}${leak.commit ? ` @${leak.commit}` : ''}`)], 'Rotate the secret, then remove it or add a gitleaks allow entry if it is a public test value.'))
  }
  rmSync(dir, { recursive: true, force: true })
}

writePart('supply-chain', { checks, findings })
console.log(`supply-chain: ${findings.length} findings, ${checks.filter((item) => item.result === 'fail').length} failed checks`)
