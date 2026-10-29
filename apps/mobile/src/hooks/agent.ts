/** The team agent's queries and writes for the signed-in person (the web's `features/agent/api.ts`). */
import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import {
  deleteAgentMemory,
  disconnectChatgpt,
  forgetAgentMemorySummary,
  readAgentMemories,
  readAgentMemoryWake,
  readAgentStatus,
  startChatgptLogin,
  type AgentMemoryScope,
} from '@orbit/api/agent'

import { useOrbit } from '../orbit-context'

const key = ['agent'] as const

/** Polls while a ChatGPT sign-in waits for its code to be entered. */
export function useAgentStatus() {
  const { client } = useOrbit()
  return useQuery({
    queryKey: key,
    queryFn: () => readAgentStatus(client),
    refetchInterval: (query) => {
      const data = query.state.data
      return data?.enabled && data.login?.state === 'pending' && !data.chatgpt.connected ? 3_000 : false
    },
  })
}

/** The server runs the team agent: its Account settings page is shown. */
export function useAgentEnabled(): boolean {
  return useAgentStatus().data?.enabled === true
}

export function useStartChatgptLogin() {
  const { client } = useOrbit()
  const queryClient = useQueryClient()
  return useMutation({ mutationFn: () => startChatgptLogin(client), onSettled: () => void queryClient.invalidateQueries({ queryKey: key }) })
}

export function useDisconnectChatgpt() {
  const { client } = useOrbit()
  const queryClient = useQueryClient()
  return useMutation({ mutationFn: () => disconnectChatgpt(client), onSettled: () => void queryClient.invalidateQueries({ queryKey: key }) })
}

const memoryKey = (workspaceId: string) => ['agent-memory', workspaceId] as const

/** The agent's memory in the current workspace, newest first; older pages load with `before`. */
export function useAgentMemories(scope: AgentMemoryScope, query: string) {
  const { client, workspaceId } = useOrbit()
  return useInfiniteQuery({
    queryKey: [...memoryKey(workspaceId), scope, 'list', query],
    queryFn: ({ pageParam }) => readAgentMemories(client, workspaceId, scope, { query, before: pageParam }),
    initialPageParam: null as number | null,
    getNextPageParam: (page) => page.next_before,
  })
}

/** What a run of the agent reads from this memory. */
export function useAgentMemoryWake(scope: AgentMemoryScope) {
  const { client, workspaceId } = useOrbit()
  return useQuery({ queryKey: [...memoryKey(workspaceId), scope, 'wake'], queryFn: () => readAgentMemoryWake(client, workspaceId, scope) })
}

export function useDeleteAgentMemory(scope: AgentMemoryScope) {
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (position: number) => deleteAgentMemory(client, workspaceId, scope, position),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: memoryKey(workspaceId) }),
  })
}

export function useForgetAgentMemorySummary(scope: AgentMemoryScope) {
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ lo, hi }: { lo: number; hi: number }) => forgetAgentMemorySummary(client, workspaceId, scope, lo, hi),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: memoryKey(workspaceId) }),
  })
}
