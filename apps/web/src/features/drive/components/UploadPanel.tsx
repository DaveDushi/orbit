import { useRef, useState } from 'react'
import { toast } from 'sonner'
import { CloseCircle, TickCircle } from 'reicon-react'
import { cancelDriveUpload } from '@orbit/api/generated/sdk.gen'
import type { DriveUploadView } from '@orbit/api/generated/types.gen'
import { sameFile } from '@orbit/api/drive'
import { apiClient } from '@/api/client'
import { Button } from '@/components/ui/button'
import { Progress } from '@/components/ui/progress'
import { formatBytes } from '@/lib/formatBytes'
import { useDriveUploads, useRefreshDrive } from '../api'
import { uploadQueue, useUploads, type UploadItem } from '../uploads'

/** The uploads of this tab, and the caller's unfinished uploads from earlier (resume with the same file). */
export function UploadPanel({ workspaceId }: { workspaceId: string }) {
  const items = useUploads().filter((item) => item.workspaceId === workspaceId)
  const unfinished = useDriveUploads(workspaceId)
  const refresh = useRefreshDrive(workspaceId)
  const active = new Set(items.filter((item) => item.status === 'queued' || item.status === 'uploading').map((item) => item.uploadId))
  const paused = (unfinished.data ?? []).filter((upload) => !active.has(upload.id))
  const [collapsed, setCollapsed] = useState(false)
  if (items.length === 0 && paused.length === 0) return null

  const running = items.filter((item) => item.status === 'uploading' || item.status === 'queued').length
  const title = running > 0 ? `Uploading ${running} ${running === 1 ? 'file' : 'files'}` : paused.length > 0 && items.length === 0 ? 'Unfinished uploads' : 'Uploads'
  return (
    <section aria-label="Uploads" className="fixed right-4 bottom-4 z-40 flex w-96 max-w-[calc(100vw-2rem)] flex-col overflow-hidden rounded-lg border bg-popover shadow-lg">
      <div className="flex h-10 items-center gap-2 border-b px-3">
        <span className="min-w-0 flex-1 truncate text-[13px] font-semibold">{title}</span>
        {running === 0 && items.length > 0 ? (
          <Button type="button" variant="ghost" size="xs" onClick={() => uploadQueue.clearFinished()}>Clear</Button>
        ) : null}
        <Button type="button" variant="ghost" size="xs" onClick={() => setCollapsed((value) => !value)}>{collapsed ? 'Show' : 'Hide'}</Button>
      </div>
      {collapsed ? null : (
        <div className="max-h-72 overflow-y-auto">
          {items.map((item) => <UploadRow key={item.key} item={item} />)}
          {paused.map((upload) => (
            <PausedRow key={upload.id} workspaceId={workspaceId} upload={upload} onCancelled={refresh} />
          ))}
        </div>
      )}
    </section>
  )
}

function UploadRow({ item }: { item: UploadItem }) {
  const percent = item.size === 0 ? 100 : Math.floor((item.sent / item.size) * 100)
  return (
    <div className="flex flex-col gap-1 border-b px-3 py-2 last:border-b-0" data-upload-status={item.status}>
      <div className="flex min-w-0 items-center gap-2">
        <span className="min-w-0 flex-1 truncate text-[13px]">{item.name}</span>
        {item.status === 'done' ? <TickCircle className="size-4 text-emerald-500" aria-label="Done" /> : null}
        {item.status === 'queued' || item.status === 'uploading' ? (
          <Button type="button" variant="ghost" size="icon-xs" aria-label={`Cancel ${item.name}`} onClick={() => uploadQueue.cancel(item.key)}>
            <CloseCircle className="size-4" />
          </Button>
        ) : null}
      </div>
      {item.status === 'uploading' || item.status === 'queued' ? (
        <div className="flex items-center gap-2">
          <Progress value={percent} className="flex-1" aria-label={`${item.name} progress`} />
          <span className="w-28 shrink-0 text-right text-xs text-muted-foreground tabular-nums">{formatBytes(item.sent)} / {formatBytes(item.size)}</span>
        </div>
      ) : null}
      {item.status === 'failed' ? <p className="text-xs text-destructive">{item.error}</p> : null}
      {item.status === 'cancelled' ? <p className="text-xs text-muted-foreground">Cancelled</p> : null}
    </div>
  )
}

function PausedRow({ workspaceId, upload, onCancelled }: { workspaceId: string; upload: DriveUploadView; onCancelled: () => void }) {
  const input = useRef<HTMLInputElement>(null)
  return (
    <div className="flex flex-col gap-1 border-b px-3 py-2 last:border-b-0" data-upload-status="paused">
      <div className="flex min-w-0 items-center gap-2">
        <span className="min-w-0 flex-1 truncate text-[13px]">{upload.name}</span>
        <span className="text-xs text-muted-foreground">{formatBytes(upload.size)}</span>
      </div>
      <div className="flex items-center gap-2">
        <span className="flex-1 text-xs text-muted-foreground">Paused. Pick the same file to go on.</span>
        <Button type="button" variant="outline" size="xs" onClick={() => input.current?.click()}>Resume</Button>
        <Button
          type="button"
          variant="ghost"
          size="xs"
          onClick={() =>
            void cancelDriveUpload({ client: apiClient, path: { workspace_id: workspaceId, upload_id: upload.id }, throwOnError: true })
              .catch(() => toast.error('Could not cancel the upload.'))
              .finally(onCancelled)
          }
        >
          Cancel
        </Button>
      </div>
      <input
        ref={input}
        type="file"
        hidden
        onChange={(event) => {
          const file = event.target.files?.[0]
          event.target.value = ''
          if (!file) return
          if (!sameFile(upload, file)) {
            toast.error(`Pick “${upload.name}” (${formatBytes(upload.size)}) to resume this upload.`)
            return
          }
          uploadQueue.resume(workspaceId, upload, file)
        }}
      />
    </div>
  )
}
