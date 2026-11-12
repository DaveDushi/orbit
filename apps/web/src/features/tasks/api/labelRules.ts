import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { apiClient } from '@/api/client'
import { createLabelRule, deleteLabelRule, listLabelRules, putLabelRuleSettings, updateLabelRule } from '@orbit/api/generated/sdk.gen'
import type { LabelRuleBody, LabelRulePatchBody, LabelRuleSettings } from '@orbit/api/generated/types.gen'
import { queryKeys } from '@/api/queryKeys'

const key = (workspaceId: string, projectId: string) => [...queryKeys.workspace(workspaceId), 'label-rules', projectId] as const

/** The project's label rules, its owner switch and the `owner:` names that match nobody (owners and admins only). */
export function useLabelRules(workspaceId: string, projectId: string, enabled = true) {
  return useQuery({
    queryKey: key(workspaceId, projectId),
    enabled,
    queryFn: async () => {
      const { data } = await listLabelRules({ client: apiClient, path: { workspace_id: workspaceId, project_id: projectId }, throwOnError: true })
      if (!data) throw new Error('Label rules response was empty.')
      return data
    },
  })
}

function useRefresh(workspaceId: string, projectId: string) {
  const queryClient = useQueryClient()
  return () => void queryClient.invalidateQueries({ queryKey: key(workspaceId, projectId) })
}

export function useCreateLabelRule(workspaceId: string, projectId: string) {
  const refresh = useRefresh(workspaceId, projectId)
  return useMutation({
    mutationFn: async (body: LabelRuleBody) => {
      await createLabelRule({ client: apiClient, path: { workspace_id: workspaceId, project_id: projectId }, body, throwOnError: true })
    },
    onSuccess: refresh,
  })
}

export function useUpdateLabelRule(workspaceId: string, projectId: string) {
  const refresh = useRefresh(workspaceId, projectId)
  return useMutation({
    mutationFn: async ({ ruleId, ...body }: LabelRulePatchBody & { ruleId: string }) => {
      await updateLabelRule({ client: apiClient, path: { workspace_id: workspaceId, project_id: projectId, rule_id: ruleId }, body, throwOnError: true })
    },
    onSuccess: refresh,
  })
}

export function useDeleteLabelRule(workspaceId: string, projectId: string) {
  const refresh = useRefresh(workspaceId, projectId)
  return useMutation({
    mutationFn: async (ruleId: string) => {
      await deleteLabelRule({ client: apiClient, path: { workspace_id: workspaceId, project_id: projectId, rule_id: ruleId }, throwOnError: true })
    },
    onSuccess: refresh,
  })
}

export function useSetOwnerLabels(workspaceId: string, projectId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (ownerLabels: boolean) => {
      const { data } = await putLabelRuleSettings({ client: apiClient, path: { workspace_id: workspaceId, project_id: projectId }, body: { owner_labels: ownerLabels }, throwOnError: true })
      if (!data) throw new Error('Label rules response was empty.')
      return data
    },
    onSuccess: (data: LabelRuleSettings) => queryClient.setQueryData(key(workspaceId, projectId), data),
  })
}
