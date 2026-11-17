/**
 * Orbit Drive on the phone (the web's `features/drive`): the calls of the Drive tab, and the upload of a picked file
 * straight to the bucket with presigned URLs (one PUT up to the server's limit, else parts of the size it gives, one at
 * a time with retries). Orbit completes the upload from the bucket's own list of parts.
 */
import { missingParts, partRange } from '@orbit/api/drive'
import {
  cancelDriveUpload,
  completeDriveUpload,
  createDriveFolder,
  createDriveShare,
  emptyDriveTrash,
  getDriveDownload,
  getDriveNode,
  getDriveUpload,
  listDriveNodes,
  listDriveShares,
  listDriveTrash,
  moveDriveNode,
  purgeDriveNode,
  renameDriveNode,
  restoreDriveNode,
  revokeDriveShare,
  searchDrive,
  signDriveUploadParts,
  startDriveUpload,
  trashDriveNode,
} from '@orbit/api/generated/sdk.gen'
import type { DriveEntry, DriveNode, DriveNodeDetail, DriveShare, DriveShareCreated, DriveSort, SortOrder } from '@orbit/api/generated/types.gen'

import type { OrbitClient } from './orbit'

export type { DriveEntry, DriveNode, DriveNodeDetail, DriveShare, DriveSort, SortOrder }

const PAGE = 500
/** Tries per part before the upload stops. */
const PART_TRIES = 3

/** The items of a folder (`null`: the top level), folders first, then by `sort` (name by default). */
export async function driveChildren(client: OrbitClient, workspaceId: string, parentId: string | null, sort: DriveSort = 'name', order: SortOrder = 'asc'): Promise<DriveNode[]> {
  const items: DriveNode[] = []
  let offset: number | null | undefined = 0
  while (offset !== null && offset !== undefined) {
    const { data }: { data: { items: DriveNode[]; next_offset?: number | null } } = await listDriveNodes({
      client,
      path: { workspace_id: workspaceId },
      query: { parent_id: parentId ?? undefined, sort, order, offset, limit: PAGE },
      throwOnError: true,
    })
    items.push(...data.items)
    offset = data.next_offset
  }
  return items
}

/** A folder and the folders above it (for the title and the way back). */
export async function driveNode(client: OrbitClient, workspaceId: string, nodeId: string): Promise<DriveNodeDetail> {
  return (await getDriveNode({ client, path: { workspace_id: workspaceId, node_id: nodeId }, throwOnError: true })).data
}

export async function createFolder(client: OrbitClient, workspaceId: string, parentId: string | null, name: string): Promise<DriveNode> {
  return (await createDriveFolder({ client, path: { workspace_id: workspaceId }, body: { parent_id: parentId, name, on_conflict: 'fail' }, throwOnError: true })).data
}

export async function renameNode(client: OrbitClient, workspaceId: string, nodeId: string, name: string): Promise<DriveNode> {
  return (await renameDriveNode({ client, path: { workspace_id: workspaceId, node_id: nodeId }, body: { name }, throwOnError: true })).data
}

export async function moveNode(client: OrbitClient, workspaceId: string, nodeId: string, parentId: string | null): Promise<DriveNode> {
  return (await moveDriveNode({ client, path: { workspace_id: workspaceId, node_id: nodeId }, body: { parent_id: parentId }, throwOnError: true })).data
}

/** Files and folders whose name matches, anywhere in Drive, with the folder each is in (the first 200). */
export async function searchNodes(client: OrbitClient, workspaceId: string, query: string): Promise<DriveEntry[]> {
  return (await searchDrive({ client, path: { workspace_id: workspaceId }, query: { q: query, limit: 200 }, throwOnError: true })).data.items
}

/** What is in the trash, newest first. */
export async function driveTrash(client: OrbitClient, workspaceId: string): Promise<DriveNode[]> {
  const items: DriveNode[] = []
  let offset: number | null | undefined = 0
  while (offset !== null && offset !== undefined) {
    const { data }: { data: { items: DriveNode[]; next_offset?: number | null } } = await listDriveTrash({ client, path: { workspace_id: workspaceId }, query: { offset, limit: PAGE }, throwOnError: true })
    items.push(...data.items)
    offset = data.next_offset
  }
  return items
}

export async function restoreNode(client: OrbitClient, workspaceId: string, nodeId: string): Promise<DriveNode> {
  return (await restoreDriveNode({ client, path: { workspace_id: workspaceId, node_id: nodeId }, throwOnError: true })).data
}

export async function purgeNode(client: OrbitClient, workspaceId: string, nodeId: string): Promise<void> {
  await purgeDriveNode({ client, path: { workspace_id: workspaceId, node_id: nodeId }, throwOnError: true })
}

export async function emptyTrash(client: OrbitClient, workspaceId: string): Promise<number> {
  return (await emptyDriveTrash({ client, path: { workspace_id: workspaceId }, throwOnError: true })).data.deleted
}

