import { expect, test } from 'bun:test'

const source = await Bun.file(new URL('./App.tsx', import.meta.url)).text()

test('terms and privacy stay public: outside the guest gate (signed-in people) and the auth gate (signed-out people)', () => {
  const guestGateEnd = source.indexOf('</Route>', source.indexOf('<Route element={<GuestGate />}>'))
  const authGate = source.indexOf('<Route element={<AuthGate />}>')
  for (const page of ['terms', 'privacy']) {
    const route = source.indexOf(`<Route path="${page}" element={<LegalPage page="${page}" />} />`)
    expect(route).toBeGreaterThan(guestGateEnd)
    expect(route).toBeLessThan(authGate)
  }
})
