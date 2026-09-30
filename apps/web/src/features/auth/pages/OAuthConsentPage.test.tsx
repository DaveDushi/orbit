import { afterEach, expect, test } from 'bun:test'
import { useEffect } from 'react'
import { render, waitFor } from '@testing-library/react'
import { Route, Routes, useLocation } from 'react-router'
import { Providers } from '@/app/Providers'
import { OAuthConsentPage } from './OAuthConsentPage'

const originalFetch = globalThis.fetch
afterEach(() => { globalThis.fetch = originalFetch; window.history.replaceState(null, '', '/') })

test('anonymous OAuth consent keeps its return path despite the global session handler', async () => {
  const request = 'a'.repeat(43)
  window.history.replaceState(null, '', `/oauth/consent?request=${request}`)
  globalThis.fetch = Object.assign(async () => Response.json({ type: 'about:blank', title: 'Sign in', code: 'authentication_required', status: 401, detail: 'A valid session is required.', instance: '/api/v1/auth/me', request_id: 'test' }, { status: 401, headers: { 'content-type': 'application/problem+json' } }), { preconnect: originalFetch.preconnect })
  let destination: { path: string; search: string } | undefined
  function LocationState() {
    const location = useLocation()
    useEffect(() => { destination = { path: location.pathname, search: location.search } }, [location])
    return null
  }
  render(<Providers><LocationState /><Routes>
    <Route path="/oauth/consent" element={<OAuthConsentPage />} />
    <Route path="/login" element={null} />
  </Routes></Providers>)
  await waitFor(() => expect(destination?.path).toBe('/login'))
  expect(new URLSearchParams(destination?.search).get('return_to')).toBe(`/oauth/consent?request=${request}`)
})
