import { useState } from 'react'
import { toast } from 'sonner'
import { AGENT_DEFAULT_CHOICE as DEFAULT, agentModelChoices, type AgentModelSettings } from '@orbit/api/agent'
import { ApiProblem } from '@orbit/api/problem'
import { SettingsCard } from '@/components/common/SettingsCard'
import { Button } from '@/components/ui/button'
import { Field, FieldDescription, FieldLabel } from '@/components/ui/field'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { FieldGrid } from '@/features/settings/components/SettingsParts'
import { useAgentModel, useAgentStatus, useSaveAgentModel } from './api'

/** Agentic → Admin: the installation's model, for every agent that keeps "Default". The default agent's name and avatar are on its card in Agentic → Agents. */
export function AgentModelPage() {
  const model = useAgentModel()
  const status = useAgentStatus().data
  if (status?.enabled === false) return <SettingsCard title="Agent">The agent is not set up on this server.</SettingsCard>
  if (model.isPending) return <SettingsCard title="Agent"><p role="status">Loading the agent settings…</p></SettingsCard>
  if (model.isError) return <SettingsCard title="Agent"><p role="alert">The agent settings could not be loaded. <Button variant="ghost" onClick={() => void model.refetch()}>Retry</Button></p></SettingsCard>
  // Remount on save so the fields show what the server stored.
  return <ModelCard key={`${model.data.model}/${model.data.thinking}`} settings={model.data} />
}

function ModelCard({ settings }: { settings: AgentModelSettings }) {
  const save = useSaveAgentModel()
  const [model, setModel] = useState(settings.model ?? DEFAULT)
  const [thinking, setThinking] = useState(settings.thinking ?? DEFAULT)
  const changed = model !== (settings.model ?? DEFAULT) || thinking !== (settings.thinking ?? DEFAULT)
  const { models, levels } = agentModelChoices(settings)

  function submit(event: React.FormEvent) {
    event.preventDefault()
    if (!changed || save.isPending) return
    save.mutate(
      { model: model === DEFAULT ? null : model, thinking: thinking === DEFAULT ? null : thinking },
      { onSuccess: () => toast('Agent model saved') },
    )
  }

  return (
    <form onSubmit={submit}>
      <SettingsCard title="Model" description="For every new run, in every workspace, of each agent whose card keeps Default. The name and avatar of the default agent are on its card in Agentic → Agents. More thinking gives better answers, but they are slower and use more of the driver's ChatGPT limits.">
        <FieldGrid>
          <Field>
            <FieldLabel htmlFor="agent-model">Model</FieldLabel>
            <Select items={models} value={model} onValueChange={(value) => { save.reset(); setModel(value ?? DEFAULT) }}>
              <SelectTrigger id="agent-model"><SelectValue /></SelectTrigger>
              <SelectContent>
                {models.map((option) => <SelectItem key={option.value} value={option.value}>{option.label}</SelectItem>)}
              </SelectContent>
            </Select>
            {settings.available ? null : <FieldDescription>The agent service is offline, so only the default model can be chosen now.</FieldDescription>}
          </Field>
          <Field>
            <FieldLabel htmlFor="agent-thinking">Thinking level</FieldLabel>
            <Select items={levels} value={thinking} onValueChange={(value) => { save.reset(); setThinking(value ?? DEFAULT) }}>
              <SelectTrigger id="agent-thinking"><SelectValue /></SelectTrigger>
              <SelectContent>
                {levels.map((option) => <SelectItem key={option.value} value={option.value}>{option.label}</SelectItem>)}
              </SelectContent>
            </Select>
          </Field>
        </FieldGrid>
        {save.isError ? <p role="alert" className="mt-3 text-sm text-destructive">{save.error instanceof ApiProblem ? save.error.detail : 'The model could not be saved.'}</p> : null}
        <div className="mt-4 flex items-center justify-end gap-3">
          {settings.updated_by_name ? <span className="mr-auto text-xs text-muted-foreground">Last changed by {settings.updated_by_name}</span> : null}
          <Button type="submit" disabled={!changed || save.isPending}>{save.isPending ? 'Saving…' : 'Save model'}</Button>
        </div>
      </SettingsCard>
    </form>
  )
}
