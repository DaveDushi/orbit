import { useEffect, useRef, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import type { DriveUploadView } from '@orbit/api/generated/types.gen'
import { formatSize } from '@/lib/attachmentLib'
import { uploadError } from '@/features/drive/uploads'
import { useChatContext } from '../../api/chatContext'
import { SMALL_FILE_BYTES, sendLargeFile, uploadLimit } from '../../api/largeFiles'
import { useUploadAttachment } from '../../api/mutations'
import { ChatError, type Attachment } from '../../api/types'

/** A failure of a file larger than the chat limit, with what to tell the user. */
class LargeFileError extends Error {}

export interface Upload {
  id: string
  file: File
  /** 0 to 1. */
  progress: number
  status: 'uploading' | 'failed' | 'done'
  /** The upload failed because the workspace storage quota is used up. */
  storageFull?: boolean
  /** Why the upload failed, when it says more than "Upload failed." */
  error?: string
  /** Larger than the chat limit: the file goes to the conversation's folder in Drive. */
  toDrive?: boolean
  /** The Drive upload, once started: "Try again" continues it with the parts that are missing. */
  drive?: DriveUploadView
  attachment?: Attachment
}

/**
 * The files of one composer: each uploads on its own, with progress, cancel and retry. A file larger than the chat
 * limit goes to the conversation's Drive folder (straight to the bucket, resumable).
 */
export function useUploads(conversationId: string) {
  const upload = useUploadAttachment()
  const { workspaceId } = useChatContext()
  const queryClient = useQueryClient()
  const [uploads, setUploads] = useState<Upload[]>([])
  const controllers = useRef(new Map<string, AbortController>())

  // Leaving the composer cancels what is still uploading.
  useEffect(() => {
    const running = controllers.current
    return () => {
      for (const controller of running.values()) controller.abort()
    }
  }, [])

  function patch(id: string, change: Partial<Upload>) {
    setUploads((current) => current.map((item) => (item.id === id ? { ...item, ...change } : item)))
  }

  async function send(id: string, file: File, signal: AbortSignal, resume?: DriveUploadView): Promise<Attachment> {
    const onProgress = (progress: number) => patch(id, { progress })
    const limit = file.size > SMALL_FILE_BYTES || resume ? await uploadLimit(queryClient, workspaceId) : null
    if (!limit || file.size <= limit.max_file_bytes) return upload(file, { signal, onProgress })
    if (!limit.drive) throw new LargeFileError(`Files larger than ${formatSize(limit.max_file_bytes)} need Drive storage, which this workspace does not have.`)
    patch(id, { toDrive: true })
    try {
      return await sendLargeFile({
        workspaceId,
        conversationId,
        file,
        signal,
        resume,
        onStarted: (drive) => patch(id, { drive }),
        onProgress,
      })
    } catch (error) {
      throw new LargeFileError(uploadError(error))
    }
  }

  function start(id: string, file: File, resume?: DriveUploadView) {
    const controller = new AbortController()
    controllers.current.set(id, controller)
    send(id, file, controller.signal, resume)
      .then(
        (attachment) => patch(id, { status: 'done', progress: 1, attachment }),
        (error: unknown) => {
          // A cancelled upload is removed by `remove`; anything else failed.
          if (controller.signal.aborted) return
          const storageFull = error instanceof ChatError && error.code === 'storage_full'
          patch(id, { status: 'failed', storageFull, error: error instanceof LargeFileError ? error.message : undefined })
        },
      )
      .finally(() => {
        if (controllers.current.get(id) === controller) controllers.current.delete(id)
      })
  }

  return {
    uploads,
    add(files: Iterable<File>) {
      const added = Array.from(files, (file): Upload => ({ id: crypto.randomUUID(), file, progress: 0, status: 'uploading' }))
      if (added.length === 0) return
      setUploads((current) => [...current, ...added])
      for (const item of added) start(item.id, item.file)
    },
    /** Cancels an upload in progress, or takes a finished or failed file out. */
    remove(id: string) {
      controllers.current.get(id)?.abort()
      setUploads((current) => current.filter((item) => item.id !== id))
    },
    retry(id: string) {
      const item = uploads.find((candidate) => candidate.id === id)
      if (!item) return
      patch(id, { status: 'uploading', storageFull: false, error: undefined })
      start(id, item.file, item.drive)
    },
    /** After a send: the files now belong to the message. */
    clear() {
      setUploads([])
    },
  }
}
