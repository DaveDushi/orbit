// Port of the chat reference Attachments: image grid (1 / 2 / 3-hero / 4+N) that opens the ImageViewer,
// plus file cards for non-images (paperclip, name, size, download on hover). Self-contained Tailwind styling.
import { useState } from 'react'
import { Download, Expand, Paperclip2 as Paperclip, Play, Xmark as X } from 'reicon-react'
import { formatSize, isImage, type Attachment } from '@/lib/attachmentLib'
import { hasViewer, isSheetFile, isVideoFile, previewView, useFilePreview } from '@/lib/filePreview'
import { FileViewer, PreviewBody } from './FileViewer'
import { SheetPreview } from './SheetPreview'
import { ImageViewer } from './ImageViewer'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { Tip } from '@/components/common/Tip'
import { cn } from 'cn'

const SINGLE_IMAGE_MAX_HEIGHT = 300

/** Dark round × over an attachment. With a mouse it shows on hover or keyboard focus; on touch it is always visible.
    `after:` grows the 22px circle to a 30px hit area. */
function RemoveAttachmentButton({ attachment, onRemove }: { attachment: Attachment; onRemove: (attachmentId: string) => void }) {
  return (
    <Tip label="Remove">
      <Button
        data-slot="attachment-remove"
        variant="ghost"
        size="icon-xs"
        className="absolute top-1.5 right-1.5 z-[2] size-[22px] rounded-full bg-black/60 text-white transition-[opacity,background-color] duration-150 after:absolute after:-inset-1 hover:bg-destructive hover:text-white focus-visible:opacity-100 hover-fine:opacity-0 hover-fine:group-hover:opacity-100 dark:hover:bg-destructive"
        aria-label={`Remove ${attachment.fileName}`}
        onClick={() => onRemove(attachment.id)}
      >
        <X className="size-3" />
      </Button>
    </Tip>
  )
}

