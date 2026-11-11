import { useQuery } from '@tanstack/react-query'
import { apiClient } from '@/api/client'
import { queryKeys } from '@/api/queryKeys'
import { resolvePageDriveFiles } from '@orbit/api/generated/sdk.gen'
import type { DriveFileReference } from '@orbit/api/generated/types.gen'

/** The page whose blocks show Drive files: the server checks the reader's access to the page and to each file. */
export interface DriveFileHost {
  workspaceId: string
  pageId: string
}

/** What a `driveFile` block stores. */
export interface DriveFileRef {
  workspaceId: string
  nodeId: string
}

/** A 5-minute URL: it is asked for again before it runs out. */
const URL_STALE_MS = 4 * 60 * 1000

/**
 * What the reader may see of the file of one `driveFile` block, checked by the server on every read (available,
 * locked or unavailable). Blocks the editor has not saved yet resolve too.
 */
export function useDriveFileRef(host: DriveFileHost | null, ref: DriveFileRef) {
  return useQuery({
    queryKey: [...queryKeys.pages.all(host?.workspaceId ?? ''), 'driveFile', host?.pageId ?? '', ref.workspaceId, ref.nodeId],
    enabled: host !== null && ref.nodeId !== '',
    staleTime: URL_STALE_MS,
    queryFn: async (): Promise<DriveFileReference> => {
      const { data } = await resolvePageDriveFiles({
        client: apiClient,
        path: { workspace_id: host?.workspaceId ?? '', page_id: host?.pageId ?? '' },
        body: { references: [{ workspace_id: ref.workspaceId, node_id: ref.nodeId }] },
        throwOnError: true,
      })
      const item = data.items[0]
      if (!item) throw new Error('The server did not resolve the Drive file.')
      return item
    },
  })
}
