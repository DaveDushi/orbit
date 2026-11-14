import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import {
  createAgentSkill,
  deleteAgentSkill,
  installAgentSkill,
  readAgentSkill,
  readAgentSkills,
  updateAgentSkill,
  uploadAgentSkill,
  type AgentSkillChange,
} from '@orbit/api/agentSkills'
import { apiClient } from '@/api/client'

const skillsKey = (workspaceId: string) => ['agent', 'skills', workspaceId] as const

/** Agentic → Skills: owners and admins only (others get 403). */
export function useAgentSkills(workspaceId: string) {
  return useQuery({ queryKey: skillsKey(workspaceId), queryFn: () => readAgentSkills(apiClient, workspaceId) })
}

/** One skill with its SKILL.md body, for the edit dialog. */
export function useAgentSkill(workspaceId: string, id: string) {
  return useQuery({ queryKey: [...skillsKey(workspaceId), id], queryFn: () => readAgentSkill(apiClient, workspaceId, id), gcTime: 0 })
}

/** The three ways to add a skill. */
export type AddSkill = { kind: 'install'; source: string } | { kind: 'upload'; file: Blob } | { kind: 'write'; name: string; description: string; body: string }

export function useAddAgentSkill(workspaceId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (input: AddSkill) => {
      if (input.kind === 'install') return installAgentSkill(apiClient, workspaceId, input.source.trim())
      if (input.kind === 'upload') return uploadAgentSkill(apiClient, workspaceId, input.file)
      return createAgentSkill(apiClient, workspaceId, { name: input.name.trim(), description: input.description, body: input.body })
    },
    onSuccess: () => void queryClient.invalidateQueries({ queryKey: skillsKey(workspaceId) }),
  })
}

export function useUpdateAgentSkill(workspaceId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ id, change }: { id: string; change: AgentSkillChange }) => updateAgentSkill(apiClient, workspaceId, id, change),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: skillsKey(workspaceId) }),
  })
}

export function useDeleteAgentSkill(workspaceId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (id: string) => deleteAgentSkill(apiClient, workspaceId, id),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: skillsKey(workspaceId) }),
  })
}
