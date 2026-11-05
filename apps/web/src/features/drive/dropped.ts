import type { PickedFile } from './uploads'

/** Whether a drag carries files from the computer (not Drive items being moved). */
export function draggingFiles(dataTransfer: DataTransfer | null): boolean {
  return dataTransfer?.types.includes('Files') ?? false
}

/** The files of a drop, with folders read to the end so a dropped folder keeps its tree. */
export async function droppedFiles(dataTransfer: DataTransfer): Promise<PickedFile[]> {
  const entries = [...dataTransfer.items]
    .filter((item) => item.kind === 'file')
    .map((item) => item.webkitGetAsEntry?.())
    .filter((entry): entry is FileSystemEntry => entry !== null && entry !== undefined)
  if (entries.length === 0) return [...dataTransfer.files].map((file) => ({ file }))
  const picked: PickedFile[] = []
  for (const entry of entries) await collect(entry, '', picked)
  return picked
}

async function collect(entry: FileSystemEntry, folder: string, picked: PickedFile[]) {
  const path = folder ? `${folder}/${entry.name}` : entry.name
  if (entry.isFile) {
    const file = await new Promise<File>((resolve, reject) => (entry as FileSystemFileEntry).file(resolve, reject))
    picked.push({ file, relativePath: path })
    return
  }
  const reader = (entry as FileSystemDirectoryEntry).createReader()
  // A directory reader gives its entries in batches until it gives an empty one.
  for (;;) {
    const batch = await new Promise<FileSystemEntry[]>((resolve, reject) => reader.readEntries(resolve, reject))
    if (batch.length === 0) return
    for (const child of batch) await collect(child, path, picked)
  }
}

/** Files picked with a folder input (`webkitdirectory`), with their paths. */
export function pickedFolder(files: FileList): PickedFile[] {
  return [...files].map((file) => ({ file, relativePath: file.webkitRelativePath || file.name }))
}
