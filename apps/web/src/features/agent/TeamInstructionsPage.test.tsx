import { afterEach, expect, mock, test } from 'bun:test'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import type { TeamInstructions } from '@orbit/api/agent'
import { WorkspaceContext } from '@/features/workspaces/workspaceContext'
import { testWorkspace } from '@/test/workspace'
import { TeamInstructionsPage } from './TeamInstructionsPage'

const originalFetch = globalThis.fetch
afterEach(() => {
  cleanup()
  globalThis.fetch = originalFetch
})

const saved: TeamInstructions = { instructions: 'Be brief.', can_edit: true, max_chars: 4000, updated_at: 1, updated_by_name: 'Ann' }

function serve(team: TeamInstructions) {
  const puts: unknown[] = []
  globalThis.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const request = new Request(input, init)
    const path = new URL(request.url).pathname
    if (path === '/api/v1/agent') return Response.json({ enabled: true, online: true, can_connect: true, bot_user_id: 'bot', name: 'Ada', chatgpt: { connected: false, invalid: false, account_id: null, connected_at: null }, login: null })
    if (request.method === 'PUT') {
      const body = (await request.json()) as { instructions: string }
      puts.push(body)
      return Response.json({ ...team, instructions: body.instructions.trim(), updated_at: 2 })
    }
    return Response.json(team)
  }) as unknown as typeof fetch
  return puts
}

function show(role: 'owner' | 'admin' | 'member') {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } })
  const workspace = testWorkspace(role)
  return render(<QueryClientProvider client={client}><WorkspaceContext.Provider value={{ workspace, workspaces: [workspace], selectWorkspace: () => {} }}><TeamInstructionsPage /></WorkspaceContext.Provider></QueryClientProvider>)
}

test('members read the team instructions and cannot change them', async () => {
  serve({ ...saved, can_edit: false })
  const view = show('member')
  expect(await view.findByText('Be brief.')).toBeTruthy()
  expect(view.getByText('Only owners and admins can change them.')).toBeTruthy()
  expect(view.queryAllByRole('textbox')).toHaveLength(0)
  expect(view.queryAllByRole('button', { name: 'Save instructions' })).toHaveLength(0)
})

test('admins save up to the limit, counted in characters as the server counts them', async () => {
  const puts = serve(saved)
  const view = show('admin')
  const field = (await view.findByLabelText('Team instructions')) as HTMLTextAreaElement
  const button = () => view.getByRole('button', { name: 'Save instructions' }) as HTMLButtonElement
  expect(button().disabled).toBe(true)

  // 4,000 two-unit characters fit; one more does not.
  fireEvent.change(field, { target: { value: '😀'.repeat(4000) } })
  expect(view.getByText(/4,000 \/ 4,000 characters/)).toBeTruthy()
  expect(button().disabled).toBe(false)
  fireEvent.change(field, { target: { value: 'a'.repeat(4001) } })
  expect(button().disabled).toBe(true)

  fireEvent.change(field, { target: { value: 'Answer in German. ' } })
  await userEvent.click(button())
  await waitFor(() => expect(puts).toEqual([{ instructions: 'Answer in German. ' }]))
  // The card shows what the server stored.
  await waitFor(() => expect((view.getByLabelText('Team instructions') as HTMLTextAreaElement).value).toBe('Answer in German.'))
})
