import * as DocumentPicker from 'expo-document-picker'
import * as ImagePicker from 'expo-image-picker'

import type { DriveFile } from './data/drive'

export type DriveSource = 'photos' | 'files'

/** The web preview: the picker's `uri` is a blob/data URL the browser can read back. */
async function driveFile(uri: string, name: string, mimeType: string | undefined): Promise<DriveFile> {
  const blob = await (await fetch(uri)).blob()
  return { name, mimeType: mimeType ?? blob.type, size: blob.size, read: async (start, end) => new Uint8Array(await blob.slice(start, end).arrayBuffer()) }
}

/** The browser's file dialog. */
export async function pickDriveFiles(source: DriveSource): Promise<DriveFile[]> {
  if (source === 'photos') {
    const result = await ImagePicker.launchImageLibraryAsync({ mediaTypes: ['images', 'videos'], allowsMultipleSelection: true })
    if (result.canceled) return []
    return Promise.all(result.assets.map((asset, index) => driveFile(asset.uri, asset.fileName ?? `photo-${index + 1}.png`, asset.mimeType)))
  }
  const result = await DocumentPicker.getDocumentAsync({ multiple: true })
  if (result.canceled) return []
  return Promise.all(result.assets.map((asset) => driveFile(asset.uri, asset.name, asset.mimeType)))
}

/** A shared file (Share to Orbit), as a Drive upload. */
export function driveFileOf(uri: string, name: string, mimeType: string | undefined): Promise<DriveFile> {
  return driveFile(uri, name, mimeType)
}
