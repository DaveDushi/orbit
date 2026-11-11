import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import {
  deleteAgentMemory,
  disconnectChatgpt,
  forgetAgentMemorySummary,
  readAgentMemories,
  readAgentMemoryWake,
  readAgentModel,
  readTeamInstructions,
  saveAgentModel,
  saveTeamInstructions,
  readAgentStatus,
  startChatgptLogin,
  createAgentProfile,
  deleteAgentProfile,
  readAgentProfiles,
  readAnswerLevel,
  saveAnswerLevel,
  setAgentProfileAvatar,
  updateAgentProfile,
  updateDefaultAgent,
  type AgentMemoryScope,
  type DefaultAgentInput,
  type AgentProfileInput,
  type AnswerLevel,
} from '@orbit/api/agent'
import { apiClient } from '@/api/client'

const key = ['agent'] as const
const memoryKey = (workspaceId: string) => ['agent-memory', workspaceId] as const

/** The agent's state for the signed-in person. Polls while a ChatGPT sign-in waits for its code to be entered. */
export function useAgentStatus(enabled = true) {
  return useQuery({
    queryKey: key,
    queryFn: () => readAgentStatus(apiClient),
    enabled,
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

const modelKey = ['agent', 'model'] as const

/** Agentic → Admin: the model and thinking level. Installation admins only. */
export function useAgentModel() {
  return useQuery({ queryKey: modelKey, queryFn: () => readAgentModel(apiClient) })
}

export function useSaveAgentModel() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (choice: { model: string | null; thinking: string | null }) => saveAgentModel(apiClient, choice),
    onSuccess: (saved) => queryClient.setQueryData(modelKey, saved),
  })
}

const teamKey = (workspaceId: string) => ['agent', 'team', workspaceId] as const

/** Agentic → Workspace: the team instructions. Every member reads them. */
export function useTeamInstructions(workspaceId: string) {
  return useQuery({ queryKey: teamKey(workspaceId), queryFn: () => readTeamInstructions(apiClient, workspaceId) })
}

export function useSaveTeamInstructions(workspaceId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (instructions: string) => saveTeamInstructions(apiClient, workspaceId, instructions),
    onSuccess: (saved) => queryClient.setQueryData(teamKey(workspaceId), saved),
  })
}

const profilesKey = (workspaceId: string) => ['agent', 'profiles', workspaceId] as const

/** Agentic → Agents: the workspace's agents besides the default one. Every member reads them; owners and admins change them. */
export function useAgentProfiles(workspaceId: string, enabled = true) {
  return useQuery({ queryKey: profilesKey(workspaceId), queryFn: () => readAgentProfiles(apiClient, workspaceId), enabled })
}

/** An agent's name and avatar are in the member lists too. */
function useRefreshAgents(workspaceId: string) {
  const queryClient = useQueryClient()
  return () => {
    void queryClient.invalidateQueries({ queryKey: profilesKey(workspaceId) })
    void queryClient.invalidateQueries({ predicate: (query) => query.queryKey.at(-1) === 'members' })
  }
}

export function useSaveAgentProfile(workspaceId: string) {
  const refresh = useRefreshAgents(workspaceId)
  return useMutation({
    mutationFn: async ({ id, input, avatar }: { id: string | null; input: AgentProfileInput; avatar?: Blob | null }) => {
      const saved = id ? await updateAgentProfile(apiClient, workspaceId, id, input) : await createAgentProfile(apiClient, workspaceId, input)
      // `undefined`: the avatar stays as it is.
      if (avatar !== undefined) await setAgentProfileAvatar(apiClient, workspaceId, saved.id, avatar)
      return saved
    },
    onSettled: refresh,
  })
}

/** The default agent's card. Its name is also in the agent status (the Ask bar, account page) and member lists. */
export function useSaveDefaultAgent(workspaceId: string) {
  const queryClient = useQueryClient()
  const refresh = useRefreshAgents(workspaceId)
  return useMutation({
    mutationFn: async ({ input, avatar }: { input: DefaultAgentInput; avatar?: Blob | null }) => {
      const saved = await updateDefaultAgent(apiClient, workspaceId, input)
      if (avatar !== undefined) await setAgentProfileAvatar(apiClient, workspaceId, 'default', avatar)
      return saved
    },
    onSettled: () => {
      refresh()
      void queryClient.invalidateQueries({ queryKey: key, exact: true })
    },
  })
}

export function useDeleteAgentProfile(workspaceId: string) {
  const refresh = useRefreshAgents(workspaceId)
  return useMutation({ mutationFn: (id: string) => deleteAgentProfile(apiClient, workspaceId, id), onSettled: refresh })
}

const answerLevelKey = ['agent', 'answer-level'] as const

/** Account settings → Agentic: how much every agent thinks and writes for the signed-in person. */
export function useAnswerLevel() {
  return useQuery({ queryKey: answerLevelKey, queryFn: () => readAnswerLevel(apiClient) })
}

export function useSaveAnswerLevel() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (level: AnswerLevel) => saveAnswerLevel(apiClient, level),
    onSuccess: (level) => queryClient.setQueryData(answerLevelKey, level),
  })
}
