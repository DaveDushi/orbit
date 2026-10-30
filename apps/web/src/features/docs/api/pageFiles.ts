import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { apiClient, type createApiClient } from '@/api/client'
import { getPageFileContent, listPageFileVersions, restorePageFileVersion, savePageFile } from '@orbit/api/generated/sdk.gen'
import type { PageFile, PageFileContent, PageFileVersion } from '@orbit/api/generated/types.gen'

type ApiClient = ReturnType<typeof createApiClient>

export interface FileTarget {
  workspaceId: string
  pageId: string
  fileId: string
}

const UUID = '[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}'
const PAGE_FILE_URL = new RegExp(`^/api/v1/workspaces/(${UUID})/pages/(${UUID})/files/(${UUID})$`, 'i')

/** The file an Orbit file URL points at; `null` for an outside link, which the panel cannot open. */
export function fileTargetOf(url: string | null | undefined): FileTarget | null {
  const match = url ? PAGE_FILE_URL.exec(url) : null
  return match ? { workspaceId: match[1], pageId: match[2], fileId: match[3] } : null
}

export const fileUrl = ({ workspaceId, pageId, fileId }: FileTarget) => `/api/v1/workspaces/${workspaceId}/pages/${pageId}/files/${fileId}`

const fileKey = ({ workspaceId, pageId, fileId }: FileTarget) => ['workspace', workspaceId, 'pageFile', pageId, fileId] as const

export const FILE_READ_ROWS = 200

export async function fetchFileContent(client: ApiClient, target: FileTarget, query: { sheet?: string; offset?: number; limit?: number } = {}): Promise<PageFileContent> {
  const { data } = await getPageFileContent({
    client,
    path: { workspace_id: target.workspaceId, page_id: target.pageId, file_id: target.fileId },
    query,
    throwOnError: true,
  })
  if (data === undefined) throw new Error('File content response was empty.')
  return data
}

/** The first part of a file: what it is, and its first rows or text. */
export function useFileContent(target: FileTarget) {
  return useQuery({
    queryKey: [...fileKey(target), 'content'],
    queryFn: () => fetchFileContent(apiClient, target),
    // Limits and damaged files do not change by asking again.
    retry: false,
  })
}

/** The parts of one sheet or text, loaded as the reader reaches them. */
export function useFileParts(target: FileTarget, sheet: string | undefined, enabled: boolean) {
  return useInfiniteQuery({
    queryKey: [...fileKey(target), 'parts', sheet ?? ''],
    queryFn: ({ pageParam }) => fetchFileContent(apiClient, target, { sheet, offset: pageParam, limit: FILE_READ_ROWS }),
    initialPageParam: 0,
    getNextPageParam: (last: PageFileContent) => last.next_offset ?? undefined,
    enabled,
    retry: false,
  })
}

/** All rows of a csv or tsv file, for editing (such a file is at most 2 MB). */
export async function fetchAllRows(target: FileTarget): Promise<{ rows: string[][]; version: number }> {
  const rows: string[][] = []
  let offset: number | null = 0
  let version = 0
  while (offset !== null) {
    const part: PageFileContent = await fetchFileContent(apiClient, target, { offset, limit: 1000 })
    version = part.file.version
    rows.push(...(part.rows ?? []))
    offset = part.next_offset ?? null
  }
  return { rows, version }
}

/** All text of a file, for editing. */
export async function fetchAllText(target: FileTarget): Promise<{ text: string; version: number }> {
  let text = ''
  let offset: number | null = 0
  let version = 0
  while (offset !== null) {
    const part: PageFileContent = await fetchFileContent(apiClient, target, { offset, limit: 200_000 })
    version = part.file.version
    text += part.text ?? ''
    offset = part.next_offset ?? null
  }
  return { text, version }
}

export type FileSave = { expected_version: number } & ({ text: string } | { rows: string[][] })

export function useSaveFile(target: FileTarget) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (body: FileSave): Promise<PageFile> => {
      const { data } = await savePageFile({
        client: apiClient,
        path: { workspace_id: target.workspaceId, page_id: target.pageId, file_id: target.fileId },
        body,
        throwOnError: true,
      })
      if (data === undefined) throw new Error('Save response was empty.')
      return data
    },
    onSuccess: () => queryClient.invalidateQueries({ queryKey: fileKey(target) }),
  })
}

export function useFileVersions(target: FileTarget, enabled: boolean) {
  return useQuery({
    queryKey: [...fileKey(target), 'versions'],
    queryFn: async (): Promise<PageFileVersion[]> => {
      const { data } = await listPageFileVersions({
        client: apiClient,
        path: { workspace_id: target.workspaceId, page_id: target.pageId, file_id: target.fileId },
        throwOnError: true,
      })
      return data?.items ?? []
    },
    enabled,
  })
}

export function useRestoreFileVersion(target: FileTarget) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async ({ version, expectedVersion }: { version: number; expectedVersion: number }) => {
      await restorePageFileVersion({
        client: apiClient,
        path: { workspace_id: target.workspaceId, page_id: target.pageId, file_id: target.fileId, version },
        body: { expected_version: expectedVersion },
        throwOnError: true,
      })
    },
    onSuccess: () => queryClient.invalidateQueries({ queryKey: fileKey(target) }),
  })
}
