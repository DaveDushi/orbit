/** Create, join, export and delete a workspace, over `data/workspaces.ts`. */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'

import { keys } from '../data/keys'
import type { WorkspaceRecord } from '../data/orbit'
import * as api from '../data/workspaces'
import { useOrbit } from '../orbit-context'
import { useSignedIn } from '../session-context'

/**
 * Creates a workspace and puts it in the list at once. Works without a current workspace (the no-workspace screen):
 * there the app opens it as soon as the list has it; `onCreated` lets a switcher select it.
 */
export function useCreateWorkspace(onCreated?: (workspace: WorkspaceRecord) => void) {
  const { client } = useSignedIn()
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (name: string) => api.createWorkspace(client, name),
    onSuccess: (created) => {
      queryClient.setQueryData(keys.workspaces, (items: WorkspaceRecord[] | undefined) => [...(items ?? []).filter((item) => item.id !== created.id), created])
      onCreated?.(created)
      void queryClient.invalidateQueries({ queryKey: keys.workspaces })
    },
  })
}

/** Joins a workspace with an invitation for the signed-in account; `onJoined` gets the workspace id. */
export function useJoinWorkspace(onJoined?: (workspaceId: string) => void) {
  const { client } = useSignedIn()
  const queryClient = useQueryClient()
  return {
    preview: useMutation({ mutationFn: (token: string) => api.previewInvitation(client, token) }),
    accept: useMutation({
      mutationFn: (token: string) => api.acceptInvitation(client, token),
      onSuccess: async (accepted) => {
        await queryClient.invalidateQueries({ queryKey: keys.workspaces })
        onJoined?.(accepted.workspace_id)
      },
    }),
  }
}

const exportsKey = (workspaceId: string) => ['workspace-exports', workspaceId] as const

export function useWorkspaceExports(enabled: boolean) {
  const { client, workspaceId } = useOrbit()
  return useQuery({
    queryKey: exportsKey(workspaceId),
    queryFn: () => api.workspaceExports(client, workspaceId),
    enabled,
    refetchInterval: (query) => api.exportPollInterval(query.state.data),
  })
}

export function useStartExport() {
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: () => api.startExport(client, workspaceId),
    onSuccess: () => void queryClient.invalidateQueries({ queryKey: exportsKey(workspaceId) }),
  })
}

/** Deletes the current workspace: the list drops it and the app opens the next one (or the no-workspace screen). */
export function useDeleteWorkspace() {
  const { client, workspace } = useOrbit()
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: () => api.deleteWorkspace(client, workspace),
    onSuccess: () => {
      queryClient.setQueryData(keys.workspaces, (items: WorkspaceRecord[] | undefined) => items?.filter((item) => item.id !== workspace.id))
      void queryClient.invalidateQueries({ queryKey: keys.workspaces })
    },
  })
}
