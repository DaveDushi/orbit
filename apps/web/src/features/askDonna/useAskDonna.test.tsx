import { afterEach, expect, test } from 'bun:test'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, cleanup, renderHook, waitFor } from '@testing-library/react'
import type { ReactNode } from 'react'
import { MemoryRouter } from 'react-router'
import type { AgentStatus } from '@orbit/api/agent'
import { ChatContext } from '@/features/chat/api/chatContext'
import type { ChatClient } from '@/features/chat/api/client'
import { WorkspaceContext } from '@/features/workspaces/workspaceContext'
import { testWorkspace } from '@/test/workspace'
import { useAskDonna } from './useAskDonna'

afterEach(cleanup)

const connection = (connected: boolean) => ({ connected, invalid: false, account_id: connected ? 'acct' : null, connected_at: connected ? 1 : null })
const status = (connected: boolean): AgentStatus => ({
  enabled: true, online: true, can_connect: true, bot_user_id: 'bot', name: 'Donna', login: null, chatgpt: connection(connected),
})

function setup(connected: boolean) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: Infinity } } })
  client.setQueryData(['agent'], status(connected))
  const chat = { openDm: async () => ({ id: 'dm-1' }), sendMessage: async () => ({}) } as unknown as ChatClient
  const workspace = testWorkspace('member')
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={['/tasks']}>
        <WorkspaceContext.Provider value={{ workspace, workspaces: [workspace], selectWorkspace: () => {} }}>
          <ChatContext.Provider value={{ client: chat, workspaceId: workspace.id, currentUserId: 'u1' }}>{children}</ChatContext.Provider>
        </WorkspaceContext.Provider>
      </MemoryRouter>
    </QueryClientProvider>
  )
  return { client, hook: renderHook(() => useAskDonna(), { wrapper }) }
}

test('no bar without a ChatGPT connection', () => {
  expect(setup(false).hook.result.current).toBeNull()
})

test('the bar shows with a ChatGPT connection', () => {
  expect(setup(true).hook.result.current?.name).toBe('Donna')
})

test('losing the connection closes the open panel', async () => {
  const { client, hook } = setup(true)
  await act(async () => {
    await hook.result.current!.ask('')
  })
  expect(hook.result.current?.panel.open).toBeTrue()

  act(() => client.setQueryData(['agent'], status(false)))
  await waitFor(() => expect(hook.result.current).toBeNull())

  act(() => client.setQueryData(['agent'], status(true)))
  await waitFor(() => expect(hook.result.current).not.toBeNull())
  expect(hook.result.current?.panel.open).toBeFalse()
})
