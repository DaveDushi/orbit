import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import {
  createDriveDownloads,
  createDriveFolder,
  emptyDriveTrash,
  getDriveDownload,
  getDriveNode,
  listDriveFolderFiles,
  listDriveNodes,
  listDriveTrash,
  listDriveUploads,
  moveDriveNode,
  purgeDriveNode,
  renameDriveNode,
  requestDrivePreview,
  restoreDriveNode,
  searchDrive,
  trashDriveNode,
} from '@orbit/api/generated/sdk.gen'
import type { DriveNodeKind, DriveSort, OnConflict, SortOrder } from '@orbit/api/generated/types.gen'
import { apiClient } from '@/api/client'

const PAGE = 200

export type { DriveSort, SortOrder }

export const driveKeys = {
  all: (workspaceId: string) => ['workspace', workspaceId, 'drive'] as const,
  children: (workspaceId: string, parentId: string | null, sort: DriveSort, order: SortOrder) =>
    [...driveKeys.all(workspaceId), 'children', parentId ?? 'top', sort, order] as const,
  folders: (workspaceId: string, parentId: string | null) => [...driveKeys.all(workspaceId), 'folders', parentId ?? 'top'] as const,
  node: (workspaceId: string, nodeId: string) => [...driveKeys.all(workspaceId), 'node', nodeId] as const,
  trash: (workspaceId: string) => [...driveKeys.all(workspaceId), 'trash'] as const,
  search: (workspaceId: string, search: DriveSearchInput) => [...driveKeys.all(workspaceId), 'search', search] as const,
  uploads: (workspaceId: string) => [...driveKeys.all(workspaceId), 'uploads'] as const,
}

export interface DriveSearchInput {
  q: string
  kind?: DriveNodeKind
  mime?: string
  updatedAfter?: string
}

/** The items of a folder (`null`: the top level), folders first, one page of 200 at a time. */
export function useDriveChildren(workspaceId: string, parentId: string | null, sort: DriveSort, order: SortOrder) {
  return useInfiniteQuery({
    queryKey: driveKeys.children(workspaceId, parentId, sort, order),
    initialPageParam: 0,
    queryFn: async ({ pageParam }) => {
      const { data } = await listDriveNodes({
        client: apiClient,
        path: { workspace_id: workspaceId },
        query: { parent_id: parentId ?? undefined, sort, order, offset: pageParam, limit: PAGE },
        throwOnError: true,
      })
      return data
    },
    getNextPageParam: (page) => page.next_offset ?? undefined,
  })
}

/** The subfolders of a folder, for the tree (folders come first, so the first page has them). */
export function useDriveFolders(workspaceId: string, parentId: string | null, enabled = true) {
  return useQuery({
    queryKey: driveKeys.folders(workspaceId, parentId),
    enabled,
    queryFn: async () => {
      const { data } = await listDriveNodes({
        client: apiClient,
        path: { workspace_id: workspaceId },
        query: { parent_id: parentId ?? undefined, limit: 500 },
        throwOnError: true,
      })
      return data.items.filter((node) => node.kind === 'folder')
    },
  })
}

/** A folder and the folders above it (breadcrumbs). */
export function useDriveNode(workspaceId: string, nodeId: string | null) {
  return useQuery({
    queryKey: driveKeys.node(workspaceId, nodeId ?? ''),
    enabled: nodeId !== null,
    queryFn: async () => {
      const { data } = await getDriveNode({ client: apiClient, path: { workspace_id: workspaceId, node_id: nodeId ?? '' }, throwOnError: true })
      return data
    },
  })
}

export function useDriveTrash(workspaceId: string) {
  return useInfiniteQuery({
    queryKey: driveKeys.trash(workspaceId),
    initialPageParam: 0,
    queryFn: async ({ pageParam }) => {
      const { data } = await listDriveTrash({ client: apiClient, path: { workspace_id: workspaceId }, query: { offset: pageParam, limit: PAGE }, throwOnError: true })
      return data
    },
    getNextPageParam: (page) => page.next_offset ?? undefined,
  })
}

export function useDriveSearch(workspaceId: string, search: DriveSearchInput, enabled: boolean) {
  return useInfiniteQuery({
    queryKey: driveKeys.search(workspaceId, search),
    enabled,
    initialPageParam: 0,
    queryFn: async ({ pageParam }) => {
      const { data } = await searchDrive({
        client: apiClient,
        path: { workspace_id: workspaceId },
        query: {
          q: search.q || undefined,
          kind: search.kind,
          mime: search.mime,
          updated_after: search.updatedAfter,
          offset: pageParam,
          limit: PAGE,
        },
        throwOnError: true,
      })
      return data
    },
    getNextPageParam: (page) => page.next_offset ?? undefined,
  })
}

