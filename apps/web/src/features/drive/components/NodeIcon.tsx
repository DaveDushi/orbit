import { Archive, Document, DocumentText, Folder, Gallery, Music, VideoSquare } from 'reicon-react'
import type { DriveNode } from '@orbit/api/generated/types.gen'
import { driveFileKind, type DriveFileKind } from '@orbit/api/drive'
import { cn } from 'cn'

const ICONS: Record<DriveFileKind, typeof Folder> = {
  folder: Folder,
  image: Gallery,
  video: VideoSquare,
  audio: Music,
  pdf: DocumentText,
  archive: Archive,
  document: DocumentText,
  other: Document,
}

const COLORS: Record<DriveFileKind, string> = {
  folder: 'text-sky-500',
  image: 'text-emerald-500',
  video: 'text-violet-500',
  audio: 'text-amber-500',
  pdf: 'text-rose-500',
  archive: 'text-orange-500',
  document: 'text-blue-500',
  other: 'text-muted-foreground',
}

export function NodeIcon({ node, className }: { node: Pick<DriveNode, 'kind' | 'mime' | 'name'>; className?: string }) {
  const kind = driveFileKind(node)
  const Icon = ICONS[kind]
  return <Icon className={cn('shrink-0', COLORS[kind], className)} aria-hidden="true" />
}