/** The public links of a file or folder. */
export async function shares(client: OrbitClient, workspaceId: string, nodeId: string): Promise<DriveShare[]> {
  return (await listDriveShares({ client, path: { workspace_id: workspaceId, node_id: nodeId }, throwOnError: true })).data.items
}

/** A new link; its address is in the answer only (Orbit keeps a hash). */
export async function newShare(client: OrbitClient, workspaceId: string, nodeId: string, body: { expires_at: string | null; password: string | null }): Promise<DriveShareCreated> {
  return (await createDriveShare({ client, path: { workspace_id: workspaceId, node_id: nodeId }, body, throwOnError: true })).data
}

export async function revokeShare(client: OrbitClient, workspaceId: string, shareId: string): Promise<void> {
  await revokeDriveShare({ client, path: { workspace_id: workspaceId, share_id: shareId }, throwOnError: true })
}

/** `Active`, `Expired` or `Off` (revoked), as the web's share list. */
export function shareState(share: Pick<DriveShare, 'revoked_at' | 'expires_at'>, now = Date.now()): { label: 'Active' | 'Expired' | 'Off'; active: boolean } {
  if (share.revoked_at) return { label: 'Off', active: false }
  if (share.expires_at && Date.parse(share.expires_at) <= now) return { label: 'Expired', active: false }
  return { label: 'Active', active: true }
}

/** The expiry of a new link: `null` (never) or `days` from now. */
export const shareExpiry = (days: number | null, now = Date.now()): string | null => (days === null ? null : new Date(now + days * 86_400_000).toISOString())

/** Folders a node may move into: not itself (the server also refuses a folder below itself). */
export const moveTargets = (folders: readonly DriveNode[], moving: Pick<DriveNode, 'id'>) => folders.filter((node) => node.kind === 'folder' && node.id !== moving.id)

export async function trashNode(client: OrbitClient, workspaceId: string, nodeId: string): Promise<void> {
  await trashDriveNode({ client, path: { workspace_id: workspaceId, node_id: nodeId }, throwOnError: true })
}

/** A 5-minute URL to the file's bytes. It is presigned: it needs no session token. */
export async function downloadUrl(client: OrbitClient, workspaceId: string, nodeId: string): Promise<string> {
  return (await getDriveDownload({ client, path: { workspace_id: workspaceId, node_id: nodeId }, query: { inline: true }, throwOnError: true })).data.url
}

/** A picked file to send: its bytes are read a range at a time, so a large video is never in memory as a whole. */
export interface DriveFile {
  name: string
  size: number
  mimeType?: string
  read: (start: number, end: number) => Promise<Uint8Array>
}

/** Sends a picked file to a folder (`null`: the top level); a file of the same name there gets a new name. */
export async function uploadToDrive(
  client: OrbitClient,
  workspaceId: string,
  parentId: string | null,
  file: DriveFile,
  onProgress: (sent: number) => void = () => {},
): Promise<DriveNode> {
  const path = { workspace_id: workspaceId }
  const { data: started } = await startDriveUpload({
    client,
    path,
    body: { parent_id: parentId, name: file.name, size: file.size, mime: file.mimeType || undefined, on_conflict: 'rename' },
    throwOnError: true,
  })
  const upload = started.upload
  try {
    if (started.url) {
      await put(started.url, await file.read(0, file.size))
      onProgress(file.size)
    } else {
      const partSize = upload.part_size ?? upload.size
      const { data: status } = await getDriveUpload({ client, path: { ...path, upload_id: upload.id }, throwOnError: true })
      let sent = 0
      for (const number of missingParts(upload, status.uploaded_parts)) {
        const [start, end] = partRange(number, partSize, upload.size)
        const bytes = await file.read(start, end)
        for (let attempt = 1; ; attempt += 1) {
          try {
            // A URL is signed for each try: an expired one is not used again.
            const { data } = await signDriveUploadParts({ client, path: { ...path, upload_id: upload.id }, body: { part_numbers: [number] }, throwOnError: true })
            await put(data.items[0]?.url ?? '', bytes)
            break
          } catch (error) {
            if (attempt >= PART_TRIES) throw error
            await new Promise((resolve) => setTimeout(resolve, 1000 * attempt))
          }
        }
        sent += end - start
        onProgress(sent)
      }
    }
    return (await completeDriveUpload({ client, path: { ...path, upload_id: upload.id }, throwOnError: true })).data
  } catch (error) {
    // Nothing to resume on the phone: the bucket's parts go now (the server would drop them after 6 days).
    await cancelDriveUpload({ client, path: { ...path, upload_id: upload.id } }).catch(() => undefined)
    throw error
  }
}

async function put(url: string, bytes: Uint8Array): Promise<void> {
  const response = await fetch(url, { method: 'PUT', body: bytes as unknown as BodyInit })
  if (!response.ok) throw new Error(`The storage answered ${response.status}.`)
}
