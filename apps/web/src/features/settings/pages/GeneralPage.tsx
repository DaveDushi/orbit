import { useRef, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Download } from 'reicon-react'
import { apiClient } from '@/api/client'
import { createBackup, listBackups } from '@/api/generated/sdk.gen'
import type { BackupSummary } from '@/api/generated/types.gen'
import { queryKeys } from '@/api/queryKeys'
import { UnsavedBar } from '@/components/common/UnsavedBar'
import { Badge } from '@/components/ui/badge'
import { Button, buttonVariants } from '@/components/ui/button'
import { formatSize } from '@/lib/attachmentLib'
import { Field, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { useTheme, type Theme } from '@/lib/themeContext'
import { useRenameWorkspace } from '@/features/workspaces/api'
import { useWorkspace } from '@/features/workspaces/workspaceContext'
import { SettingsCard } from '@/components/common/SettingsCard'
import { useSlowPending } from '@/lib/useDebouncedValue'
import { cn } from 'cn'

const THEME_OPTIONS = [
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
] as const

const FIELD_LABEL = 'mb-1.5 h-4 gap-1 text-[13px] leading-4 font-medium text-muted-foreground'
const GRID = 'grid grid-cols-1 gap-4 min-[900px]:grid-cols-2'
// `gap-0` keeps the Field rows at the previous label/control spacing (the label owns its `mb-1.5`).
const FIELD = 'w-full min-w-0 gap-0'
const BACKUP_ROW = 'flex items-center gap-3 px-4 py-3 text-[13px]'

export function GeneralPage() {
  const { theme, setTheme } = useTheme()
  const { workspace } = useWorkspace()
  const renameWorkspace = useRenameWorkspace(workspace.id)
  const renameSlow = useSlowPending(renameWorkspace.isPending)
  const queryClient = useQueryClient()
  const backups = useQuery({
    queryKey: queryKeys.backups,
    queryFn: async () => {
      const { data } = await listBackups({ client: apiClient, throwOnError: true })
      return data.items
    },
  })
  const backup = useMutation({
    mutationFn: async () => {
      const { data } = await createBackup({ client: apiClient, throwOnError: true })
      if (!data) throw new Error('Backup response was empty.')
      return data
    },
    onSettled: () => queryClient.invalidateQueries({ queryKey: queryKeys.backups }),
  })
  const formRef = useRef<HTMLFormElement>(null)
  const [nameDraft, setNameDraft] = useState<{ workspaceId: string; value: string } | null>(null)
  const name = nameDraft?.workspaceId === workspace.id ? nameDraft.value : workspace.name
  const dirty = name.trim() !== workspace.name

  function saveWorkspace() {
    if (!name.trim() || !dirty || renameWorkspace.isPending) return
    renameWorkspace.mutate(
      { name: name.trim(), version: workspace.version },
      { onSuccess: () => setNameDraft(null) },
    )
  }

  return (
    <>
      <SettingsCard title="Workspace" description="Rename this workspace.">
        <form ref={formRef} className={GRID} onSubmit={(event) => { event.preventDefault(); saveWorkspace() }}>
          <Field className={FIELD}><FieldLabel className={FIELD_LABEL} htmlFor="workspace-name">Name</FieldLabel><Input id="workspace-name" required disabled={renameSlow} value={name} onChange={(event) => setNameDraft({ workspaceId: workspace.id, value: event.target.value })} /></Field>
        </form>
        {renameWorkspace.isError ? <p role="alert" className="text-destructive">Workspace rename failed. <Button variant="ghost" onClick={saveWorkspace}>Retry</Button></p> : null}
        {renameSlow ? <p role="status">Saving workspace…</p> : null}
      </SettingsCard>

      <SettingsCard title="Appearance" description="Theme for this browser.">
        <div className={GRID}>
          <Field className={FIELD}>
            <FieldLabel className={FIELD_LABEL} htmlFor="appearance-theme">
              Theme
            </FieldLabel>
            {/* `items` lets Select.Value render the option label instead of the raw value. */}
            <Select items={THEME_OPTIONS} value={theme} onValueChange={(value) => setTheme(value as Theme)}>
              <SelectTrigger id="appearance-theme">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {THEME_OPTIONS.map((option) => <SelectItem key={option.value} value={option.value}>{option.label}</SelectItem>)}
              </SelectContent>
            </Select>
          </Field>
        </div>
      </SettingsCard>

      <SettingsCard title="About" description="Version and backend status of this Orbit instance.">
        <div className={GRID}>
          <Field className={FIELD}>
            <FieldLabel className={FIELD_LABEL} htmlFor="about-version">
              Version
            </FieldLabel>
            <Input id="about-version" value="0.1.0" readOnly />
          </Field>
          <Field className={FIELD}>
            <FieldLabel className={FIELD_LABEL} htmlFor="about-backend">
              Backend
            </FieldLabel>
            <Input
              id="about-backend"
              value="Connected"
              readOnly
            />
          </Field>
          <Field className={FIELD}>
            <FieldLabel className={FIELD_LABEL} htmlFor="about-storage">
              Storage
            </FieldLabel>
            <Input id="about-storage" value="Server data directory" readOnly />
          </Field>
        </div>
      </SettingsCard>
      <SettingsCard
        title="Backup"
        description="Create a verified snapshot of the database and attachments without stopping Orbit."
        actions={
          <Button disabled={backup.isPending} onClick={() => backup.mutate()}>
            {backup.isPending ? 'Creating backup…' : 'Create backup now'}
          </Button>
        }
        flush
      >
        <div className="flex flex-col divide-y divide-border">
          {backup.isError ? <div className={BACKUP_ROW} role="alert"><span className="text-destructive">Backup failed. Installation administrator access is required.</span></div> : null}
          {backups.isPending ? <div className={BACKUP_ROW}>Loading backups…</div> : null}
          {backups.isError ? <div className={BACKUP_ROW} role="alert">Backups could not be loaded. Installation administrator access is required. <Button variant="ghost" onClick={() => void backups.refetch()}>Retry</Button></div> : null}
          {backups.data?.length === 0 ? <div className={cn(BACKUP_ROW, 'text-muted-foreground')}>No backups yet.</div> : null}
          {backups.data?.map((item) => <BackupRow key={item.id} backup={item} />)}
        </div>
      </SettingsCard>
      {dirty ? (
        <UnsavedBar
          onReset={() => { if (renameWorkspace.isPending) return; setNameDraft(null); renameWorkspace.reset() }}
          onSave={() => formRef.current?.requestSubmit()}
          saving={renameWorkspace.isPending}
        />
      ) : null}
    </>
  )
}

function BackupRow({ backup }: { backup: BackupSummary }) {
  const created = new Date(backup.created_at)
  return (
    <div className={BACKUP_ROW}>
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="flex items-center gap-2 font-medium text-foreground">
          <time dateTime={created.toISOString()}>{created.toLocaleString()}</time>
          {backup.kind === 'pre_migration' ? <Badge className="h-auto rounded-full border-0 bg-primary/10 text-[10px] leading-[14px] text-primary">Before upgrade</Badge> : null}
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
    </div>
  )
}
