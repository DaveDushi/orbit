import { useState } from 'react'
import { View } from 'react-native'

import { SelectField } from '../settings/Fields'
import { Note, SettingsCard } from '../settings/Shell'
import { AGENT_DEFAULT_CHOICE as DEFAULT, agentModelChoices, type AgentModelSettings } from '@orbit/api/agent'
import { useAgentModel, useSaveAgentModel } from '../../hooks/agent'
import { useOrbit } from '../../orbit-context'
import { errorMessage } from '../../session-context'
import { Button, Text } from '../../ui'

/**
 * Agentic → Agents, below the agents: the installation's model, for every agent whose card keeps Default. Only
 * installation admins see and change it (it was the Agentic → Admin tab).
 */
export function DefaultModelCard() {
  return useOrbit().user.installation_admin ? <DefaultModel /> : null
}

function DefaultModel() {
  const model = useAgentModel()
  if (model.isPending) return <SettingsCard title="Default model"><Note loading>Loading the default model…</Note></SettingsCard>
  if (model.isError) {
    return (
      <SettingsCard title="Default model">
        <View className="flex-row items-center gap-2">
          <Note error>The default model could not be loaded.</Note>
          <Button label="Retry" variant="ghost" onPress={() => void model.refetch()} />
        </View>
      </SettingsCard>
    )
  }
  // Remount on save so the fields show what the server stored.
  return <ModelCard key={`${model.data.model}/${model.data.thinking}`} settings={model.data} />
}

function ModelCard({ settings }: { settings: AgentModelSettings }) {
  const save = useSaveAgentModel()
  const [model, setModel] = useState(settings.model ?? DEFAULT)
  const [thinking, setThinking] = useState(settings.thinking ?? DEFAULT)
  const changed = model !== (settings.model ?? DEFAULT) || thinking !== (settings.thinking ?? DEFAULT)
  const { models, levels } = agentModelChoices(settings)
  return (
    <SettingsCard title="Default model" description="The installation's model, for every new run, in every workspace, of each agent whose card keeps Default. Only installation administrators change it. More thinking gives better answers, but they are slower and use more of the driver's ChatGPT limits.">
      <View className="gap-4">
        <SelectField label="Model" value={model} options={models} onChange={(value) => { save.reset(); setModel(value) }} description={settings.available ? undefined : 'The agent service is offline, so only the default model can be chosen now.'} />
        <SelectField label="Thinking level" value={thinking} options={levels} onChange={(value) => { save.reset(); setThinking(value) }} />
        {save.isError ? <Note error>{errorMessage(save.error)}</Note> : null}
        {save.isSuccess ? <Note>Default model saved.</Note> : null}
        <View className="flex-row items-center justify-end gap-3">
          {settings.updated_by_name ? <Text className="mr-auto text-[12px] text-muted-foreground">Last changed by {settings.updated_by_name}</Text> : null}
          <Button
            label={save.isPending ? 'Saving…' : 'Save default model'}
            disabled={!changed || save.isPending}
            onPress={() => save.mutate({ model: model === DEFAULT ? null : model, thinking: thinking === DEFAULT ? null : thinking })}
          />
        </View>
      </View>
    </SettingsCard>
  )
}
