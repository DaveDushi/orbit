import { useState } from 'react'
import { Pressable, View } from 'react-native'

import { AdminShell, FieldLabel, Note, SettingsCard } from '../../../../components/settings/Shell'
import { AGENT_DEFAULT_CHOICE as DEFAULT, agentModelChoices, type AgentIdentity, type AgentModelSettings } from '@orbit/api/agent'
import { useAgentIdentity, useAgentModel, useSaveAgentModel, useSaveAgentName } from '../../../../hooks/agent'
import { errorMessage } from '../../../../session-context'
import { useTheme } from '../../../../theme'
import { Button, Icon, Input, PickerSheet, Text } from '../../../../ui'

/** The web's Admin → AI agent: the agent's one name for the installation, which is also its @handle in chat, and its model. */
export default function AdminAgentScreen() {
  const identity = useAgentIdentity()
  const model = useAgentModel()
  return (
    <AdminShell refreshing={identity.isRefetching || model.isRefetching} onRefresh={() => { void identity.refetch(); void model.refetch() }}>
      {identity.isPending || model.isPending ? <SettingsCard title="AI agent"><Note>Loading the agent settings…</Note></SettingsCard> : null}
      {identity.isError || model.isError ? (
        <SettingsCard title="AI agent">
          <View className="flex-row items-center gap-2">
            <Note error>The agent settings could not be loaded.</Note>
            <Button label="Retry" variant="ghost" onPress={() => { void identity.refetch(); void model.refetch() }} />
          </View>
        </SettingsCard>
      ) : null}
      {identity.data ? <NameCard identity={identity.data} /> : null}
      {model.data ? <ModelCard key={`${model.data.model}/${model.data.thinking}`} settings={model.data} /> : null}
    </AdminShell>
  )
}

function ModelCard({ settings }: { settings: AgentModelSettings }) {
  const save = useSaveAgentModel()
  const [model, setModel] = useState(settings.model ?? DEFAULT)
  const [thinking, setThinking] = useState(settings.thinking ?? DEFAULT)
  const changed = model !== (settings.model ?? DEFAULT) || thinking !== (settings.thinking ?? DEFAULT)
  const { models, levels } = agentModelChoices(settings)
  return (
    <SettingsCard title="Model" description="The ChatGPT model and thinking level of every new run, in every workspace. More thinking gives better answers, but they are slower and use more of the driver's ChatGPT limits.">
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

function NameCard({ identity }: { identity: AgentIdentity }) {
  const save = useSaveAgentName()
  const [name, setName] = useState(identity.name)
  const changed = name.trim() !== identity.name
  return (
    <SettingsCard title="AI agent" description="One name for the agent in every workspace. People mention it as @Name in chat, and its messages and earlier mentions show the new name. It cannot be the name of another person in Orbit.">
      <View className="gap-4">
        <Input
          label="Name"
          autoCapitalize="words"
          autoCorrect={false}
          maxLength={identity.max_name_chars}
          value={name}
          onChangeText={(value) => {
            save.reset()
            setName(value)
          }}
          description={`${identity.min_name_chars} to ${identity.max_name_chars} characters: letters, digits, single spaces and - _ . '`}
        />
        {save.isError ? <Note error>{errorMessage(save.error)}</Note> : null}
        {save.isSuccess ? <Note>Agent name saved.</Note> : null}
        <View className="flex-row justify-end">
          <Button label={save.isPending ? 'Saving…' : 'Save name'} disabled={!changed || save.isPending} onPress={() => save.mutate(name.trim(), { onSuccess: (saved) => setName(saved.name) })} />
        </View>
      </View>
    </SettingsCard>
  )
}
