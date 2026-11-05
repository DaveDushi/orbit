/**
 * The Drive upload queue of this tab. Files go straight from the browser to the bucket with presigned URLs: up to
 * 32 MiB in one PUT, larger ones in parts of the one size the server gives (4 parts at a time). Orbit completes an
 * upload from the bucket's own list of parts, so a reload loses nothing that was sent: the unfinished upload is
 * listed again, and resuming it with the same file sends only the missing parts.
 */
import { useSyncExternalStore } from 'react'
import {
  cancelDriveUpload,
  completeDriveUpload,
  getDriveUpload,
  signDriveUploadParts,
  startDriveUpload,
} from '@orbit/api/generated/sdk.gen'
import type { DriveNode, DriveUploadView } from '@orbit/api/generated/types.gen'
import { folderOf, foldersOf, missingParts, partRange } from '@orbit/api/drive'
import { ApiProblem } from '@orbit/api/problem'
import { apiClient } from '@/api/client'
import { createFolder } from './api'
import { makeThumbnail } from './media'

/** Files that upload at the same time. */
const PARALLEL_FILES = 2
/** Parts of one file that upload at the same time. */
const PARALLEL_PARTS = 4
/** Tries per part before the upload stops (it can be resumed). */
const PART_TRIES = 3
/** Part URLs asked for at once (the server signs at most 100). */
const URL_BATCH = 50

export type UploadStatus = 'queued' | 'uploading' | 'done' | 'failed' | 'cancelled'

export interface UploadItem {
  key: string
  workspaceId: string
  parentId: string | null
  name: string
  size: number
  status: UploadStatus
  /** Bytes the bucket has. */
  sent: number
  error?: string
  /** The server's id of the upload, once started. */
  uploadId?: string
  /** The file it became. */
  node?: DriveNode
}

interface Job {
  item: UploadItem
  file: File
  /** An unfinished upload to continue. */
  resume?: DriveUploadView
  controller: AbortController
}

/** A file to upload and its path below the upload's target (`a/b/c.txt` in a folder upload, else its name). */
export interface PickedFile {
  file: File
  relativePath?: string
}

type Listener = () => void

class UploadQueue {
  private jobs: Job[] = []
  private items: UploadItem[] = []
  private listeners = new Set<Listener>()
  private running = 0
  private nextKey = 0

  subscribe = (listener: Listener) => {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }

  snapshot = () => this.items

  /** Queues files for a folder (`null`: the top level). Files of a folder upload keep their tree: the folders are made first. */
  async add(workspaceId: string, parentId: string | null, picked: PickedFile[]) {
    const folders = new Map<string, string | null>([['', parentId]])
    const paths = picked.map((entry) => entry.relativePath ?? entry.file.name)
    for (const folder of foldersOf(paths)) {
      const parent = folders.get(folderOf(folder)) ?? parentId
      const name = folder.split('/').pop() ?? folder
      // A folder that exists at the target is kept: the upload goes into a new one ("Photos (2)").
      const created = await createFolder(workspaceId, parent, name, 'rename')
      folders.set(folder, created.id)
    }
    for (const [index, entry] of picked.entries()) {
      this.enqueue(workspaceId, folders.get(folderOf(paths[index] ?? '')) ?? parentId, entry.file)
    }
  }

  /** Continues an unfinished upload with the file it was started for. */
  resume(workspaceId: string, upload: DriveUploadView, file: File) {
    this.enqueue(workspaceId, upload.parent_id ?? null, file, upload)
  }

  cancel(key: string) {
    const job = this.jobs.find((candidate) => candidate.item.key === key)
    if (!job || job.item.status === 'done') return
    job.controller.abort()
    if (job.item.uploadId) void cancelUpload(job.item.workspaceId, job.item.uploadId)
    this.update(job, { status: 'cancelled' })
  }

  /** Removes finished, failed and cancelled uploads from the list. */
  clearFinished() {
    this.jobs = this.jobs.filter((job) => job.item.status === 'queued' || job.item.status === 'uploading')
    this.publish()
  }

  get busy() {
    return this.items.some((item) => item.status === 'queued' || item.status === 'uploading')
  }

  private enqueue(workspaceId: string, parentId: string | null, file: File, resume?: DriveUploadView) {
    this.nextKey += 1
    const item: UploadItem = {
      key: `upload-${this.nextKey}`,
      workspaceId,
      parentId,
      name: file.name,
      size: file.size,
      status: 'queued',
      sent: 0,
      uploadId: resume?.id,
    }
    this.jobs.push({ item, file, resume, controller: new AbortController() })
    this.publish()
    this.pump()
  }

  private pump() {
    while (this.running < PARALLEL_FILES) {
      const job = this.jobs.find((candidate) => candidate.item.status === 'queued')
      if (!job) return
      this.running += 1
      this.update(job, { status: 'uploading' })
      void this.run(job).finally(() => {
        this.running -= 1
        this.pump()
      })
    }
  }

  private async run(job: Job) {
    try {
      let node = await uploadFile(job, (patch) => this.update(job, patch))
      // The thumbnail comes from the local file; without one the file shows its icon.
      node = (await makeThumbnail(job.item.workspaceId, node, job.file).catch(() => null)) ?? node
      this.update(job, { status: 'done', sent: job.item.size, node })
    } catch (error) {
      if (job.controller.signal.aborted) return
      this.update(job, { status: 'failed', error: uploadError(error) })
    }
  }

