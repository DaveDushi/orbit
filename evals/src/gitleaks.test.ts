import { describe, expect, test } from 'bun:test'
import { spawnSync } from 'node:child_process'
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'

// The allowlist entries of evals/gitleaks.toml must hide only the triaged values in their own files: a planted
// secret next to them, or a triaged value in another file, is still reported.
const config = join(import.meta.dir, '../gitleaks.toml')
const hasGitleaks = spawnSync('gitleaks', ['version']).status === 0
const planted = 'Zk' + 'Q3vT9pLm2XcR8sWb4NyJ6hAe' // built at runtime, so this file is no finding itself

function scan(files: Record<string, string>): string[] {
  const dir = mkdtempSync(join(tmpdir(), 'evals-gitleaks-test-'))
  try {
    for (const [file, text] of Object.entries(files)) {
      mkdirSync(dirname(join(dir, file)), { recursive: true })
      writeFileSync(join(dir, file), text)
    }
    const report = join(dir, 'report.json')
    spawnSync('gitleaks', ['dir', '--config', config, '--redact', '--no-banner', '--report-format', 'json', '--report-path', report, '.'], { cwd: dir })
    return JSON.parse(readFileSync(report, 'utf8')).map((leak: any) => `${leak.File}:${leak.StartLine}`).sort()
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
}

describe.skipIf(!hasGitleaks)('gitleaks allowlist', () => {
  test('hides the triaged values in their files only', () => {
    // Built at runtime too: the triaged values are findings in this file.
    const websocketKey = 'Sec-WebSocket-Key: ' + ['dGhlIHNhbXBsZSBub25j', 'ZQ=='].join('')
    const hex = '0123456789abcdef'
    const evalValue = 'eval-secret-' + hex + hex
    const evalSecret = 'export const SECRET = "' + evalValue + '";'
    expect(
      scan({
        'apps/desktop/scripts/gateway-probe.sh': `key=(-H "${websocketKey}")\napi_key="${planted}"\n`,
        'evals/donna-codemode/fake-orbit.ts': `${evalSecret}\nexport const API_KEY = "${planted}";\n`,
        'apps/other/client.ts': `${evalSecret}\nconst probe = { "${websocketKey.replace(': ', '": "')}" };\n`,
        'target/debug/deps/libx.rmeta': `api_key="${planted}"\n`,
      }),
    ).toEqual(['apps/desktop/scripts/gateway-probe.sh:2', 'apps/other/client.ts:1', 'apps/other/client.ts:2', 'evals/donna-codemode/fake-orbit.ts:2'])
  })
})