/** The caller's unfinished uploads (to resume after a reload). */
export function useDriveUploads(workspaceId: string) {
  return useQuery({
    queryKey: driveKeys.uploads(workspaceId),
    queryFn: async () => {
      const { data } = await listDriveUploads({ client: apiClient, path: { workspace_id: workspaceId }, throwOnError: true })
      return data.items
    },
  })
}

/** Every Drive list may change after a write: they are refetched. */
export function useRefreshDrive(workspaceId: string) {
  const queryClient = useQueryClient()
  return () => void queryClient.invalidateQueries({ queryKey: driveKeys.all(workspaceId) })
}

function useDriveMutation<Input, Output>(workspaceId: string, mutationFn: (input: Input) => Promise<Output>) {
  const refresh = useRefreshDrive(workspaceId)
  return useMutation({ mutationFn, onSettled: refresh })
}

export async function createFolder(workspaceId: string, parentId: string | null, name: string, onConflict: OnConflict = 'fail') {
  const { data } = await createDriveFolder({
    client: apiClient,
    path: { workspace_id: workspaceId },
    body: { parent_id: parentId, name, on_conflict: onConflict },
    throwOnError: true,
  })
  return data
}

export function useCreateFolder(workspaceId: string) {
  return useDriveMutation(workspaceId, ({ parentId, name }: { parentId: string | null; name: string }) => createFolder(workspaceId, parentId, name))
}

export function useRenameNode(workspaceId: string) {
  return useDriveMutation(workspaceId, async ({ nodeId, name }: { nodeId: string; name: string }) => {
    const { data } = await renameDriveNode({ client: apiClient, path: { workspace_id: workspaceId, node_id: nodeId }, body: { name }, throwOnError: true })
    return data
  })
}

export async function moveNode(workspaceId: string, nodeId: string, parentId: string | null) {
  const { data } = await moveDriveNode({ client: apiClient, path: { workspace_id: workspaceId, node_id: nodeId }, body: { parent_id: parentId }, throwOnError: true })
  return data
}

export async function trashNode(workspaceId: string, nodeId: string) {
  await trashDriveNode({ client: apiClient, path: { workspace_id: workspaceId, node_id: nodeId }, throwOnError: true })
}

export function useRestoreNode(workspaceId: string) {
  return useDriveMutation(workspaceId, async (nodeId: string) => {
    const { data } = await restoreDriveNode({ client: apiClient, path: { workspace_id: workspaceId, node_id: nodeId }, throwOnError: true })
    return data
  })
}

export function usePurgeNode(workspaceId: string) {
  return useDriveMutation(workspaceId, async (nodeId: string) => {
    await purgeDriveNode({ client: apiClient, path: { workspace_id: workspaceId, node_id: nodeId }, throwOnError: true })
  })
}

export function useEmptyTrash(workspaceId: string) {
  return useDriveMutation(workspaceId, async () => {
    const { data } = await emptyDriveTrash({ client: apiClient, path: { workspace_id: workspaceId }, throwOnError: true })
    return data
  })
}

/** A 5-minute URL to the file's bytes; `inline` lets the browser show it instead of saving it. */
export async function downloadUrl(workspaceId: string, nodeId: string, inline = false) {
  const { data } = await getDriveDownload({ client: apiClient, path: { workspace_id: workspaceId, node_id: nodeId }, query: { inline }, throwOnError: true })
  return data.url
}

/** Queues the preview of a file that has none yet and returns the file, whose `preview` says how far it is. */
export async function requestPreview(workspaceId: string, nodeId: string) {
  const { data } = await requestDrivePreview({ client: apiClient, path: { workspace_id: workspaceId, node_id: nodeId }, throwOnError: true })
  return data
}

/** Download URLs of up to 100 files, by node id. */
export async function downloadUrls(workspaceId: string, nodeIds: string[]): Promise<Map<string, string>> {
  const { data } = await createDriveDownloads({ client: apiClient, path: { workspace_id: workspaceId }, body: { node_ids: nodeIds }, throwOnError: true })
  return new Map(data.items.map((item) => [item.node_id, item.url]))
}

/** Every live file below a folder, with its path relative to it. */
export async function filesBelow(workspaceId: string, folderId: string) {
  const entries = []
  let offset: number | null | undefined = 0
  while (offset !== null && offset !== undefined) {
    const { data }: { data: Awaited<ReturnType<typeof listDriveFolderFiles<true>>>['data'] } = await listDriveFolderFiles({
      client: apiClient,
      path: { workspace_id: workspaceId, node_id: folderId },
      query: { offset, limit: 500 },
      throwOnError: true,
    })
    entries.push(...data.items)
    offset = data.next_offset
  }
  return entries
}
