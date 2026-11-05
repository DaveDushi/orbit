/**
 * Orbit Drive logic that web and mobile share: which parts of an upload are left to send, the folders a folder
 * upload creates, and the paths inside a ZIP. The bytes never go through Orbit: the server gives presigned URLs.
 */
import type { DriveEntry, DriveNode, DriveUploadView, DriveUploadedPart } from './generated/types.gen'

export type { DriveEntry, DriveNode, DriveUploadView }

/** The byte range `[start, end)` of a part (numbers from 1). Every part but the last has `partSize` bytes. */
export function partRange(partNumber: number, partSize: number, size: number): [number, number] {
  const start = (partNumber - 1) * partSize
  return [start, Math.min(size, start + partSize)]
}

/**
 * The part numbers still to send: the bucket does not have them, or has them with a size other than planned (a part
 * that broke off). After a reload this is what a resumed upload sends.
 */
export function missingParts(upload: Pick<DriveUploadView, 'part_count' | 'part_size' | 'size'>, uploaded: readonly DriveUploadedPart[]): number[] {
  const partSize = upload.part_size ?? upload.size
  const have = new Map(uploaded.map((part) => [part.part_number, part.size]))
  const missing: number[] = []
  for (let number = 1; number <= upload.part_count; number += 1) {
    const [start, end] = partRange(number, partSize, upload.size)
    if (have.get(number) !== end - start) missing.push(number)
  }
  return missing
}

/** Whether a picked file is the one an unfinished upload was started for. */
export function sameFile(upload: Pick<DriveUploadView, 'name' | 'size'>, file: { name: string; size: number }): boolean {
  return upload.name === file.name.trim() && upload.size === file.size
}

/** The folders that a folder upload creates, parents before children, from the files' relative paths (`a/b/c.txt`). */
export function foldersOf(relativePaths: readonly string[]): string[] {
  const folders = new Set<string>()
  for (const path of relativePaths) {
    const parts = path.split('/').filter((part) => part !== '')
    for (let depth = 1; depth < parts.length; depth += 1) folders.add(parts.slice(0, depth).join('/'))
  }
  return [...folders].sort((left, right) => left.split('/').length - right.split('/').length || left.localeCompare(right))
}

/** The folder part of a relative path (`''` for a file at the top). */
export function folderOf(relativePath: string): string {
  const parts = relativePath.split('/').filter((part) => part !== '')
  return parts.slice(0, -1).join('/')
}

/**
 * A name that is safe in a ZIP on every system: no characters that Windows refuses, no trailing dots or spaces, no
 * names that Windows reserves.
 */
export function zipSafeName(name: string): string {
  let safe = name.replace(/[<>:"\\|?*]/g, '_').replace(/[\u0000-\u001f]/g, '_').replace(/[. ]+$/, '')
  if (safe === '' || safe === '.' || safe === '..') safe = '_'
  if (/^(con|prn|aux|nul|com\d|lpt\d)(\..*)?$/i.test(safe)) safe = `_${safe}`
  return safe
}

/** The path of a file inside a ZIP: its folder path below the zipped folder (`''` at its top), under `root`. */
export function zipPath(root: string, entry: Pick<DriveEntry, 'folder_path'> & { node: Pick<DriveNode, 'name'> }): string {
  return [root, ...entry.folder_path.split('/'), entry.node.name]
    .filter((part) => part !== '')
    .map(zipSafeName)
    .join('/')
}

/**
 * Paths made unique in a ZIP: names that differ in a way that `zipSafeName` removes, or only in case, would
 * overwrite each other when extracted.
 */
export function uniqueZipPaths(paths: readonly string[]): string[] {
  const used = new Set<string>()
  return paths.map((path) => {
    let candidate = path
    for (let number = 2; used.has(candidate.toLowerCase()); number += 1) {
      const slash = path.lastIndexOf('/')
      const name = path.slice(slash + 1)
      const dot = name.lastIndexOf('.')
      const [stem, extension] = dot > 0 ? [name.slice(0, dot), name.slice(dot)] : [name, '']
      candidate = `${path.slice(0, slash + 1)}${stem} (${number})${extension}`
    }
    used.add(candidate.toLowerCase())
    return candidate
  })
}

/** What kind of file a MIME type and name are, for icons and filters. */
export type DriveFileKind = 'folder' | 'image' | 'video' | 'audio' | 'pdf' | 'archive' | 'document' | 'other'

export function driveFileKind(node: Pick<DriveNode, 'kind' | 'mime' | 'name'>): DriveFileKind {
  if (node.kind === 'folder') return 'folder'
  const mime = node.mime ?? ''
  if (mime.startsWith('image/')) return 'image'
  if (mime.startsWith('video/')) return 'video'
  if (mime.startsWith('audio/')) return 'audio'
  if (mime === 'application/pdf') return 'pdf'
  if (/zip|x-7z|x-rar|x-tar|gzip/.test(mime)) return 'archive'
  if (mime.startsWith('text/') || /word|excel|spreadsheet|presentation|powerpoint|opendocument|rtf|csv/.test(mime)) return 'document'
  return 'other'
}

/** The long side of a thumbnail and of a video's poster, in pixels. */
export const THUMB_PIXELS = 480
export const POSTER_PIXELS = 1280

/** The size of an image scaled down so its long side is at most `max` (never scaled up), in whole pixels. */
export function scaledSize(width: number, height: number, max: number): { width: number; height: number } {
  const scale = Math.min(1, max / Math.max(width, height, 1))
  return { width: Math.max(1, Math.round(width * scale)), height: Math.max(1, Math.round(height * scale)) }
}

/**
 * Whether a browser should make this file's thumbnail now: an image or video without one yet (also when an earlier
 * try stopped before it finished). A file the browser could not read (`failed`) is not tried again.
 */
export function needsThumbnail(node: Pick<DriveNode, 'kind' | 'mime' | 'name' | 'media'>): boolean {
  const kind = driveFileKind(node)
  if (kind !== 'image' && kind !== 'video') return false
  return node.media == null || node.media.status === 'pending'
}
