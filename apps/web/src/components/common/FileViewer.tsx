import { lazy, Suspense, useState, type KeyboardEvent, type ReactNode } from 'react'
import { ArrowLeft2, ArrowRight2, DocumentDownload, Xmark as X } from 'reicon-react'
import { cn } from 'cn'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent } from '@/components/ui/dialog'
import { Spinner } from '@/components/ui/spinner'
import type { PreviewView } from '@/lib/filePreview'

const PdfPages = lazy(() => import('@/features/docs/components/PdfPages'))

interface FileViewerProps<T> {
  /** The files of the list it opened from, in list order: arrows go to the previous and next one. */
  files: T[]
  startIndex: number
  title: (file: T) => string
  /** The line under the name (size, measures). */
  details?: (file: T) => string
  /** The preview of the file on show. */
  renderBody: (file: T) => ReactNode
  onDownload: (file: T) => void
  onClose: () => void
}

/** Full-screen preview of a list of files: name, position, download and close on top, arrows to the next file. */
export function FileViewer<T>({ files, startIndex, title, details, renderBody, onDownload, onClose }: FileViewerProps<T>) {
  const [index, setIndex] = useState(() => Math.min(Math.max(0, startIndex), Math.max(0, files.length - 1)))
  const [open, setOpen] = useState(true)
  const file = files[index]
  const step = (delta: number) => setIndex((current) => Math.min(files.length - 1, Math.max(0, current + delta)))
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === 'ArrowLeft') step(-1)
    if (event.key === 'ArrowRight') step(1)
  }
  if (file === undefined) return null
  const name = title(file)
  const more = details?.(file) ?? ''

  return (
    <Dialog open={open} onOpenChange={(next) => !next && setOpen(false)} onOpenChangeComplete={(next) => !next && onClose()}>
      <DialogContent
        showCloseButton={false}
        aria-label={name}
        onKeyDown={onKeyDown}
        className="top-0 right-0 left-0 z-[100] flex h-[var(--app-height,100svh)] max-h-[var(--app-height,100svh)] w-full max-w-none translate-x-0 translate-y-0 flex-col gap-0 overflow-hidden rounded-none bg-black/90 p-0 text-white ring-0 backdrop-blur-sm duration-150 motion-reduce:animate-none sm:max-w-none"
      >
        <div className="flex h-14 shrink-0 items-center justify-between gap-3 border-b border-white/10 px-4">
          <div className="min-w-0">
            <div className="truncate text-sm font-bold">{name}</div>
            <div className="text-xs font-semibold text-white/55 tabular-nums">
              {index + 1} / {files.length}
              {more ? ` · ${more}` : ''}
            </div>
          </div>
          <div className="flex items-center gap-2">
            <BarButton label="Download" onClick={() => onDownload(file)}>
              <DocumentDownload className="size-4" />
            </BarButton>
            <BarButton label="Close preview" onClick={() => setOpen(false)}>
              <X className="size-4" />
            </BarButton>
          </div>
        </div>
        <div className="relative flex min-h-0 flex-1 items-center justify-center p-4 sm:px-16">
          {renderBody(file)}
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

/** The preview of one file in the viewer. `onUnplayable`: the browser cannot play this video. */
export function PreviewBody({
  view,
  name,
  icon,
  onDownload,
  onPdfError,
}: {
  view: PreviewView | 'loading' | 'error'
  name: string
  icon?: ReactNode
  onDownload: () => void
  onPdfError?: () => void
}) {
  const [unplayable, setUnplayable] = useState<string | null>(null)
  if (view === 'loading') return <Spinner className="text-white/70" />
  if (view === 'error') return <p className="text-sm text-white/70">The preview could not load. Try again or download the file.</p>
  if (view.kind === 'image') {
    return <img key={view.src} src={view.src} alt={name} className="max-h-full max-w-full rounded-md object-contain" data-drive-preview="image" />
  }
  if (view.kind === 'video' && unplayable !== view.src) {
    return (
      <video
        key={view.src}
        src={view.src}
        poster={view.poster}
        controls
        autoPlay
        playsInline
        onError={() => setUnplayable(view.src)}
        className="max-h-full max-w-full rounded-md bg-black"
        data-drive-preview="video"
      />
    )
  }
  if (view.kind === 'audio') return <audio key={view.src} src={view.src} controls autoPlay data-drive-preview="audio" />
  if (view.kind === 'pdf') {
    return (
      <div className="h-full w-full max-w-4xl overflow-y-auto rounded-md bg-white/5" data-drive-preview="pdf">
        <Suspense fallback={<Spinner className="text-white/70" />}>
          <PdfPages key={view.src} url={view.src} onError={() => onPdfError?.()} />
        </Suspense>
      </div>
    )
  }
  if (view.kind === 'pending') {
    return (
      <div className="flex flex-col items-center gap-3 text-center" data-drive-preview="pending">
        <Spinner className="text-white/70" />
        <p className="text-sm text-white/70">{view.label}</p>
      </div>
    )
  }
  const message = view.kind === 'video' ? 'This browser cannot play this video. Download it to watch it.' : view.message
  return (
    <div className="flex flex-col items-center gap-4 text-center" data-drive-preview="unavailable">
      {icon}
      <p className="max-w-sm text-sm text-white/70">{message}</p>
      <Button type="button" variant="outline" onClick={onDownload}>
        <DocumentDownload className="size-4" /> Download
      </Button>
    </div>
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
