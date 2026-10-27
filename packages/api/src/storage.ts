import type { StorageFile, StorageFileUse } from './generated/types.gen'

/** The name and the "where" line of a file in the Storage card's largest files. */
export function storageFileLabel(file: StorageFile): { name: string; where: string } {
  if (file.uses.length === 0) return { name: file.name ?? 'Deleted file', where: 'Deleted · stops counting within a day' }
  const places = [...new Set(file.uses.map(usePlace))]
  return { name: file.name ?? 'Private file', where: places.join(', ') }
}

function usePlace(use: StorageFileUse): string {
  switch (use.kind) {
    case 'task': return `Task “${use.title || 'Untitled'}”`
    case 'page': return `Page “${use.title || 'Untitled'}”`
    case 'channel': return `#${use.title ?? ''}`
    case 'emoji': return 'Custom emoji'
    case 'sticker': return 'Sticker'
    case 'private': return 'Private page or chat'
  }
}
