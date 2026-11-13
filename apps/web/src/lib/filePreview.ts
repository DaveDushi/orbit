import { useQuery } from '@tanstack/react-query'
import type { FilePreview } from '@orbit/api/generated/types.gen'
import { CONTRACT_ID } from '@/api/client'

export type { FilePreview }

/** How often a preview that the server is still making is asked for again. */
const PREVIEW_POLL_MS = 4000
/** A preview is not waited for longer than this (the job may wait behind others): the card stops asking. */
const PREVIEW_POLL_LIMIT = 150

/** What the viewer shows for a file. */
export type PreviewView =
  | { kind: 'image'; src: string }
  | { kind: 'video'; src: string; poster?: string }
  | { kind: 'audio'; src: string }
  | { kind: 'pdf'; src: string }
  /** The server makes the preview now. */
  | { kind: 'pending'; label: string }
  | { kind: 'unavailable'; message: string }

export interface PreviewFile {
  mimeType: string
  fileName: string
  /** Same-origin URL of the bytes. */
  url: string
  preview?: FilePreview | null
}

/** Whether a file shows as a table with sheet tabs in the viewer (the server reads these kinds): xlsx, xls, csv, tsv. */
export function isSheetFile(fileName: string): boolean {
  return /\.(xlsx|xlsm|xls|csv|tsv)$/i.test(fileName)
}

/** Whether a file opens in the viewer (rather than downloads) when clicked. */
export function hasViewer(file: Pick<PreviewFile, 'mimeType' | 'fileName' | 'preview'>): boolean {
  return /^(image|video|audio)\//.test(file.mimeType) || file.mimeType === 'application/pdf' || Boolean(file.preview) || isSheetFile(file.fileName)
}

/** Whether a file plays as a video: by its type, or because the server makes a playable copy of it. */
export function isVideoFile(file: Pick<PreviewFile, 'mimeType' | 'preview'>): boolean {
  return file.mimeType.startsWith('video/') || file.preview?.kind === 'video'
}

/**
 * What to show for a file: a video plays its playable copy once the server made it, and the original when the
 * copy failed or is too large to make; an office file shows its PDF.
 */
export function previewView(file: PreviewFile): PreviewView {
  const preview = file.preview
  if (file.mimeType.startsWith('image/')) return { kind: 'image', src: file.url }
  if (file.mimeType === 'application/pdf') return { kind: 'pdf', src: file.url }
  if (file.mimeType.startsWith('audio/')) return { kind: 'audio', src: file.url }
  if (isVideoFile(file)) {
    if (preview?.status === 'pending') return { kind: 'pending', label: 'Processing the video…' }
    const poster = preview?.poster_url ?? undefined
    if (preview?.status === 'ready' && preview.play_url) return { kind: 'video', src: preview.play_url, poster }
    return { kind: 'video', src: inlineUrl(file.url), poster }
  }
  if (preview?.kind === 'document') {
    if (preview.status === 'ready' && preview.pdf_url) return { kind: 'pdf', src: preview.pdf_url }
    if (preview.status === 'pending' || preview.status === 'none') return { kind: 'pending', label: 'Preparing the preview…' }
    return { kind: 'unavailable', message: 'Preview not available for this file. Download it to open it.' }
  }
  return { kind: 'unavailable', message: 'No preview for this file.' }
}

/** A file kept in Drive opens in the browser with `?inline=true` (else its URL makes the browser save it). */
export function inlineUrl(url: string): string {
  return url.includes('?') ? `${url}&inline=true` : `${url}?inline=true`
}

async function previewRequest(url: string, method: 'GET' | 'POST'): Promise<FilePreview | null> {
  const response = await fetch(`${url}/preview`, {
    method,
    credentials: 'include',
    headers: { 'X-Orbit-Contract': CONTRACT_ID },
  })
  if (response.status === 404) return null
  if (!response.ok) throw new Error(`preview: ${response.status}`)
  return (await response.json()) as FilePreview
}

/**
 * The preview of a chat file, asked for again while the server makes it. `request`: a file from before previews
 * (status `none`) gets one queued (when it is opened in the viewer, not just shown in a list).
 */
export function useFilePreview(file: Pick<PreviewFile, 'url' | 'preview'>, request = false) {
  const initial = file.preview ?? null
  const query = useQuery({
    queryKey: ['file-preview', file.url, request],
    enabled: initial !== null && (initial.status === 'pending' || (request && initial.status === 'none')),
    queryFn: () => previewRequest(file.url, request && initial?.status === 'none' ? 'POST' : 'GET'),
    refetchInterval: (current) => {
      const status = current.state.data?.status
      const waiting = status === undefined || status === 'pending'
      return waiting && current.state.dataUpdateCount < PREVIEW_POLL_LIMIT ? PREVIEW_POLL_MS : false
    },
    staleTime: PREVIEW_POLL_MS,
  })
  return query.data ?? initial
}
