import { useState, type ReactNode } from 'react'
import { DocumentText, Lock, Trash } from 'reicon-react'
import type { DriveNode } from '@orbit/api/generated/types.gen'
import { cn } from 'cn'
import { Spinner } from '@/components/ui/spinner'
import { formatBytes } from '@/lib/formatBytes'
import { DriveViewer } from '@/features/drive/components/DriveViewer'
import { NodeIcon } from '@/features/drive/components/NodeIcon'
import { downloadUrl } from '@/features/drive/api'
import { useDriveFileRef, type DriveFileRef } from '../api/driveFiles'
import { driveFileView } from './driveFileView'
import { usePageEditorContext } from './pageEditorContext'


/** `render` of the `driveFile` block spec: the Drive file as it is now, for this reader. */
export function DriveFileBlockView({ block }: { block: { props: DriveFileRef } }) {
  const ref = block.props
  const { driveFiles } = usePageEditorContext()
  const query = useDriveFileRef(driveFiles, ref)
  const [previewing, setPreviewing] = useState(false)
  const view = driveFileView(ref, { data: query.data, isError: query.isError, hosted: driveFiles !== null })

  let body: ReactNode
  switch (view.kind) {
    case 'placeholder':
      body = <Card icon={<DocumentText className="size-5" />} title="Drive file" muted />
      break
    case 'loading':
      body = <Card icon={<Spinner className="size-4" />} title="Loading file…" muted />
      break
    case 'locked':
      body = <Card icon={<Lock className="size-5" />} title="Locked file" detail="You do not have access to this Drive file." muted state="locked" />
      break
    case 'unavailable':
      body = <Card icon={<Trash className="size-5" />} title="File not available" detail="It is in the Drive trash or was deleted." muted state="unavailable" />
      break
    case 'image':
      body = (
        <button type="button" className="block max-w-full cursor-zoom-in rounded-md outline-none focus-visible:ring-2 focus-visible:ring-ring/50" aria-label={`Open ${view.file.name}`} onClick={() => setPreviewing(true)}>
          <MediaImage key={`${view.file.id}:${view.file.updated_at}`} file={view.file} url={view.url} onExpired={() => void query.refetch()} />
        </button>
      )
      break
    case 'video':
      body = <MediaVideo key={`${view.file.id}:${view.file.updated_at}`} file={view.file} url={view.url} onExpired={() => void query.refetch()} />
      break
    case 'card':
      body = (
        <Card
          icon={<NodeIcon node={view.file} className="size-5" />}
          title={view.file.name}
          detail={[view.file.size != null ? formatBytes(view.file.size) : null, 'Drive'].filter(Boolean).join(' · ')}
          onOpen={() => setPreviewing(true)}
          state="available"
        />
      )
      break
  }

  return (
    <div
      contentEditable={false}
      data-drive-file={ref.nodeId}
      data-state={view.kind}
      className="w-full py-1 select-none"
      onMouseDown={(event) => {
        // Keep ProseMirror from turning the click into a node selection / drag start.
        event.stopPropagation()
      }}
    >
      {body}
      {previewing && 'file' in view ? (
        <DriveViewer
          workspaceId={ref.workspaceId}
          files={[view.file]}
          startId={view.file.id}
          onDownload={(node) => void saveDriveFile(ref.workspaceId, node.id)}
          onClose={() => setPreviewing(false)}
        />
      ) : null}
    </div>
  )
}

/** Saves the file (without `features/drive/download`, which brings the ZIP writer into the editor's chunk). */
async function saveDriveFile(workspaceId: string, nodeId: string) {
  const link = document.createElement('a')
  link.href = await downloadUrl(workspaceId, nodeId)
  link.rel = 'noopener'
  document.body.append(link)
  link.click()
  link.remove()
}

/** `toExternalHTML` (copy as text, HTML export paths): never the file's name, which another reader may not see. */
export function DriveFileExternalView() {
  return <p>Drive file</p>
}

function Card({
  icon,
  title,
  detail,
  muted,
  state,
  onOpen,
}: {
  icon: ReactNode
  title: string
  detail?: string
  muted?: boolean
  state?: string
  onOpen?: () => void
}) {
  const content = (
    <>
      <span className={cn('flex size-9 shrink-0 items-center justify-center rounded-md bg-muted', muted && 'text-muted-foreground')} aria-hidden>
        {icon}
      </span>
      <span className="flex min-w-0 flex-1 flex-col">
        <span className={cn('truncate text-sm font-medium', muted ? 'text-muted-foreground' : 'text-foreground')}>{title}</span>
        {detail ? <span className="truncate text-xs text-muted-foreground">{detail}</span> : null}
      </span>
    </>
  )
  const className = 'flex w-full max-w-[480px] min-w-0 items-center gap-3 rounded-lg border border-border bg-card px-3 py-2 text-left'
  if (!onOpen) {
    return (
      <div className={className} data-slot="drive-file-card" data-card-state={state}>
        {content}
      </div>
    )
  }
  return (
    <button
      type="button"
      className={cn(className, 'cursor-pointer outline-none transition-colors duration-150 ease-out hover:bg-foreground/[0.04] focus-visible:ring-2 focus-visible:ring-ring/50')}
      data-slot="drive-file-card"
      data-card-state={state}
      onClick={onOpen}
    >
      {content}
    </button>
  )
}

/**
 * The 5-minute URL of a media element: kept while it works, so a refetch does not reload the image or restart the
 * video; after a load error (the URL ran out) the next one is asked for, once.
 */
function useMediaUrl(url: string, refresh: () => void) {
  const [src, setSrc] = useState(url)
  const [state, setState] = useState<'ok' | 'refreshing' | 'retried'>('ok')
  if (state === 'refreshing' && url !== src) {
    setSrc(url)
    setState('retried')
  }
  const onError = () => {
    if (state !== 'ok') return
    setState('refreshing')
    refresh()
  }
  return { src, onError }
}

/** The image as it is in Drive. */
function MediaImage({ file, url, onExpired }: { file: DriveNode; url: string; onExpired: () => void }) {
  const { src, onError } = useMediaUrl(url, onExpired)
  return (
    <img
      src={src}
      alt={file.name}
      width={file.media?.width ?? undefined}
      height={file.media?.height ?? undefined}
      className="h-auto max-h-[560px] w-auto max-w-full rounded-md"
      data-drive-media="image"
      onError={onError}
    />
  )
}

/** The video in an HTML5 player. */
function MediaVideo({ file, url, onExpired }: { file: DriveNode; url: string; onExpired: () => void }) {
  const { src, onError } = useMediaUrl(url, onExpired)
  return (
    <video
      src={src}
      poster={file.poster_url ?? file.thumbnail_url ?? undefined}
      controls
      preload="metadata"
      playsInline
      className="max-h-[560px] w-full max-w-full rounded-md bg-black"
      aria-label={file.name}
      data-drive-media="video"
      onError={onError}
    />
  )
}
