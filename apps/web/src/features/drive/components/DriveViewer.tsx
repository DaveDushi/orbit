import { lazy, Suspense, useState, type KeyboardEvent, type ReactNode } from 'react'
import { useQuery } from '@tanstack/react-query'
import { ArrowLeft2, ArrowRight2, DocumentDownload, Xmark as X } from 'reicon-react'
import type { DriveNode } from '@orbit/api/generated/types.gen'
import { driveFileKind } from '@orbit/api/drive'
import { cn } from 'cn'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent } from '@/components/ui/dialog'
import { Spinner } from '@/components/ui/spinner'
import { formatBytes } from '@/lib/formatBytes'
import { driveKeys, downloadUrl } from '../api'
import { NodeIcon } from './NodeIcon'

const PdfPages = lazy(() => import('@/features/docs/components/PdfPages'))

/** A 5-minute URL: it is asked for again before it runs out. */
const URL_STALE_MS = 4 * 60 * 1000

interface DriveViewerProps {
  workspaceId: string
  /** The files of the list it opened from, in list order: arrows go to the previous and next one. */
  files: DriveNode[]
  startId: string
  onDownload: (node: DriveNode) => void
  onClose: () => void
}

/** Full-screen preview: images, videos (HTML5 player, with Range from the bucket), audio and PDFs (pdf.js). */
export function DriveViewer({ workspaceId, files, startId, onDownload, onClose }: DriveViewerProps) {
  const [index, setIndex] = useState(() => Math.max(0, files.findIndex((file) => file.id === startId)))
  const [open, setOpen] = useState(true)
  const node = files[index]
  const kind = node ? driveFileKind(node) : 'other'
  const previewable = kind === 'image' || kind === 'video' || kind === 'audio' || kind === 'pdf'
  const url = useQuery({
    queryKey: [...driveKeys.all(workspaceId), 'inline', node?.id ?? ''],
    enabled: node !== undefined && previewable,
    staleTime: URL_STALE_MS,
    queryFn: () => downloadUrl(workspaceId, node?.id ?? '', true),
  })
  const step = (delta: number) => setIndex((current) => Math.min(files.length - 1, Math.max(0, current + delta)))
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === 'ArrowLeft') step(-1)
    if (event.key === 'ArrowRight') step(1)
  }
  if (!node) return null

  let body
  if (!previewable) {
    body = (
      <div className="flex flex-col items-center gap-4 text-center">
        <NodeIcon node={node} className="size-16" />
        <p className="text-sm text-white/70">No preview for this file.</p>
        <Button type="button" variant="outline" onClick={() => onDownload(node)}>
          <DocumentDownload className="size-4" /> Download
        </Button>
      </div>
    )
  } else if (url.isError) {
    body = <p className="text-sm text-white/70">The preview could not load. Try again or download the file.</p>
  } else if (!url.data) {
    body = <Spinner className="text-white/70" />
  } else if (kind === 'image') {
    body = <img key={node.id} src={url.data} alt={node.name} className="max-h-full max-w-full rounded-md object-contain" data-drive-preview="image" />
  } else if (kind === 'video') {
    body = (
      <video
        key={node.id}
        src={url.data}
        poster={node.poster_url ?? undefined}
        controls
        autoPlay
        playsInline
        className="max-h-full max-w-full rounded-md bg-black"
        data-drive-preview="video"
      />
    )
  } else if (kind === 'audio') {
    body = <audio key={node.id} src={url.data} controls autoPlay data-drive-preview="audio" />
  } else {
    body = (
      <div className="h-full w-full max-w-4xl overflow-y-auto rounded-md bg-white/5" data-drive-preview="pdf">
        <Suspense fallback={<Spinner className="text-white/70" />}>
          <PdfPages key={node.id} url={url.data} onError={() => void url.refetch()} />
        </Suspense>
      </div>
    )
  }

  return (
    <Dialog open={open} onOpenChange={(next) => !next && setOpen(false)} onOpenChangeComplete={(next) => !next && onClose()}>
      <DialogContent
        showCloseButton={false}
        aria-label={node.name}
        onKeyDown={onKeyDown}
        className="top-0 right-0 left-0 z-[100] flex h-[var(--app-height,100svh)] max-h-[var(--app-height,100svh)] w-full max-w-none translate-x-0 translate-y-0 flex-col gap-0 overflow-hidden rounded-none bg-black/90 p-0 text-white ring-0 backdrop-blur-sm duration-150 motion-reduce:animate-none sm:max-w-none"
      >
        <div className="flex h-14 shrink-0 items-center justify-between gap-3 border-b border-white/10 px-4">
          <div className="min-w-0">
            <div className="truncate text-sm font-bold">{node.name}</div>
            <div className="text-xs font-semibold text-white/55 tabular-nums">
              {index + 1} / {files.length}
              {node.size != null ? ` · ${formatBytes(node.size)}` : ''}
              {node.media?.width && node.media.height ? ` · ${node.media.width} × ${node.media.height}` : ''}
            </div>
          </div>
          <div className="flex items-center gap-2">
            <BarButton label="Download" onClick={() => onDownload(node)}>
              <DocumentDownload className="size-4" />
            </BarButton>
            <BarButton label="Close preview" onClick={() => setOpen(false)}>
              <X className="size-4" />
            </BarButton>
          </div>
        </div>
        <div className="relative flex min-h-0 flex-1 items-center justify-center p-4 sm:px-16">
          {body}
          <BarButton label="Previous file" className={cn('absolute top-1/2 left-3 -translate-y-1/2', index === 0 && 'invisible')} onClick={() => step(-1)}>
            <ArrowLeft2 className="size-5" />
          </BarButton>
          <BarButton label="Next file" className={cn('absolute top-1/2 right-3 -translate-y-1/2', index === files.length - 1 && 'invisible')} onClick={() => step(1)}>
            <ArrowRight2 className="size-5" />
          </BarButton>
        </div>
      </DialogContent>
    </Dialog>
  )
}

function BarButton({ label, className, onClick, children }: { label: string; className?: string; onClick: () => void; children: ReactNode }) {
  return (
    <Button
      type="button"
      variant="ghost"
      size="icon-lg"
      aria-label={label}
      className={cn('bg-white/10 text-white hover:bg-white/15 hover:text-white dark:hover:bg-white/15', className)}
      onClick={onClick}
    >
      {children}
    </Button>
  )
}
