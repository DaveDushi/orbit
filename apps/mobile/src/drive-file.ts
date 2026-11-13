import * as DocumentPicker from 'expo-document-picker'
import { File } from 'expo-file-system'
import * as ImagePicker from 'expo-image-picker'

import type { DriveFile } from './data/drive'

export type DriveSource = 'photos' | 'files'

/** A picked file whose bytes are read a range at a time from disk (`drive-file.web.ts` in the web preview). */
function driveFile(uri: string, name: string, mimeType: string | undefined): DriveFile {
  const file = new File(uri)
  return {
    name,
    mimeType,
    size: file.size ?? 0,
    read: async (start, end) => {
      const handle = file.open()
      try {
        handle.offset = start
        return await handle.readBytes(end - start)
      } finally {
        handle.close()
      }
    },
  }
}

/** Photos and videos from the library (as they are) or any files, for Drive. `[]` when cancelled. */
export async function pickDriveFiles(source: DriveSource): Promise<DriveFile[]> {
  if (source === 'photos') {
    const result = await ImagePicker.launchImageLibraryAsync({ mediaTypes: ['images', 'videos'], allowsMultipleSelection: true, quality: 1 })
    if (result.canceled) return []
    return result.assets.map((asset, index) => driveFile(asset.uri, asset.fileName ?? `photo-${index + 1}.jpg`, asset.mimeType))
  }
  const result = await DocumentPicker.getDocumentAsync({ multiple: true, copyToCacheDirectory: true })
  if (result.canceled) return []
  return result.assets.map((asset) => driveFile(asset.uri, asset.name, asset.mimeType))
}

/** A shared file on disk (Share to Orbit), as a Drive upload. */
export async function driveFileOf(uri: string, name: string, mimeType: string | undefined): Promise<DriveFile> {
  return driveFile(uri, name, mimeType)
}
