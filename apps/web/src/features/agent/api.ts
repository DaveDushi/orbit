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
import { apiClient } from '@/api/client'

const key = ['agent'] as const
const memoryKey = (workspaceId: string) => ['agent-memory', workspaceId] as const

/** The agent's state for the signed-in person. Polls while a ChatGPT sign-in waits for its code to be entered. */
export function useAgentStatus() {
  return useQuery({
    queryKey: key,
    queryFn: () => readAgentStatus(apiClient),
    refetchInterval: (query) => {
      const data = query.state.data
      return data?.enabled && data.login?.state === 'pending' && !data.chatgpt.connected ? 3_000 : false
    },
  })
}

export function useStartChatgptLogin() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: () => startChatgptLogin(apiClient),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: key }),
  })
}

export function useDisconnectChatgpt() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: () => disconnectChatgpt(apiClient),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: key }),
  })
}

/** One page of memories per scope and search; older pages load with `before`. */
export function useAgentMemories(workspaceId: string, scope: AgentMemoryScope, query: string) {
  return useInfiniteQuery({
    queryKey: [...memoryKey(workspaceId), scope, 'list', query],
    queryFn: ({ pageParam }) => readAgentMemories(apiClient, workspaceId, scope, { query, before: pageParam }),
    initialPageParam: null as number | null,
    getNextPageParam: (page) => page.next_before,
  })
}

/** What a run of the agent reads from this memory. */
export function useAgentMemoryWake(workspaceId: string, scope: AgentMemoryScope) {
  return useQuery({
    queryKey: [...memoryKey(workspaceId), scope, 'wake'],
    queryFn: () => readAgentMemoryWake(apiClient, workspaceId, scope),
  })
}

export function useDeleteAgentMemory(workspaceId: string, scope: AgentMemoryScope) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (position: number) => deleteAgentMemory(apiClient, workspaceId, scope, position),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: memoryKey(workspaceId) }),
  })
}

export function useForgetAgentMemorySummary(workspaceId: string, scope: AgentMemoryScope) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ lo, hi }: { lo: number; hi: number }) => forgetAgentMemorySummary(apiClient, workspaceId, scope, lo, hi),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: memoryKey(workspaceId) }),
  })
}