export function Attachments({
  attachments,
  hasTextContent = true,
  onRemove,
  previews = false,
}: {
  attachments: Attachment[]
  hasTextContent?: boolean
  /** When given, every attachment gets a remove button. */
  onRemove?: (attachmentId: string) => void
  /**
   * Chat files: videos play in the list (the server's playable copy once it is made), and PDFs, office files and
   * videos open in the full-screen viewer.
   */
  previews?: boolean
}) {
  const [viewerImage, setViewerImage] = useState<Attachment | null>(null)
  const [viewerFile, setViewerFile] = useState<Attachment | null>(null)
  if (attachments.length === 0) return null

  const images = attachments.filter((att) => isImage(att) && !att.deleted)
  const videos = previews ? attachments.filter((att) => !att.deleted && !isImage(att) && isVideoFile(att)) : []
  const otherFiles = attachments.filter((att) => (!isImage(att) || att.deleted) && !videos.includes(att))
  const viewable = previews ? attachments.filter((att) => !att.deleted && !isImage(att) && hasViewer(att)) : []
  const single = images.length === 1

  return (
    <div className={cn('flex flex-col gap-2', hasTextContent ? 'mt-2' : 'mt-1')} data-no-text={hasTextContent ? undefined : 'true'}>
      {images.length > 0 ? (
        <div
          className={cn('grid max-w-[512px] gap-1', images.length === 3 ? 'grid-cols-3' : 'grid-cols-2')}
          data-count={Math.min(images.length, 4)}
        >
          {images.slice(0, 4).map((att, index) => {
            const hero = images.length === 3 && index === 0
            return (
              <div
                key={att.id}
                className={cn(
                  'group relative overflow-hidden rounded-md border border-border transition-opacity hover:opacity-90',
                  single ? 'col-span-2 aspect-auto w-fit' : 'aspect-square',
                  hero && 'col-span-2 row-span-2',
                )}
                data-single={single || undefined}
                data-hero={hero ? 'true' : undefined}
              >
                <Button type="button" variant="ghost" className="block size-full cursor-pointer rounded-none border-0 p-0 hover:bg-transparent dark:hover:bg-transparent active:not-aria-[haspopup]:translate-y-0" aria-label={`Open image ${att.fileName}`} onClick={() => setViewerImage(att)}>
                  <img
                    src={att.url}
                    alt={att.fileName}
                    loading={att.url.startsWith('data:') ? 'eager' : 'lazy'}
                    width={att.width}
                    height={att.height}
                    // a single image with a known size holds its place before it loads (at most 300px high)
                    style={single && att.width && att.height
                      ? { aspectRatio: `${att.width} / ${att.height}`, width: Math.min(att.width, Math.round((SINGLE_IMAGE_MAX_HEIGHT * att.width) / att.height)) }
                      : undefined}
                    className={single ? 'block h-auto max-h-[300px] w-auto max-w-full object-contain' : 'block size-full object-cover'}
                  />
                </Button>
                {images.length > 4 && index === 3 ? (
                  <div className="pointer-events-none absolute inset-0 flex items-center justify-center bg-black/60 text-lg font-semibold text-white">+{images.length - 4}</div>
                ) : null}
                {onRemove ? (
                  <RemoveAttachmentButton attachment={att} onRemove={onRemove} />
                ) : null}
              </div>
            )
          })}
        </div>
      ) : null}

      {videos.length > 0 ? (
        <div className="flex max-w-md flex-col gap-2">
          {videos.map((att) => (
            <span key={att.id} className="group relative block">
              <InlineVideo attachment={att} onOpen={() => setViewerFile(att)} />
              {onRemove ? <RemoveAttachmentButton attachment={att} onRemove={onRemove} /> : null}
            </span>
          ))}
        </div>
      ) : null}

      {otherFiles.length > 0 ? (
        <div className="flex max-w-md flex-col gap-2">
          {otherFiles.map((att) => (
            <span key={att.id} className="group relative block">
              {!att.deleted && viewable.includes(att) ? (
                <OpenableFileCard attachment={att} onOpen={() => setViewerFile(att)} />
              ) : att.deleted ? (
                <span data-slot="attachment-deleted" className="flex items-center gap-3 rounded-lg border border-dashed border-border p-3 text-muted-foreground">
                  <Paperclip className="size-10 shrink-0 opacity-50" />
                  <span className="flex min-w-0 flex-1 flex-col">
                    <span className="text-sm font-medium">Deleted file</span>
                    <span className="truncate text-xs">{att.fileName} · an admin deleted it</span>
                  </span>
                </span>
              ) : <a href={att.url} download={att.fileName} className="group/card flex items-center gap-3 rounded-lg border border-border bg-background p-3 text-muted-foreground shadow-sm transition-colors hover:bg-muted">
                <Paperclip className="size-10 shrink-0" />
                <span className="flex min-w-0 flex-1 flex-col">
                  <span className="truncate text-sm font-medium text-foreground">{att.fileName}</span>
                  <span className="text-xs text-muted-foreground">{formatSize(att.fileSize)}</span>
                </span>
                <span className="transition-opacity duration-150 group-focus-visible/card:opacity-100 hover-fine:opacity-0 hover-fine:group-hover/card:opacity-100">
                  <Download className="size-5" />
                </span>
              </a>}
              {onRemove ? (
                <RemoveAttachmentButton attachment={att} onRemove={onRemove} />
              ) : null}
            </span>
          ))}
        </div>
      ) : null}

      {viewerImage ? <ImageViewer attachment={viewerImage} onClose={() => setViewerImage(null)} /> : null}
      {viewerFile ? (
        <FileViewer
          files={viewable}
          startIndex={viewable.indexOf(viewerFile)}
          title={(att) => att.fileName}
          details={(att) => formatSize(att.fileSize)}
          renderBody={(att) => <ViewerBody key={att.id} attachment={att} />}
          onDownload={download}
          onClose={() => setViewerFile(null)}
        />
      ) : null}
    </div>
  )
}

/** Saves a file: a file kept in Drive is sent from the bucket with its name. */
function download(att: Attachment) {
  const link = document.createElement('a')
  link.href = att.url
  link.download = att.fileName
  link.rel = 'noopener'
  link.click()
}

/** Name, size and "in Drive" for a file kept in Drive. */
function FileLines({ attachment }: { attachment: Attachment }) {
  return (
    <span className="flex min-w-0 flex-1 flex-col">
      <span className="truncate text-sm font-medium text-foreground">{attachment.fileName}</span>
      <span className="text-xs text-muted-foreground">
        {formatSize(attachment.fileSize)}
        {attachment.driveNodeId ? ' · in Drive' : ''}
      </span>
    </span>
  )
}

