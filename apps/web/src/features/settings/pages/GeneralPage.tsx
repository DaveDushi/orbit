import { useRef, useState } from 'react'
import { ApiProblem } from '@orbit/api/problem'
import { UnsavedBar } from '@/components/common/UnsavedBar'
import { Button } from '@/components/ui/button'
import { Field, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { WorkspaceLogo } from '@/components/common/WorkspaceLogo'
import { avatarImage } from '@/features/profile/avatarImage'
import { useRenameWorkspace, useSetWorkspaceLogo } from '@/features/workspaces/api'
import { useCan } from '@/features/workspaces/permissions'
import { useWorkspace } from '@/features/workspaces/workspaceContext'
import { SettingsCard } from '@/components/common/SettingsCard'
import { FieldGrid } from '@/features/settings/components/SettingsParts'
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
      {canRename ? <SettingsCard title="Workspace" description="The name and logo of this workspace.">
        <LogoField />
        <form ref={formRef} onSubmit={(event) => { event.preventDefault(); saveWorkspace() }}>
          <FieldGrid>
            <Field><FieldLabel htmlFor="workspace-name">Name</FieldLabel><Input id="workspace-name" required disabled={renameSlow} value={name} onChange={(event) => setNameDraft({ workspaceId: workspace.id, value: event.target.value })} /></Field>
          </FieldGrid>
        </form>
        {renameWorkspace.isError ? <p role="alert" className="text-destructive">Workspace rename failed. <Button variant="ghost" onClick={saveWorkspace}>Retry</Button></p> : null}
        {renameSlow ? <p role="status">Saving workspace…</p> : null}
      </SettingsCard> : null}

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

/** The workspace logo with upload and remove, for owners and admins. The image is cropped to a square and scaled in
    the browser first, like a profile picture. */
function LogoField() {
  const { workspace } = useWorkspace()
  const setLogo = useSetWorkspaceLogo(workspace.id)
  const inputRef = useRef<HTMLInputElement>(null)
  const [readError, setReadError] = useState<string | null>(null)

  const pick = async (file: File | undefined) => {
    if (!file) return
    setReadError(null)
    setLogo.reset()
    let image: Blob
    try {
      image = await avatarImage(file)
    } catch {
      setReadError('This image could not be read. Use a PNG, JPEG or WebP file.')
      return
    }
    setLogo.mutate(image)
  }

  const error = readError ?? (setLogo.isError ? (setLogo.error instanceof ApiProblem ? setLogo.error.detail : 'The logo could not be saved.') : null)
  return (
    <div className="mb-4 flex flex-wrap items-center gap-4">
      <WorkspaceLogo workspace={workspace} size={64} />
      <div className="flex min-w-0 flex-col gap-2">
        <div className="flex flex-wrap gap-2">
          <Button type="button" variant="outline" disabled={setLogo.isPending} onClick={() => inputRef.current?.click()}>
            {setLogo.isPending ? 'Saving…' : workspace.logo_url ? 'Change logo' : 'Upload logo'}
          </Button>
          {workspace.logo_url ? (
            <Button type="button" variant="ghost" disabled={setLogo.isPending} onClick={() => setLogo.mutate(null)}>
              Remove
            </Button>
          ) : null}
        </div>
        {error ? (
          <p className="text-xs text-destructive" role="alert">{error}</p>
        ) : (
          <p className="text-xs text-muted-foreground">PNG, JPEG or WebP, cropped to a square. Everyone in this workspace sees it.</p>
        )}
      </div>
      <input
        ref={inputRef}
        type="file"
        accept="image/png,image/jpeg,image/webp"
        className="hidden"
        aria-label="Workspace logo"
        onChange={(event) => {
          void pick(event.target.files?.[0])
          event.target.value = ''
        }}
      />
    </div>
  )
}
