import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import {
  addEgressRule,
  answerEgressRequest,
  deleteEgressRule,
  readAgentSandbox,
  readEgressRequests,
  saveAgentSandbox,
  type AgentSandbox,
  type EgressAnswer,
  type SandboxSettingsInput,
} from '@orbit/api/agentSandbox'
import { ApiProblem } from '@orbit/api/problem'
import { apiClient } from '@/api/client'

const egressKey = (workspaceId: string) => ['agent', 'egress', workspaceId] as const
const sandboxKey = (workspaceId: string, agentId: string) => ['agent', 'sandbox', workspaceId, agentId] as const

/** The tool that gives a custom agent its sandbox (ticked on its card). */
export const SANDBOX_TOOL = 'sandbox_shell'

/** The server has no agent sandbox: every sandbox route answers 404 `sandbox_off`. */
export const sandboxOff = (error: unknown) => error instanceof ApiProblem && error.code === 'sandbox_off'

/**
 * The egress cards of one conversation (one request for all its card messages). While one waits for an answer it
 * reloads every 15 s, so an answer by someone else and the expiry show without a reload.
 */
export function useEgressRequests(workspaceId: string, conversationId: string) {
  return useQuery({
    queryKey: [...egressKey(workspaceId), conversationId],
    queryFn: () => readEgressRequests(apiClient, workspaceId, conversationId),
    refetchInterval: (query) => (query.state.data?.some((request) => request.status === 'pending') ? 15_000 : false),
    retry: (count, error) => !sandboxOff(error) && count < 2,
  })
}

/** Answers a card from the thread or from the Internet access page; both lists reload. */
export function useAnswerEgress(workspaceId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ requestId, answer }: { requestId: string; answer: EgressAnswer }) => answerEgressRequest(apiClient, workspaceId, requestId, answer),
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: egressKey(workspaceId) })
      void queryClient.invalidateQueries({ queryKey: ['agent', 'sandbox', workspaceId] })
    },
  })
}

/** An agent's sandbox settings, rules, pending requests and log (owners and admins). */
export function useAgentSandbox(workspaceId: string, agentId: string | null) {
  return useQuery({
    queryKey: sandboxKey(workspaceId, agentId ?? ''),
    queryFn: () => readAgentSandbox(apiClient, workspaceId, agentId ?? ''),
    enabled: agentId !== null,
    retry: (count, error) => !sandboxOff(error) && count < 2,
  })
}

/** Each change answers the whole view, which replaces the cached one. */
function useSandboxChange<Input>(workspaceId: string, agentId: string, change: (input: Input) => Promise<AgentSandbox>) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: change,
    onSuccess: (view) => queryClient.setQueryData(sandboxKey(workspaceId, agentId), view),
  })
}

export function useSaveAgentSandbox(workspaceId: string, agentId: string) {
  return useSandboxChange(workspaceId, agentId, (input: SandboxSettingsInput) => saveAgentSandbox(apiClient, workspaceId, agentId, input))
}

export function useAddEgressRule(workspaceId: string, agentId: string) {
  return useSandboxChange(workspaceId, agentId, (rule: { host: string; port: number; action: 'allow' | 'deny' }) => addEgressRule(apiClient, workspaceId, agentId, rule))
}

export function useDeleteEgressRule(workspaceId: string, agentId: string) {
  return useSandboxChange(workspaceId, agentId, (ruleId: string) => deleteEgressRule(apiClient, workspaceId, agentId, ruleId))
}
