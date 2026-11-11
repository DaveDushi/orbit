import type { DriveFileReference, DriveNode } from '@orbit/api/generated/types.gen'
import { driveFileKind } from '@orbit/api/drive'
import type { DriveFileRef } from '../api/driveFiles'

/** What a `driveFile` block shows (kept pure for tests). */
export type DriveFileView =
  | { kind: 'placeholder' }
  | { kind: 'loading' }
  | { kind: 'locked' }
  | { kind: 'unavailable' }
  | { kind: 'image' | 'video'; file: DriveNode; url: string }
  | { kind: 'card'; file: DriveNode }

/** Maps the server's answer for one reference to the view: images and videos inline, other files as a card. */
export function driveFileView(ref: DriveFileRef, answer: { data?: DriveFileReference; isError: boolean; hosted: boolean }): DriveFileView {
  if (!answer.hosted || !ref.nodeId) return { kind: 'placeholder' }
  // A page the reader cannot read any more, or a failed check: show nothing about the file.
  if (answer.isError) return { kind: 'unavailable' }
  const data = answer.data
  if (!data) return { kind: 'loading' }
  if (data.status === 'locked') return { kind: 'locked' }
  if (data.status !== 'available' || !data.file) return { kind: 'unavailable' }
  const kind = driveFileKind(data.file)
  if ((kind === 'image' || kind === 'video') && data.url) return { kind, file: data.file, url: data.url }
  return { kind: 'card', file: data.file }
}
