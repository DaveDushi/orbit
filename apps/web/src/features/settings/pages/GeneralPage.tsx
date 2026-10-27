import { useRef, useState } from 'react'
import { cn } from 'cn'
import { Link } from 'react-router'
import { UnsavedBar } from '@/components/common/UnsavedBar'
import { Button } from '@/components/ui/button'
import { Field, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Progress, ProgressLabel } from '@/components/ui/progress'
import { useRenameWorkspace, useWorkspaceStorage } from '@/features/workspaces/api'
import { useCan } from '@/features/workspaces/permissions'
import { useWorkspace } from '@/features/workspaces/workspaceContext'
import { SettingsCard } from '@/components/common/SettingsCard'
import { FieldGrid } from '@/features/settings/components/SettingsParts'
import { formatBytes } from '@/lib/formatBytes'
import { WorkspaceLeave } from '@/features/settings/components/WorkspaceLeave'
import { useSlowPending } from '@/lib/useDebouncedValue'

export function GeneralPage() {
  const { workspace } = useWorkspace()
  const renameWorkspace = useRenameWorkspace(workspace.id)
  const renameSlow = useSlowPending(renameWorkspace.isPending)
  const canRename = useCan('workspace.update')
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
      {canRename ? <SettingsCard title="Workspace" description="Rename this workspace.">
        <form ref={formRef} onSubmit={(event) => { event.preventDefault(); saveWorkspace() }}>
          <FieldGrid>
            <Field><FieldLabel htmlFor="workspace-name">Name</FieldLabel><Input id="workspace-name" required disabled={renameSlow} value={name} onChange={(event) => setNameDraft({ workspaceId: workspace.id, value: event.target.value })} /></Field>
          </FieldGrid>
        </form>
        {renameWorkspace.isError ? <p role="alert" className="text-destructive">Workspace rename failed. <Button variant="ghost" onClick={saveWorkspace}>Retry</Button></p> : null}
        {renameSlow ? <p role="status">Saving workspace…</p> : null}
      </SettingsCard> : null}

      <StorageCard workspaceId={workspace.id} />

      <WorkspaceLeave key={workspace.id} />
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

/** How much of its storage quota the workspace uses. Every member sees it. */
function StorageCard({ workspaceId }: { workspaceId: string }) {
  const storage = useWorkspaceStorage(workspaceId)
  const used = storage.data?.used_bytes ?? 0
  const quota = storage.data?.quota_bytes ?? null
  const percent = quota ? Math.min(100, (used / quota) * 100) : 0
  const canManageStorage = useCan('storage.manage')
  return (
    <SettingsCard
      title="Storage"
      description="Files, emoji and stickers of this workspace. A file attached more than once counts once; a deleted file stops counting about a day after the trash is emptied."
      actions={canManageStorage ? <Button variant="outline" render={<Link to="/settings/file-storage" />} nativeButton={false}>Use your own bucket</Button> : null}
    >
      {storage.isPending ? <p role="status" className="text-[13px] text-muted-foreground">Loading storage…</p> : null}
      {storage.isError ? <p role="alert" className="text-[13px] text-destructive">Storage use could not be loaded.</p> : null}
      {storage.data ? (
        <Progress
          value={quota ? percent : null}
          className={cn('gap-2', !quota && '[&_[data-slot=progress-track]]:hidden', percent >= 90 && '[&_[data-slot=progress-indicator]]:bg-destructive')}
        >
          <ProgressLabel className="text-[13px] text-foreground">Used</ProgressLabel>
          <span className="ml-auto text-[13px] text-muted-foreground tabular-nums">
            {quota ? `${formatBytes(used)} of ${formatBytes(quota)}` : `${formatBytes(used)} · no limit`}
          </span>
        </Progress>
      ) : null}
    </SettingsCard>
  )
}
