import { useState } from 'react'
import { Pressable, View } from 'react-native'

import { FieldLabel, Note, SettingsCard } from '../settings/Shell'
import { AGENT_DEFAULT_CHOICE as DEFAULT, agentModelChoices, type AgentModelSettings } from '@orbit/api/agent'
import { useAgentModel, useSaveAgentModel } from '../../hooks/agent'
import { errorMessage } from '../../session-context'
import { useTheme } from '../../theme'
import { Button, Icon, PickerSheet, Text } from '../../ui'

/** The web's Agentic → Admin: the installation's model for every agent that keeps Default. The default agent's name and avatar are on its card in the web's Agentic → Agents. */
export function AdminAgent() {
  const model = useAgentModel()
  return (
    <>
      {model.isPending ? <SettingsCard title="Agent"><Note>Loading the agent settings…</Note></SettingsCard> : null}
      {model.isError ? (
        <SettingsCard title="Agent">
          <View className="flex-row items-center gap-2">
            <Note error>The agent settings could not be loaded.</Note>
            <Button label="Retry" variant="ghost" onPress={() => void model.refetch()} />
          </View>
        </SettingsCard>
      ) : null}
      {model.data ? <ModelCard key={`${model.data.model}/${model.data.thinking}`} settings={model.data} /> : null}
    </>
  )
}

function ModelCard({ settings }: { settings: AgentModelSettings }) {
  const save = useSaveAgentModel()
  const [model, setModel] = useState(settings.model ?? DEFAULT)
  const [thinking, setThinking] = useState(settings.thinking ?? DEFAULT)
  const changed = model !== (settings.model ?? DEFAULT) || thinking !== (settings.thinking ?? DEFAULT)
  const { models, levels } = agentModelChoices(settings)
  return (
    <SettingsCard title="Model" description="For every new run, in every workspace, of each agent whose card keeps Default. The default agent's name and avatar are on its card in Agentic → Agents on the web. More thinking gives better answers, but they are slower and use more of the driver's ChatGPT limits.">
      <View className="gap-4">
        <Choice label="Model" value={model} options={models} onChange={(value) => { save.reset(); setModel(value) }} />
        {settings.available ? null : <Note>The agent service is offline, so only the default model can be chosen now.</Note>}
        <Choice label="Thinking level" value={thinking} options={levels} onChange={(value) => { save.reset(); setThinking(value) }} />
        {save.isError ? <Note error>{errorMessage(save.error)}</Note> : null}
        {save.isSuccess ? <Note>Agent model saved.</Note> : null}
        <View className="flex-row items-center justify-end gap-3">
          {settings.updated_by_name ? <Text className="mr-auto text-[12px] text-muted-foreground">Last changed by {settings.updated_by_name}</Text> : null}
          <Button
            label={save.isPending ? 'Saving…' : 'Save model'}
            disabled={!changed || save.isPending}
            onPress={() => save.mutate({ model: model === DEFAULT ? null : model, thinking: thinking === DEFAULT ? null : thinking })}
          />
        </View>
      </View>
    </SettingsCard>
  )
}

/** A select field: the current choice, and a sheet with every choice. */
function Choice({ label, value, options, onChange }: { label: string; value: string; options: { value: string; label: string }[]; onChange: (value: string) => void }) {
  const { colors } = useTheme()
  const [open, setOpen] = useState(false)
  return (
    <View className="gap-2">
      <FieldLabel>{label}</FieldLabel>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={label}
        onPress={() => setOpen(true)}
        className="min-h-8 flex-row items-center justify-between rounded-lg border border-input px-2.5 py-[3px] dark:bg-input/30"
      >
        <Text className="text-[16px] leading-[24px]">{options.find((option) => option.value === value)?.label ?? value}</Text>
        <Icon name="chevronDown" size={16} color={colors.mutedForeground} />
      </Pressable>
      <PickerSheet
        visible={open}
        title={label}
        options={options.map((option) => ({ key: option.value, label: option.label, selected: option.value === value }))}
        onSelect={(key) => { setOpen(false); onChange(key) }}
        onClose={() => setOpen(false)}
      />
    </View>
  )
}
