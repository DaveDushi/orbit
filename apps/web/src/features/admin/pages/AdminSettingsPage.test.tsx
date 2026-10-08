import { afterEach, expect, test } from 'bun:test'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { AdminSettingsPage } from './AdminSettingsPage'

const originalFetch = globalThis.fetch
afterEach(() => { globalThis.fetch = originalFetch })

const SMTP = { host: 'smtp.example.com', port: 587, security: 'starttls', username: 'orbit', password_set: true, from_address: 'orbit@example.com', from_name: 'Orbit' }

test('saving with an empty password field keeps the saved password, and registration needs a mail server', async () => {
  const bodies: unknown[] = []
  let settings: object = { registration_open: false, smtp: null }
  globalThis.fetch = (async (input: RequestInfo | URL) => {
    const request = input as Request
    if (request.method === 'PUT') {
      const body = await request.json()
      bodies.push(body)
      settings = { registration_open: false, smtp: SMTP }
    }
    return Response.json(settings)
  }) as unknown as typeof fetch
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const view = render(<QueryClientProvider client={client}><AdminSettingsPage /></QueryClientProvider>)

  const toggle = await view.findByRole('switch', { name: 'Open registration' })
  expect(toggle.getAttribute('aria-disabled') === 'true' || toggle.hasAttribute('data-disabled')).toBe(true)

  await userEvent.type(view.getByLabelText(/^Host/), 'smtp.example.com')
  await userEvent.type(view.getByLabelText(/^From address/), 'orbit@example.com')
  await userEvent.click(view.getByRole('button', { name: 'Save mail settings' }))
  await waitFor(() => expect(bodies).toHaveLength(1))
  expect(bodies[0]).toEqual({ host: 'smtp.example.com', port: 587, security: 'starttls', username: null, from_address: 'orbit@example.com', from_name: null })
  expect(await view.findByRole('button', { name: 'Send test email' })).toBeTruthy()
})
