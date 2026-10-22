import { afterEach, expect, mock, spyOn, test } from 'bun:test'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { MemoryRouter } from 'react-router'
import type { BillingView } from '@/api/generated/types.gen'
import { WorkspaceContext } from '@/features/workspaces/workspaceContext'
import { testWorkspace } from '@/test/workspace'
import { BillingPage } from './BillingPage'

const originalFetch = globalThis.fetch

afterEach(() => {
  globalThis.fetch = originalFetch
})

const readOnly: BillingView = {
  plan: null,
  status: null,
  access: 'read_only',
  cancel_at_period_end: false,
  current_period_end: null,
  plans: ['team', 'business'],
  has_customer: false,
  can_manage: true,
}

function serve(billing: BillingView, requests: Request[] = []) {
  globalThis.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const request = new Request(input, init)
    requests.push(request)
    const path = new URL(request.url).pathname
    if (path === '/api/v1/auth/options') {
      return Response.json({ registration_open: false, email_enabled: false, passkeys_enabled: false, billing_enabled: true })
    }
    if (path.endsWith('/billing/checkout')) return Response.json({ url: 'https://checkout.stripe.test/cs' })
    if (path.endsWith('/billing/portal')) return Response.json({ url: 'https://billing.stripe.test/p' })
    if (path.endsWith('/billing')) return Response.json(billing)
    return Response.json({}, { status: 404 })
  }) as unknown as typeof fetch
}

function renderPage(role: 'owner' | 'member') {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } })
  const workspace = testWorkspace(role)
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={['/settings/billing']}>
        <WorkspaceContext.Provider value={{ workspace, workspaces: [workspace], selectWorkspace: () => {} }}>
          <BillingPage />
        </WorkspaceContext.Provider>
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

test('the owner of a read-only workspace picks a plan and goes to Stripe Checkout', async () => {
  const requests: Request[] = []
  serve(readOnly, requests)
  const assign = spyOn(window.location, 'assign').mockImplementation(() => {})
  const view = renderPage('owner')

  await waitFor(() => expect(view.getByText('Read-only')).toBeTruthy())
  expect(view.getByText('Not subscribed')).toBeTruthy()
  expect(view.queryAllByRole('button', { name: 'Manage billing' })).toHaveLength(0)
  await userEvent.click(view.getByRole('button', { name: 'Business' }))

  await waitFor(() => expect(assign).toHaveBeenCalledWith('https://checkout.stripe.test/cs'))
  const checkout = requests.find((request) => request.method === 'POST')!
  expect(new URL(checkout.url).pathname).toBe(`/api/v1/workspaces/${testWorkspace('owner').id}/billing/checkout`)
  expect(await checkout.json()).toEqual({ plan: 'business' })
  assign.mockRestore()
})

test('a subscribed owner sees the renewal date and opens the billing portal', async () => {
  serve({ ...readOnly, plan: 'team', status: 'active', access: 'full', has_customer: true, current_period_end: '2030-03-17T17:46:40.000Z' })
  const assign = spyOn(window.location, 'assign').mockImplementation(() => {})
  const view = renderPage('owner')

  await waitFor(() => expect(view.getByText('Full access')).toBeTruthy())
  expect(view.getByText('Team')).toBeTruthy()
  expect(view.getByText('Active')).toBeTruthy()
  expect(view.getByText(/^Renews on /)).toBeTruthy()
  expect(view.queryAllByRole('button', { name: 'Team' })).toHaveLength(0)
  await userEvent.click(view.getByRole('button', { name: 'Manage billing' }))
  await waitFor(() => expect(assign).toHaveBeenCalledWith('https://billing.stripe.test/p'))
  assign.mockRestore()
})

test('members see the status but cannot choose a plan', async () => {
  serve({ ...readOnly, can_manage: false, has_customer: true })
  const view = renderPage('member')

  await waitFor(() => expect(view.getByText('Only the workspace owner can choose a plan.')).toBeTruthy())
  expect(view.queryAllByRole('button')).toHaveLength(0)
})
