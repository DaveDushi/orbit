import { useQuery } from '@tanstack/react-query'
import type { DriveNode } from '@orbit/api/generated/types.gen'
import { driveFileKind } from '@orbit/api/drive'
import { FileViewer, PreviewBody } from '@/components/common/FileViewer'
import { previewView, type PreviewView } from '@/lib/filePreview'
import { formatBytes } from '@/lib/formatBytes'
import { driveKeys, downloadUrl, requestPreview } from '../api'
import { NodeIcon } from './NodeIcon'

/** A 5-minute URL: it is asked for again before it runs out. */
const URL_STALE_MS = 4 * 60 * 1000
/** How often a preview that the server is still making is asked for again. */
const PREVIEW_POLL_MS = 4000

interface DriveViewerProps {
  workspaceId: string
  /** The files of the list it opened from, in list order: arrows go to the previous and next one. */
  files: DriveNode[]
  startId: string
  onDownload: (node: DriveNode) => void
  onClose: () => void
}

/**
 * Full-screen preview: images, videos (HTML5 player, with Range from the bucket; the server's playable copy once it
 * is made), audio, PDFs (pdf.js) and office files (the server's PDF of them).
 */
export function DriveViewer({ workspaceId, files, startId, onDownload, onClose }: DriveViewerProps) {
  return (
    <FileViewer
      files={files}
      startIndex={files.findIndex((file) => file.id === startId)}
      title={(node) => node.name}
      details={(node) =>
        [node.size != null ? formatBytes(node.size) : '', node.media?.width && node.media.height ? `${node.media.width} × ${node.media.height}` : '']
          .filter(Boolean)
          .join(' · ')}
      renderBody={(node) => <DriveBody key={node.id} workspaceId={workspaceId} node={node} onDownload={() => onDownload(node)} />}
      onDownload={onDownload}
      onClose={onClose}
    />
  )
}

function DriveBody({ workspaceId, node, onDownload }: { workspaceId: string; node: DriveNode; onDownload: () => void }) {
  // A file from before previews gets its preview queued when it is opened; it is asked for again while it is made.
  const preview = useQuery({
    queryKey: [...driveKeys.all(workspaceId), 'preview', node.id],
    enabled: node.preview != null && (node.preview.status === 'none' || node.preview.status === 'pending'),
    queryFn: () => requestPreview(workspaceId, node.id),
    refetchInterval: (query) => (query.state.data?.preview?.status === 'pending' ? PREVIEW_POLL_MS : false),
  })
  const current = preview.data?.preview ?? node.preview
  const kind = driveFileKind(node)
  const video = kind === 'video' || current?.kind === 'video'
  const original = video || kind === 'image' || kind === 'audio' || kind === 'pdf'
  const url = useQuery({
    queryKey: [...driveKeys.all(workspaceId), 'inline', node.id],
    enabled: original,
    staleTime: URL_STALE_MS,
    queryFn: () => downloadUrl(workspaceId, node.id, true),
  })
  let view: PreviewView | 'loading' | 'error'
  const poster = node.poster_url ?? current?.poster_url ?? undefined
  if (video && current?.status === 'ready' && current.play_url) {
    view = { kind: 'video', src: current.play_url, poster }
  } else if (original) {
    // A video plays its original while the server makes the playable copy, or when it makes none.
    if (url.isError) view = 'error'
    else if (!url.data) view = 'loading'
    else if (video) view = { kind: 'video', src: url.data, poster }
    else view = previewView({ mimeType: node.mime ?? '', fileName: node.name, url: url.data })
  } else {
    view = previewView({ mimeType: node.mime ?? '', fileName: node.name, url: '', preview: current })
  }
  return (
    <PreviewBody
      view={view}
      name={node.name}
      icon={<NodeIcon node={node} className="size-16" />}
      onDownload={onDownload}
      onPdfError={() => void url.refetch()}
    />
  )
}
