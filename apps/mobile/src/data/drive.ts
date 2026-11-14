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
  getDriveDownload,
  getDriveNode,
  getDriveUpload,
  listDriveNodes,
  renameDriveNode,
  signDriveUploadParts,
  startDriveUpload,
  trashDriveNode,
} from '@orbit/api/generated/sdk.gen'
import type { DriveNode, DriveNodeDetail } from '@orbit/api/generated/types.gen'

import type { OrbitClient } from './orbit'

export type { DriveNode, DriveNodeDetail }

const PAGE = 500
/** Tries per part before the upload stops. */
const PART_TRIES = 3

/** The items of a folder (`null`: the top level), folders first, then by name. */
export async function driveChildren(client: OrbitClient, workspaceId: string, parentId: string | null): Promise<DriveNode[]> {
  const items: DriveNode[] = []
  let offset: number | null | undefined = 0
  while (offset !== null && offset !== undefined) {
    const { data }: { data: { items: DriveNode[]; next_offset?: number | null } } = await listDriveNodes({
      client,
      path: { workspace_id: workspaceId },
      query: { parent_id: parentId ?? undefined, sort: 'name', order: 'asc', offset, limit: PAGE },
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