  private update(job: Job, patch: Partial<UploadItem>) {
    job.item = { ...job.item, ...patch }
    this.publish()
  }

  private publish() {
    this.items = this.jobs.map((job) => job.item)
    for (const listener of this.listeners) listener()
  }
}

export const uploadQueue = new UploadQueue()

export function useUploads(): UploadItem[] {
  return useSyncExternalStore(uploadQueue.subscribe, uploadQueue.snapshot)
}

if (typeof window !== 'undefined') {
  window.addEventListener('beforeunload', (event) => {
    if (uploadQueue.busy) event.preventDefault()
  })
}

async function uploadFile(job: Job, update: (patch: Partial<UploadItem>) => void): Promise<DriveNode> {
  const { item, file } = job
  let upload = job.resume
  let singleUrl: string | null | undefined
  if (upload && upload.part_size == null) {
    // A single-PUT URL is not signed again: the old upload goes and a new one starts.
    await cancelUpload(item.workspaceId, upload.id)
    upload = undefined
  }
  if (!upload) {
    const { data } = await startDriveUpload({
      client: apiClient,
      path: { workspace_id: item.workspaceId },
      body: { parent_id: item.parentId, name: file.name, size: file.size, mime: file.type || undefined, on_conflict: 'rename' },
      throwOnError: true,
    })
    upload = data.upload
    singleUrl = data.url
    update({ uploadId: upload.id })
  }
  if (singleUrl) {
    await put(singleUrl, file, (loaded) => update({ sent: loaded }), job.controller.signal)
  } else {
    await sendParts(job, upload, update)
  }
  const { data } = await completeDriveUpload({
    client: apiClient,
    path: { workspace_id: item.workspaceId, upload_id: upload.id },
    throwOnError: true,
  })
  return data
}

async function sendParts(job: Job, upload: DriveUploadView, update: (patch: Partial<UploadItem>) => void) {
  const { item, file, controller } = job
  const partSize = upload.part_size ?? upload.size
  const { data: status } = await getDriveUpload({
    client: apiClient,
    path: { workspace_id: item.workspaceId, upload_id: upload.id },
    throwOnError: true,
  })
  const left = missingParts(upload, status.uploaded_parts)
  const done = new Set(Array.from({ length: upload.part_count }, (_, index) => index + 1).filter((number) => !left.includes(number)))
  const inFlight = new Map<number, number>()
  const report = () => {
    let sent = 0
    for (const number of done) sent += partBytes(number, partSize, upload.size)
    for (const loaded of inFlight.values()) sent += loaded
    update({ sent })
  }
  report()

  const urls = new Map<number, string>()
  const sign = async (numbers: number[]) => {
    const { data } = await signDriveUploadParts({
      client: apiClient,
      path: { workspace_id: item.workspaceId, upload_id: upload.id },
      body: { part_numbers: numbers },
      throwOnError: true,
    })
    for (const part of data.items) urls.set(part.part_number, part.url)
  }
  const queue = [...left]
  const worker = async () => {
    for (let number = queue.shift(); number !== undefined; number = queue.shift()) {
      const [start, end] = partRange(number, partSize, upload.size)
      for (let attempt = 1; ; attempt += 1) {
        if (!urls.has(number)) await sign([number, ...queue.slice(0, URL_BATCH - 1).filter((next) => !urls.has(next))])
        try {
          await put(urls.get(number) ?? '', file.slice(start, end), (loaded) => {
            inFlight.set(number, loaded)
            report()
          }, controller.signal)
          break
        } catch (error) {
          inFlight.delete(number)
          // An expired or refused URL is signed again on the next try.
          urls.delete(number)
          if (controller.signal.aborted || attempt >= PART_TRIES) throw error
          await new Promise((resolve) => setTimeout(resolve, 1000 * attempt))
        }
      }
      inFlight.delete(number)
      done.add(number)
      report()
    }
  }
  await Promise.all(Array.from({ length: Math.min(PARALLEL_PARTS, Math.max(1, left.length)) }, worker))
}

function partBytes(number: number, partSize: number, size: number) {
  const [start, end] = partRange(number, partSize, size)
  return end - start
}

class TransferError extends Error {}

/** PUTs the bytes to a presigned URL with progress (fetch has no upload progress). */
function put(url: string, body: Blob, onProgress: (loaded: number) => void, signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    const request = new XMLHttpRequest()
    request.open('PUT', url)
    request.upload.onprogress = (event) => onProgress(event.loaded)
    request.onload = () => (request.status >= 200 && request.status < 300 ? resolve() : reject(new TransferError(`storage answered ${request.status}`)))
    request.onerror = () => reject(new TransferError('the connection to the storage failed'))
    request.onabort = () => reject(new TransferError('cancelled'))
    signal.addEventListener('abort', () => request.abort(), { once: true })
    request.send(body)
  })
}

async function cancelUpload(workspaceId: string, uploadId: string) {
  try {
    await cancelDriveUpload({ client: apiClient, path: { workspace_id: workspaceId, upload_id: uploadId }, throwOnError: true })
  } catch {
    // An upload that is already gone needs no cancel; stale ones are cancelled by the server after 6 days.
  }
}

function uploadError(error: unknown): string {
  if (error instanceof ApiProblem) return error.detail
  if (error instanceof TransferError) return `Upload stopped: ${error.message}. Resume it later.`
  return 'Upload failed.'
}
