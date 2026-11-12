/**
 * Files larger than the chat limit go to the conversation's folder in Drive (`Uploads/#channel`, or a private
 * folder of a direct message's members): from the browser straight to the bucket, in parts that are sent again
 * after a dropped connection. The message then shows the Drive file like a chat file.
 */
import type { QueryClient } from '@tanstack/react-query'
import { attachChatDriveFile, getChatDriveFolder, getChatUploadLimit } from '@orbit/api/generated/sdk.gen'
import type { ChatUploadLimit, DriveUploadView } from '@orbit/api/generated/types.gen'
import { apiClient } from '@/api/client'
import { makeThumbnail } from '@/features/drive/media'
import { sendToDrive } from '@/features/drive/uploads'
import { chatKeys } from './keys'
import type { Attachment } from './types'
import { toAttachment } from './wire'

/** Smaller files are never too large for chat: no need to ask for the limit. */
export const SMALL_FILE_BYTES = 1024 * 1024

export function uploadLimit(queryClient: QueryClient, workspaceId: string): Promise<ChatUploadLimit> {
  return queryClient.fetchQuery({
    queryKey: chatKeys.uploadLimit(workspaceId),
    staleTime: Infinity,
    queryFn: async () => (await getChatUploadLimit({ client: apiClient, path: { workspace_id: workspaceId }, throwOnError: true })).data,
  })
}

export interface LargeFileSend {
  workspaceId: string
  conversationId: string
  file: File
  signal: AbortSignal
  /** An unfinished upload of this file: only the parts the bucket lacks are sent. */
  resume?: DriveUploadView
  onStarted: (upload: DriveUploadView) => void
  /** 0 to 1. */
  onProgress: (progress: number) => void
}

/** Sends a large file to the conversation's Drive folder and makes it a file for the next message. */
export async function sendLargeFile(send: LargeFileSend): Promise<Attachment> {
  const path = { workspace_id: send.workspaceId }
  const { data: folder } = await getChatDriveFolder({
    client: apiClient,
    path: { ...path, conversation_id: send.conversationId },
    throwOnError: true,
  })
  const node = await sendToDrive({
    workspaceId: send.workspaceId,
    parentId: folder.id,
    file: send.file,
    resume: send.resume,
    signal: send.signal,
    onStarted: send.onStarted,
    onProgress: (sent) => send.onProgress(send.file.size > 0 ? sent / send.file.size : 1),
  })
  // The Drive list shows a thumbnail, as for files uploaded in Drive; without one it shows the file's icon.
  await makeThumbnail(send.workspaceId, node, send.file).catch(() => null)
  const { data } = await attachChatDriveFile({ client: apiClient, path, body: { node_id: node.id }, throwOnError: true })
  return toAttachment(data)
}
