import { afterEach, expect, test } from 'bun:test'
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { AgentStatus } from '@orbit/api/agent'
import { AgentPage } from './AgentPage'

const realFetch = globalThis.fetch
afterEach(() => {
  cleanup()
  globalThis.fetch = realFetch
})

function serve(statuses: AgentStatus[]) {
  const calls: string[] = []
  globalThis.fetch = (async (input: Request | string) => {
    const request = input instanceof Request ? input : new Request(input)
    calls.push(`${request.method} ${new URL(request.url).pathname}`)
    if (request.method === 'POST') return Response.json({ login: { state: 'pending', user_code: 'ABCD-1234', verification_uri: 'https://auth.openai.com/codex/device' } })
    return Response.json(statuses.length > 1 ? statuses.shift() : statuses[0])
  }) as typeof fetch
  return calls
}

function show() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(<QueryClientProvider client={client}><AgentPage /></QueryClientProvider>)
}

const base = { enabled: true as const, online: true, can_connect: true, bot_user_id: 'bot', login: null }
const disconnected = { connected: false, invalid: false, account_id: null, connected_at: null }

test('connect shows the device code, then the connection', async () => {
  const calls = serve([
    { ...base, chatgpt: disconnected },
    { ...base, chatgpt: disconnected, login: { state: 'pending', user_code: 'ABCD-1234', verification_uri: 'https://auth.openai.com/codex/device' } },
    { ...base, chatgpt: { connected: true, invalid: false, account_id: 'acct', connected_at: 1 } },
  ])
  const view = show()
  fireEvent.click(await view.findByRole('button', { name: 'Connect ChatGPT' }))
  expect((await view.findByLabelText('Sign-in code')).textContent).toBe('ABCD-1234')
  await waitFor(() => expect(view.getByRole('button', { name: 'Disconnect ChatGPT' })).toBeTruthy(), { timeout: 5_000 })
  expect(calls).toContain('POST /api/v1/agent/chatgpt/login')
})

test('a failed refresh asks to connect again', async () => {
  serve([{ ...base, chatgpt: { connected: true, invalid: true, account_id: 'acct', connected_at: 1 } }])
  const view = show()
  expect(await view.findByText(/stopped working/)).toBeTruthy()
  expect(view.getAllByRole('button', { name: 'Connect ChatGPT' }).length).toBe(1)
})
