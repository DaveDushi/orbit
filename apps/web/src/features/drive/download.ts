/**
 * Drive downloads go from the bucket to the browser with 5-minute presigned URLs. A folder or a selection is zipped
 * in the browser (client-zip, no compression), file by file: each URL is asked for just before its file is read, so
 * a long ZIP does not outlive its URLs. Where the browser can save a stream (Chromium), the ZIP goes straight to
 * disk; elsewhere it is built in memory first.
 */
import { makeZip } from 'client-zip'
import type { DriveNode } from '@orbit/api/generated/types.gen'
import { uniqueZipPaths, zipPath } from '@orbit/api/drive'
import { downloadUrl, downloadUrls, filesBelow } from './api'

/** URLs asked for at once while zipping. */
const URL_BATCH = 20

/** Saves one file under its name. */
export async function downloadFile(workspaceId: string, nodeId: string) {
  const url = await downloadUrl(workspaceId, nodeId)
  const link = document.createElement('a')
  link.href = url
  link.rel = 'noopener'
  document.body.append(link)
  link.click()
  link.remove()
}

interface ZipFile {
  path: string
  node: DriveNode
}

/** The files to zip: folders with everything in them (under the folder's name), files at the top. */
async function zipFiles(workspaceId: string, nodes: DriveNode[]): Promise<ZipFile[]> {
  const entries: { path: string; node: DriveNode }[] = []
  for (const node of nodes) {
    if (node.kind === 'file') {
      entries.push({ path: zipPath('', { folder_path: '', node }), node })
    } else {
      for (const entry of await filesBelow(workspaceId, node.id)) {
        entries.push({ path: zipPath(node.name, entry), node: entry.node })
      }
    }
  }
  const paths = uniqueZipPaths(entries.map((entry) => entry.path))
  return entries.map((entry, index) => ({ ...entry, path: paths[index] ?? entry.path }))
}

async function* zipInputs(workspaceId: string, files: ZipFile[]) {
  for (let start = 0; start < files.length; start += URL_BATCH) {
    const batch = files.slice(start, start + URL_BATCH)
    const urls = await downloadUrls(workspaceId, batch.map((file) => file.node.id))
    for (const file of batch) {
      let url = urls.get(file.node.id)
      if (!url) continue
      let response = await fetch(url)
      // The URL expired while earlier files were read: ask again.
      if (response.status === 403) {
        url = await downloadUrl(workspaceId, file.node.id)
        response = await fetch(url)
      }
      if (!response.ok || !response.body) throw new Error(`Could not read “${file.node.name}” (${response.status}).`)
      yield { name: file.path, input: response.body, lastModified: new Date(file.node.updated_at), size: file.node.size ?? undefined }
    }
  }
}

interface SaveFilePicker {
  showSaveFilePicker?: (options: { suggestedName: string; types?: { description: string; accept: Record<string, string[]> }[] }) => Promise<{
    createWritable: () => Promise<WritableStream<Uint8Array>>
  }>
}

/** Zips the folders and files and saves the ZIP. Returns the number of files, or `null` when the save was cancelled. */
export async function downloadZip(workspaceId: string, nodes: DriveNode[], zipName: string): Promise<number | null> {
  const suggestedName = `${zipName}.zip`
  const picker = (window as unknown as SaveFilePicker).showSaveFilePicker
  let target: WritableStream<Uint8Array> | null = null
  if (picker) {
    try {
      const handle = await picker({ suggestedName, types: [{ description: 'ZIP archive', accept: { 'application/zip': ['.zip'] } }] })
      target = await handle.createWritable()
    } catch (error) {
      if (error instanceof DOMException && error.name === 'AbortError') return null
      target = null
    }
  }
  const files = await zipFiles(workspaceId, nodes)
  const stream = makeZip(zipInputs(workspaceId, files), { buffersAreUTF8: true })
  if (target) {
    await stream.pipeTo(target)
  } else {
    const blob = await new Response(stream).blob()
    const url = URL.createObjectURL(blob)
    const link = document.createElement('a')
    link.href = url
    link.download = suggestedName
    document.body.append(link)
    link.click()
    link.remove()
    setTimeout(() => URL.revokeObjectURL(url), 60_000)
  }
  return files.length
}
