import { useState } from 'react'
import { toast } from 'sonner'
import type { AgentIdentity } from '@orbit/api/agent'
import { ApiProblem } from '@orbit/api/problem'
import { SettingsCard } from '@/components/common/SettingsCard'
import { Button } from '@/components/ui/button'
import { Field, FieldDescription, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { FieldGrid } from '@/features/settings/components/SettingsParts'
import { useAgentIdentity, useSaveAgentName } from './api'

/** Admin → AI agent: the agent's one name for the installation, which is also its @handle in chat. */
export function AgentNamePage() {
  const identity = useAgentIdentity()
  if (identity.isPending) return <SettingsCard title="AI agent"><p role="status">Loading the agent settings…</p></SettingsCard>
  if (identity.isError) return <SettingsCard title="AI agent"><p role="alert">The agent settings could not be loaded. <Button variant="ghost" onClick={() => void identity.refetch()}>Retry</Button></p></SettingsCard>
  // Remount on save so the field shows what the server stored.
  return <NameCard key={identity.data.name} identity={identity.data} />
}

function NameCard({ identity }: { identity: AgentIdentity }) {
  const save = useSaveAgentName()
  const [name, setName] = useState(identity.name)
  const changed = name.trim() !== identity.name

  function submit(event: React.FormEvent) {
    event.preventDefault()
    if (!changed || save.isPending) return
    save.mutate(name.trim(), { onSuccess: () => toast('Agent name saved') })
  }

  return (
    <form onSubmit={submit}>
      <SettingsCard title="AI agent" description="One name for the agent in every workspace. People mention it as @Name in chat, and its messages and earlier mentions show the new name. It cannot be the name of another person in Orbit.">
        <FieldGrid>
          <Field>
            <FieldLabel htmlFor="agent-name">Name</FieldLabel>
            <Input id="agent-name" required maxLength={identity.max_name_chars} value={name} onChange={(event) => { save.reset(); setName(event.target.value) }} />
            <FieldDescription>{identity.min_name_chars} to {identity.max_name_chars} characters: letters, digits, single spaces and - _ . '</FieldDescription>
          </Field>
        </FieldGrid>
        {save.isError ? <p role="alert" className="mt-3 text-sm text-destructive">{save.error instanceof ApiProblem ? save.error.detail : 'The name could not be saved.'}</p> : null}
        <div className="mt-4 flex justify-end">
          <Button type="submit" disabled={!changed || save.isPending}>{save.isPending ? 'Saving…' : 'Save name'}</Button>
        </div>
      </SettingsCard>
    </form>
  )
}