/** A file card that opens the viewer (a PDF, an office file); its download button saves it. */
function OpenableFileCard({ attachment, onOpen }: { attachment: Attachment; onOpen: () => void }) {
  return (
    <span data-slot="attachment-openable" className="flex items-center gap-1 rounded-lg border border-border bg-background p-1 text-muted-foreground shadow-sm transition-colors hover:bg-muted">
      <button type="button" className="flex min-w-0 flex-1 items-center gap-3 rounded-md p-2 text-left focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none" aria-label={`Open ${attachment.fileName}`} onClick={onOpen}>
        <Paperclip className="size-10 shrink-0" />
        <FileLines attachment={attachment} />
      </button>
      <Tip label="Download">
        <Button variant="ghost" size="icon" aria-label={`Download ${attachment.fileName}`} onClick={() => download(attachment)}>
          <Download className="size-5" />
        </Button>
      </Tip>
    </span>
  )
}

/**
 * A video in the list: its playable copy (or the original, when no copy is made) with its poster. While the
 * server makes the copy it says so; a video the browser cannot play shows as a file to download.
 */
function InlineVideo({ attachment, onOpen }: { attachment: Attachment; onOpen: () => void }) {
  const preview = useFilePreview(attachment)
  const view = previewView({ ...attachment, preview })
  const [unplayable, setUnplayable] = useState<string | null>(null)
  const poster = preview?.poster_url ?? undefined
  if (view.kind === 'pending') {
    return (
      <span data-slot="attachment-video" data-status="pending" className="relative flex aspect-video w-full max-w-md flex-col items-center justify-center gap-2 overflow-hidden rounded-lg border border-border bg-black text-white/80">
        {poster ? <img src={poster} alt="" className="absolute inset-0 size-full object-cover opacity-40" /> : null}
        <Spinner className="relative" />
        <span className="relative text-xs font-medium">Processing the video…</span>
        <span className="relative max-w-full truncate px-3 text-xs text-white/60">{attachment.fileName}</span>
      </span>
    )
  }
  if (view.kind !== 'video' || unplayable === view.src) {
    return (
      <a href={attachment.url} download={attachment.fileName} data-slot="attachment-video" data-status="unplayable" className="flex items-center gap-3 rounded-lg border border-border bg-background p-3 text-muted-foreground shadow-sm transition-colors hover:bg-muted">
        <Play className="size-10 shrink-0" />
        <FileLines attachment={attachment} />
        <Download className="size-5 shrink-0" />
      </a>
    )
  }
  return (
    <span data-slot="attachment-video" data-status={preview?.status ?? 'original'} className="relative block w-full max-w-md overflow-hidden rounded-lg border border-border bg-black">
      <video
        key={view.src}
        src={view.src}
        poster={view.poster}
        controls
        playsInline
        preload="metadata"
        onError={() => setUnplayable(view.src)}
        className="block max-h-[360px] w-full"
        aria-label={attachment.fileName}
      />
      <Tip label="Open">
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={`Open ${attachment.fileName}`}
          className="absolute top-1.5 left-1.5 bg-black/60 text-white hover:bg-black/80 hover:text-white dark:hover:bg-black/80"
          onClick={onOpen}
        >
          <Expand className="size-4" />
        </Button>
      </Tip>
      {attachment.driveNodeId ? (
        <span className="flex items-center gap-2 border-t border-border bg-background px-3 py-1.5 text-xs text-muted-foreground">
          <span className="min-w-0 flex-1 truncate font-medium text-foreground">{attachment.fileName}</span>
          <span className="shrink-0">{formatSize(attachment.fileSize)} · in Drive</span>
        </span>
      ) : null}
    </span>
  )
}

/**
 * One file in the viewer: a workbook, csv or tsv as a table (its PDF when the server cannot read it), else its
 * preview. A file kept in Drive is too large for the table and keeps its PDF.
 */
function ViewerBody({ attachment }: { attachment: Attachment }) {
  const preview = <PreviewViewerBody attachment={attachment} />
  if (!isSheetFile(attachment.fileName) || attachment.driveNodeId) return preview
  return <SheetPreview contentUrl={`${attachment.url}/content`} fallback={preview} />
}

/** The preview of a file; a file from before previews gets its preview queued when it is opened here. */
function PreviewViewerBody({ attachment }: { attachment: Attachment }) {
  const preview = useFilePreview(attachment, true)
  return (
    <PreviewBody
      view={previewView({ ...attachment, preview })}
      name={attachment.fileName}
      icon={<Paperclip className="size-16 text-white/70" />}
      onDownload={() => download(attachment)}
    />
  )
}
