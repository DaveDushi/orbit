import { useState } from 'react'
import { toast } from 'sonner'
import type { TeamInstructions } from '@orbit/api/agent'
import { ApiProblem } from '@orbit/api/problem'
import { SettingsCard } from '@/components/common/SettingsCard'
import { Button } from '@/components/ui/button'
import { Field, FieldDescription, FieldLabel } from '@/components/ui/field'
import { Textarea } from '@/components/ui/textarea'
import { useWorkspace } from '@/features/workspaces/workspaceContext'
import { useAgentStatus, useSaveTeamInstructions, useTeamInstructions } from './api'

/** Characters as the server counts them (code points, not UTF-16 units). */
const characters = (text: string) => [...text.replace(/\r\n/g, '\n').trim()].length

/** Workspace settings → AI agent: the team's instructions for every agent run in this workspace. */
export function TeamInstructionsPage() {
  const { workspace } = useWorkspace()
  const team = useTeamInstructions(workspace.id)
  const status = useAgentStatus().data
  const name = status?.enabled ? status.name : 'the agent'
  if (team.isPending) return <SettingsCard title="AI agent"><p role="status">Loading the team instructions…</p></SettingsCard>
  if (team.isError) return <SettingsCard title="AI agent"><p role="alert">The team instructions could not be loaded. <Button variant="ghost" onClick={() => void team.refetch()}>Retry</Button></p></SettingsCard>
  // Remount on save and on a workspace switch so the field shows what the server stored.
  return <TeamCard key={`${workspace.id}:${team.data.updated_at}`} workspaceId={workspace.id} team={team.data} agentName={name} />
}

function TeamCard({ workspaceId, team, agentName }: { workspaceId: string; team: TeamInstructions; agentName: string }) {
  const save = useSaveTeamInstructions(workspaceId)
  const [text, setText] = useState(team.instructions)
  const count = characters(text)
  const tooLong = count > team.max_chars
  const changed = text.replace(/\r\n/g, '\n').trim() !== team.instructions
  const description = `Instructions that @${agentName} follows in every run in this workspace, for example your language, tone or conventions. Every member can read them. They cannot override the agent's built-in safety rules.`

  if (!team.can_edit) {
    return (
      <SettingsCard title="AI agent" description={description}>
        {team.instructions
          ? <p className="text-sm whitespace-pre-wrap text-foreground">{team.instructions}</p>
          : <p className="text-sm text-muted-foreground">This workspace has no team instructions.</p>}
        <p className="mt-3 text-xs text-muted-foreground">Only owners and admins can change them.</p>
      </SettingsCard>
    )
  }

  function submit(event: React.FormEvent) {
    event.preventDefault()
    if (!changed || tooLong || save.isPending) return
    save.mutate(text, { onSuccess: () => toast('Team instructions saved') })
  }

  return (
    <form onSubmit={submit}>
      <SettingsCard title="AI agent" description={description}>
        <Field>
          <FieldLabel htmlFor="team-instructions">Team instructions</FieldLabel>
          <Textarea id="team-instructions" className="min-h-40" aria-invalid={tooLong || undefined} placeholder="For example: Answer in German. Our sprints start on Mondays." value={text} onChange={(event) => { save.reset(); setText(event.target.value) }} />
          <FieldDescription>
            <span className={tooLong ? 'text-destructive' : undefined}>{count.toLocaleString()} / {team.max_chars.toLocaleString()} characters</span>
            {team.updated_at ? ` · Last changed by ${team.updated_by_name ?? 'a former member'} on ${new Date(team.updated_at).toLocaleDateString()}` : null}
          </FieldDescription>
        </Field>
        {save.isError ? <p role="alert" className="mt-3 text-sm text-destructive">{save.error instanceof ApiProblem ? save.error.detail : 'The team instructions could not be saved.'}</p> : null}
        <div className="mt-4 flex justify-end">
          <Button type="submit" disabled={!changed || tooLong || save.isPending}>{save.isPending ? 'Saving…' : 'Save instructions'}</Button>
        </div>
      </SettingsCard>
    </form>
  )
}
