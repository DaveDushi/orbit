import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import {
  createDriveShare,
  listDriveShares,
  openPublicShare,
  publicShareFolder,
  revokeDriveShare,
  unlockPublicShare,
} from '@orbit/api/generated/sdk.gen'
import { apiClient, createApiClient } from '@/api/client'
import { driveKeys } from './api'

/** Public link pages have no session: their client never sends anyone to sign in. */
const publicClient = createApiClient()
const PASS_HEADER = 'X-Orbit-Share-Pass'

const sharesKey = (workspaceId: string, nodeId: string) => [...driveKeys.all(workspaceId), 'shares', nodeId] as const

/** The links of a file or folder. */
export function useDriveShares(workspaceId: string, nodeId: string) {
  return useQuery({
    queryKey: sharesKey(workspaceId, nodeId),
    queryFn: async () => {
      const { data } = await listDriveShares({ client: apiClient, path: { workspace_id: workspaceId, node_id: nodeId }, throwOnError: true })
      return data.items
    },
  })
}

export function useCreateDriveShare(workspaceId: string, nodeId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (input: { expiresAt: string | null; password: string | null }) => {
      const { data } = await createDriveShare({
        client: apiClient,
        path: { workspace_id: workspaceId, node_id: nodeId },
        body: { expires_at: input.expiresAt, password: input.password },
        throwOnError: true,
      })
      return data
    },
    onSettled: () => queryClient.invalidateQueries({ queryKey: sharesKey(workspaceId, nodeId) }),
  })
}

export function useRevokeDriveShare(workspaceId: string, nodeId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (shareId: string) => {
      const { data } = await revokeDriveShare({ client: apiClient, path: { workspace_id: workspaceId, share_id: shareId }, throwOnError: true })
      return data
    },
    onSettled: () => queryClient.invalidateQueries({ queryKey: sharesKey(workspaceId, nodeId) }),
  })
}

/** The pass of a link with a password, for this tab only. */
const passKey = (token: string) => `orbit.drive.share-pass.${token}`
export const sharePass = (token: string) => sessionStorage.getItem(passKey(token))

const passHeaders = (token: string) => {
  const pass = sharePass(token)
  return pass ? { [PASS_HEADER]: pass } : undefined
}

export async function openShare(token: string) {
  const { data } = await openPublicShare({ client: publicClient, path: { token }, headers: passHeaders(token), throwOnError: true })
  return data
}

export async function shareFolder(token: string, folderId: string | null) {
  const { data } = await publicShareFolder({
    client: publicClient,
    path: { token },
    query: { folder_id: folderId ?? undefined, limit: 500 },
    headers: passHeaders(token),
    throwOnError: true,
  })
  return data
}

export async function unlockShare(token: string, password: string) {
  const { data } = await unlockPublicShare({ client: publicClient, path: { token }, body: { password }, throwOnError: true })
  sessionStorage.setItem(passKey(token), data.pass)
}

/** The address that sends the browser to the file's bytes (a redirect to the bucket). */
export function shareDownloadHref(token: string, nodeId: string | null, inline = false) {
  const query = new URLSearchParams()
  if (nodeId) query.set('node_id', nodeId)
  if (inline) query.set('inline', 'true')
  const pass = sharePass(token)
  if (pass) query.set('pass', pass)
  const search = query.toString()
  return `/api/v1/public/drive/shares/${encodeURIComponent(token)}/download${search ? `?${search}` : ''}`
}
