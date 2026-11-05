/**
 * Thumbnails and video posters are made in the browser, never on the server: when a file is uploaded (from the
 * local file), and for older files when a member first sees them (from a presigned URL). The browser measures the
 * image or video, draws a scaled frame on a canvas and PUTs it to the bucket; the server keeps it after it checked
 * the object.
 */
import { beginDriveMedia, completeDriveMedia } from '@orbit/api/generated/sdk.gen'
import type { DriveNode } from '@orbit/api/generated/types.gen'
import { POSTER_PIXELS, THUMB_PIXELS, driveFileKind, scaledSize } from '@orbit/api/drive'
import { apiClient } from '@/api/client'
import { downloadUrl } from './api'

interface Frame {
  source: CanvasImageSource
  width: number
  height: number
  durationMs?: number
  release: () => void
}

/** The time a video's frame is taken at: one second in, or earlier in a short video. */
const frameTime = (duration: number) => (Number.isFinite(duration) && duration > 0 ? Math.min(1, duration / 3) : 0)

async function imageFrame(blob: Blob): Promise<Frame> {
  const bitmap = await createImageBitmap(blob, { imageOrientation: 'from-image' })
  return { source: bitmap, width: bitmap.width, height: bitmap.height, release: () => bitmap.close() }
}

function videoFrame(src: string, crossOrigin: boolean): Promise<Frame> {
  return new Promise((resolve, reject) => {
    const video = document.createElement('video')
    if (crossOrigin) video.crossOrigin = 'anonymous'
    video.muted = true
    video.preload = 'auto'
    video.playsInline = true
    const fail = () => reject(new Error('the browser cannot read this video'))
    const timer = window.setTimeout(fail, 30_000)
    video.onerror = fail
    video.onloadedmetadata = () => {
      video.currentTime = frameTime(video.duration)
    }
    video.onseeked = () => {
      window.clearTimeout(timer)
      if (video.videoWidth === 0) return fail()
      resolve({
        source: video,
        width: video.videoWidth,
        height: video.videoHeight,
        durationMs: Number.isFinite(video.duration) ? Math.round(video.duration * 1000) : undefined,
        release: () => video.removeAttribute('src'),
      })
    }
    video.src = src
  })
}

async function encode(frame: Frame, maxPixels: number): Promise<Blob> {
  const size = scaledSize(frame.width, frame.height, maxPixels)
  const canvas = document.createElement('canvas')
  canvas.width = size.width
  canvas.height = size.height
  const context = canvas.getContext('2d')
  if (!context) throw new Error('no canvas')
  context.drawImage(frame.source, 0, 0, size.width, size.height)
  const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, 'image/webp', 0.8))
  if (blob && blob.type === 'image/webp') return blob
  // Browsers without WebP encoding give PNG: JPEG is smaller.
  const jpeg = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, 'image/jpeg', 0.82))
  if (!jpeg) throw new Error('the canvas gave no image')
  return jpeg
}

/** Files whose thumbnail this tab made or is making. */
const started = new Set<string>()

async function put(url: string, blob: Blob) {
  const response = await fetch(url, { method: 'PUT', body: blob, headers: { 'Content-Type': blob.type } })
  if (!response.ok) throw new Error(`storage answered ${response.status}`)
}

/**
 * Makes and stores the thumbnail (and a video's poster) of a file. `local` is the uploaded file; without it the
 * bytes are read from the bucket. A file the browser cannot read is marked so no one tries again. Returns the node
 * with its thumbnail URL, or `null` when nothing was stored.
 */
export async function makeThumbnail(workspaceId: string, node: DriveNode, local?: Blob): Promise<DriveNode | null> {
  const kind = driveFileKind(node)
  // Once per file and tab: an upload and the grid can both see a new file without a thumbnail.
  if ((kind !== 'image' && kind !== 'video') || started.has(node.id)) return null
  started.add(node.id)
  const path = { workspace_id: workspaceId, node_id: node.id }
  let frame: Frame
  let objectUrl: string | null = null
  try {
    if (kind === 'image') {
      const blob = local ?? (await (await fetch(await downloadUrl(workspaceId, node.id, true))).blob())
      frame = await imageFrame(blob)
    } else if (local) {
      objectUrl = URL.createObjectURL(local)
      frame = await videoFrame(objectUrl, false)
    } else {
      frame = await videoFrame(await downloadUrl(workspaceId, node.id, true), true)
    }
  } catch {
    if (objectUrl) URL.revokeObjectURL(objectUrl)
    await beginDriveMedia({ client: apiClient, path, body: { failed: true }, throwOnError: true }).catch(() => undefined)
    return null
  }
  try {
    const thumb = await encode(frame, THUMB_PIXELS)
    const poster = kind === 'video' ? await encode(frame, POSTER_PIXELS) : null
    const { data: urls } = await beginDriveMedia({
      client: apiClient,
      path,
      body: { width: frame.width, height: frame.height, duration_ms: frame.durationMs ?? null },
      throwOnError: true,
    })
    if (urls.thumbnail_put_url) await put(urls.thumbnail_put_url, thumb)
    if (urls.poster_put_url && poster) await put(urls.poster_put_url, poster)
    const { data } = await completeDriveMedia({ client: apiClient, path, throwOnError: true })
    return data
  } finally {
    frame.release()
    if (objectUrl) URL.revokeObjectURL(objectUrl)
  }
}

let chain: Promise<unknown> = Promise.resolve()

/**
 * Makes thumbnails of older files one at a time, each file at most once per page load. `onDone` runs after each
 * stored thumbnail (to refresh the list).
 */
export function queueThumbnails(workspaceId: string, nodes: DriveNode[], onDone: () => void) {
  for (const node of nodes) {
    if (started.has(node.id)) continue
    chain = chain.then(() =>
      makeThumbnail(workspaceId, node)
        .then((made) => {
          if (made) onDone()
        })
        .catch(() => undefined),
    )
  }
}
