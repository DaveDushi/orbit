/** Agent sandboxes: egress cards in chat, an agent's sandbox settings and Internet access (the web's `features/agent/sandboxApi.ts`). */
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

import { useOrbit } from '../orbit-context'

/** The tool that gives a custom agent its sandbox (ticked on its card). */
export const SANDBOX_TOOL = 'sandbox_shell'

/** The server has no agent sandbox: every sandbox route answers 404 `sandbox_off`. */
export const sandboxOff = (error: unknown) => error instanceof ApiProblem && error.code === 'sandbox_off'

const egressKey = (workspaceId: string) => ['agent', 'egress', workspaceId] as const
const sandboxKey = (workspaceId: string, agentId: string) => ['agent', 'sandbox', workspaceId, agentId] as const

/** The egress cards of one conversation; while one waits it reloads every 15 s (another person's answer, the expiry). */
export function useEgressRequests(conversationId: string) {
  const { client, workspaceId } = useOrbit()
  return useQuery({
    queryKey: [...egressKey(workspaceId), conversationId],
    queryFn: () => readEgressRequests(client, workspaceId, conversationId),
    refetchInterval: (query) => (query.state.data?.some((request) => request.status === 'pending') ? 15_000 : false),
    retry: (count, error) => !sandboxOff(error) && count < 2,
  })
}

export function useAnswerEgress() {
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ requestId, answer }: { requestId: string; answer: EgressAnswer }) => answerEgressRequest(client, workspaceId, requestId, answer),
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: egressKey(workspaceId) })
      void queryClient.invalidateQueries({ queryKey: ['agent', 'sandbox', workspaceId] })
    },
  })
}

/** An agent's sandbox (owners and admins); `agentId` null: nothing to load (a new agent). */
export function useAgentSandbox(agentId: string | null) {
  const { client, workspaceId } = useOrbit()
  return useQuery({
    queryKey: sandboxKey(workspaceId, agentId ?? ''),
    queryFn: () => readAgentSandbox(client, workspaceId, agentId ?? ''),
    enabled: agentId !== null,
    retry: (count, error) => !sandboxOff(error) && count < 2,
  })
}

function useSandboxChange<Input>(agentId: string, change: (input: Input) => Promise<AgentSandbox>) {
  const { workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  return useMutation({ mutationFn: change, onSuccess: (view) => queryClient.setQueryData(sandboxKey(workspaceId, agentId), view) })
}

export function useSaveAgentSandbox(agentId: string) {
  const { client, workspaceId } = useOrbit()
  return useSandboxChange(agentId, (input: SandboxSettingsInput) => saveAgentSandbox(client, workspaceId, agentId, input))
}

export function useAddEgressRule(agentId: string) {
  const { client, workspaceId } = useOrbit()
  return useSandboxChange(agentId, (rule: { host: string; port: number; action: 'allow' | 'deny' }) => addEgressRule(client, workspaceId, agentId, rule))
}

export function useDeleteEgressRule(agentId: string) {
  const { client, workspaceId } = useOrbit()
  return useSandboxChange(agentId, (ruleId: string) => deleteEgressRule(client, workspaceId, agentId, ruleId))
}
