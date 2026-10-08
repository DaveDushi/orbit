import { Download } from 'reicon-react'
import type { BackupSummary } from '@/api/generated/types.gen'
import { SettingsCard } from '@/components/common/SettingsCard'
import { Badge } from '@/components/ui/badge'
import { Button, buttonVariants } from '@/components/ui/button'
import { useBackups, useCreateBackup } from '@/features/admin/api'
import { SettingsRow } from '@/features/settings/components/SettingsParts'
import { formatSize } from '@/lib/attachmentLib'

/** Snapshots of the whole instance: every workspace, account and attachment. */
export function AdminBackupsPage() {
  const backups = useBackups()
  const backup = useCreateBackup()
  return (
    <SettingsCard
      title="Backups"
      description="Create a verified snapshot of the database and attachments without stopping Orbit."
      actions={
        <Button disabled={backup.isPending} onClick={() => backup.mutate()}>
          {backup.isPending ? 'Creating backup…' : 'Create backup now'}
        </Button>
      }
      flush
    >
      <div className="flex flex-col divide-y text-[13px]">
        {backup.isError ? <SettingsRow role="alert"><span className="text-destructive">Backup failed.</span></SettingsRow> : null}
        {backups.isError ? <SettingsRow role="alert">Backups could not be loaded. <Button variant="ghost" onClick={() => void backups.refetch()}>Retry</Button></SettingsRow> : null}
        {backups.data?.length === 0 ? <SettingsRow className="text-muted-foreground">No backups yet.</SettingsRow> : null}
        {backups.data?.map((item) => <BackupRow key={item.id} backup={item} />)}
      </div>
    </SettingsCard>
  )
}

function BackupRow({ backup }: { backup: BackupSummary }) {
  const created = new Date(backup.created_at)
  return (
    <SettingsRow className="flex-nowrap">
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="flex items-center gap-2 font-medium text-foreground">
          <time dateTime={created.toISOString()}>{created.toLocaleString()}</time>
          {backup.kind === 'pre_migration' ? <Badge variant="secondary">Before upgrade</Badge> : null}
        </span>
        <span className="text-xs text-muted-foreground/70">
          {formatSize(backup.byte_size)} · {backup.file_count} {backup.file_count === 1 ? 'file' : 'files'} · Orbit {backup.application_version}
        </span>
      </div>
      <a
        href={`/api/v1/admin/backups/${encodeURIComponent(backup.id)}/download`}
        download
        className={buttonVariants({ variant: 'outline', size: 'sm' })}
        aria-label={`Download backup from ${created.toLocaleString()}`}
      >
        <Download className="size-4" />
        Download
      </a>
    </SettingsRow>
  )
}
