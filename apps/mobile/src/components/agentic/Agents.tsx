import { useState } from 'react'
import { Image, View } from 'react-native'
import { AGENT_DEFAULT_CHOICE as DEFAULT, ANSWER_LEVELS, type AgentProfile, type AgentProfiles, type AnswerLevel } from '@orbit/api/agent'

import { ConfirmDialog, type ConfirmRequest } from '../settings/Dialog'
import { CheckRow, SelectField } from '../settings/Fields'
import { Badge, FieldLabel, Note, OptionHint, Segmented, SettingsCard, SettingsRow, Toggle } from '../settings/Shell'
import { ProfilePicture } from '../settings/ProfilePicture'
import { FormSheet } from '../chat/Sheet'
import { agentChannelChoices, agentFormOf, agentNameValid, answerLevelLabel, channelsLabel, customAgentInput, defaultAgentInput, problemDetail, servicesLabel, toolGroups, type AgentForm } from '../../data/agentic'
import type { UploadFile } from '../../data/account-api'
import { pickImage } from '../../hooks/account'
import { useAgentProfiles, useDeleteAgentProfile, useSaveAgentProfile, useSaveDefaultAgent } from '../../hooks/agent'
import { useOrbit } from '../../orbit-context'
import { useConversations } from '../../queries'
import { errorMessage } from '../../session-context'
import { Button, Input, Text } from '../../ui'

/** What the sheet edits: the default agent, a custom agent, or a new one (`agent` null). */
type Editing = { kind: 'default' } | { kind: 'custom'; agent: AgentProfile | null }

/** The web's Agentic → Agents: the default agent first, then the others. Every member reads them; owners and admins manage them. */
export function AgentsCard() {
  const profiles = useAgentProfiles()
  const remove = useDeleteAgentProfile()
  const [editing, setEditing] = useState<Editing | null>(null)
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null)
  const data = profiles.data
  const agents = data?.agents ?? []
  const main = data?.default_agent
  const defaultName = main?.name ?? 'the default agent'

  return (
    <SettingsCard
      title="Agents"
      description={`${defaultName} and the workspace's other agents, each with its own name, instructions, tools, connected services, model and channels. People @mention one in its channels or send it a direct message. It works with their permissions only and asks for their yes before changes.`}
      actions={data?.can_manage ? <Button label="New agent" size="sm" disabled={agents.length >= data.max_agents} onPress={() => setEditing({ kind: 'custom', agent: null })} /> : null}
      flush
    >
      {profiles.isPending ? <View className="p-4"><Note loading>Loading the agents…</Note></View> : null}
      {profiles.isError ? (
        <View className="p-4 gap-2 items-start">
          <Note error>The agents could not be loaded.</Note>
          <Button label="Retry" variant="ghost" onPress={() => void profiles.refetch()} />
        </View>
      ) : null}
      {data && main ? (
        <>
          <AgentRow first id={main.user_id} name={main.name} avatarUrl={main.avatar_url} badges={[{ label: 'Default' }, ...(main.model ? [{ label: main.model, outline: true }] : []), { label: `${answerLevelLabel(main.answer_level)} answers`, outline: true }]} line={`Every channel and direct messages · ${servicesLabel(data, main.integration_ids)}`} instructions={main.instructions}>
            {data.can_manage ? <Button label="Edit" variant="ghost" size="sm" onPress={() => setEditing({ kind: 'default' })} /> : null}
          </AgentRow>
          {agents.map((agent) => (
            <AgentRow key={agent.id} id={agent.user_id} name={agent.name} avatarUrl={agent.avatar_url} badges={[{ label: `${agent.tools.length} tools`, secondary: true }, ...(agent.model ? [{ label: agent.model, outline: true }] : []), { label: `${answerLevelLabel(agent.answer_level)} answers`, outline: true }]} line={`${channelsLabel(agent)} · ${servicesLabel(data, agent.integration_ids)}`} instructions={agent.instructions}>
              {data.can_manage ? (
                <View className="flex-row">
                  <Button label="Edit" variant="ghost" size="sm" onPress={() => setEditing({ kind: 'custom', agent })} />
                  <Button
                    label="Delete"
                    variant="ghost"
                    size="sm"
                    onPress={() =>
                      setConfirm({
                        title: `Delete ${agent.name}?`,
                        description: 'Nobody can use it any more and its routines pause. Its messages and direct messages stay readable.',
                        confirmLabel: 'Delete',
                        danger: true,
                        onConfirm: () => remove.mutate(agent.id),
                      })
                    }
                  />
                </View>
              ) : null}
            </AgentRow>
          ))}
          {agents.length === 0 && data.can_manage ? (
            <SettingsRow>
              <Note>No other agents yet. Create one for a job, for example a blog writer that drafts Docs pages.</Note>
            </SettingsRow>
          ) : null}
          {remove.isError ? <View className="p-4"><Note error>{problemDetail(remove.error, 'The agent could not be deleted.')}</Note></View> : null}
        </>
      ) : null}
      {editing && data ? <AgentSheet profiles={data} editing={editing} onClose={() => setEditing(null)} /> : null}
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </SettingsCard>
  )
}

function AgentRow({ first = false, id, name, avatarUrl, badges, line, instructions, children }: { first?: boolean; id: string; name: string; avatarUrl: string | null; badges: { label: string; outline?: boolean; secondary?: boolean }[]; line: string; instructions: string; children?: React.ReactNode }) {
  return (
    <View className={`px-4 py-3 gap-2 border-border ${first ? '' : 'border-t'}`}>
      <View className="flex-row items-start gap-3">
        <ProfilePicture id={id} name={name} url={avatarUrl} size={36} />
        <View className="flex-1 min-w-0 gap-1">
          <View className="flex-row flex-wrap items-center gap-1.5">
            <Text className="font-noto-medium text-[14px]">{name}</Text>
            {badges.map((badge, index) => <Badge key={index} label={badge.label} tone={badge.outline ? 'outline' : badge.secondary ? 'secondary' : 'primary'} />)}
          </View>
          <Text className="text-[12px] text-muted-foreground">{line}</Text>
          {instructions ? <Text numberOfLines={2} className="text-[13px] text-muted-foreground">{instructions}</Text> : null}
        </View>
      </View>
      {children ? <View className="flex-row justify-end">{children}</View> : null}
    </View>
  )
}

const toggled = (list: string[], item: string, on: boolean) => (on ? [...list, item] : list.filter((own) => own !== item))

/** Create or edit one agent, or edit the default agent. The avatar is uploaded after the agent is saved. */
function AgentSheet({ profiles, editing, onClose }: { profiles: AgentProfiles; editing: Editing; onClose: () => void }) {
  const { user } = useOrbit()
  const saveCustom = useSaveAgentProfile()
  const saveDefault = useSaveDefaultAgent()
  const save = editing.kind === 'default' ? saveDefault : saveCustom
  const isDefault = editing.kind === 'default'
  const custom = editing.kind === 'custom' ? editing.agent : null
  const main = profiles.default_agent
  const channels = agentChannelChoices(useConversations().data ?? [], user.id)
  // The default agent's name and avatar are the installation's.
  const canRename = !isDefault || main.can_rename
  const [form, setForm] = useState<AgentForm>(() => agentFormOf(custom, isDefault ? main : undefined))
  const set = (change: Partial<AgentForm>) => {
    save.reset()
    setForm((current) => ({ ...current, ...change }))
  }
  // `undefined`: the avatar stays; `null`: removed.
  const [avatar, setAvatar] = useState<UploadFile | null | undefined>(undefined)
  const [avatarError, setAvatarError] = useState<string | null>(null)
  const savedAvatar = isDefault ? main.avatar_url : (custom?.avatar_url ?? null)
  const localUri = avatar ? (avatar as { uri?: string }).uri : undefined
  const models = [{ value: DEFAULT, label: 'Default (the Default model below)' }, ...(profiles.models?.models ?? []).map((option) => ({ value: option.id, label: option.name }))]
  if (form.model !== DEFAULT && !models.some((option) => option.value === form.model)) models.push({ value: form.model, label: form.model })
  const levels = [{ value: DEFAULT, label: 'Default' }, ...profiles.thinking_levels.map((level) => ({ value: level, label: level }))]
  const title = isDefault ? `Edit ${main.name}` : custom ? `Edit ${custom.name}` : 'New agent'

  const chooseAvatar = () => {
    setAvatarError(null)
    pickImage({ square: true, maxBytes: 512 * 1024 })
      .then((file) => file && setAvatar(file))
      .catch((error: unknown) => setAvatarError(errorMessage(error)))
  }

  const submit = () => {
    const done = { onSuccess: onClose }
    if (isDefault) saveDefault.mutate({ input: defaultAgentInput(form, main.name), avatar }, done)
    else saveCustom.mutate({ id: custom?.id ?? null, input: customAgentInput(form), avatar }, done)
  }

  return (
    <FormSheet visible title={title} submitLabel={custom || isDefault ? 'Save' : 'Create'} canSubmit={agentNameValid(form.name) && !save.isPending} busy={save.isPending} onSubmit={submit} onClose={onClose}>
      <View className="flex-row items-center gap-3">
        {localUri ? <Image accessibilityLabel="New avatar" source={{ uri: localUri }} className="rounded-full" style={{ width: 48, height: 48 }} /> : <ProfilePicture id={isDefault ? main.user_id : (custom?.user_id ?? 'new')} name={form.name || '?'} url={avatar === null ? null : savedAvatar} size={48} />}
        {canRename ? (
          <View className="flex-row flex-wrap gap-2">
            <Button label="Upload avatar" variant="outline" size="sm" onPress={chooseAvatar} />
            {avatar || (avatar === undefined && savedAvatar) ? <Button label="Remove" variant="ghost" size="sm" onPress={() => setAvatar(null)} /> : null}
          </View>
        ) : null}
      </View>
      {avatarError ? <Note error>{avatarError}</Note> : null}
      <Input
        label="Name"
        required
        value={form.name}
        maxLength={40}
        editable={canRename}
        onChangeText={(name) => set({ name })}
        description={`Its @handle in chat. 2 to 40 characters: letters, digits, single spaces and - _ . ' It cannot be anyone else's name.${isDefault ? (canRename ? ' The name and avatar are the same in every workspace; mentions and earlier messages show the new name.' : ' The name and avatar are the same in every workspace, so only an installation administrator changes them.') : ''}`}
      />
      <Input
        label="Instructions"
        multiline
        value={form.instructions}
        maxLength={profiles.max_instructions_chars}
        onChangeText={(instructions) => set({ instructions })}
        description={`Its role, tone and how it works${isDefault ? ' in this workspace, besides the team instructions (Agentic → Workspace) that every agent follows' : ''}. Orbit's own rules stay: it acts with the person's permissions and asks before changes.`}
      />
      {isDefault ? null : (
        <View className="gap-2">
          <FieldLabel>Tools</FieldLabel>
          <OptionHint>Orbit refuses every tool that is not ticked, whatever the instructions say.</OptionHint>
          {toolGroups(profiles.tools).map(([group, tools]) => (
            <View key={group} className="gap-0.5">
              <Text className="mt-2 text-[12px] font-noto-medium text-muted-foreground">{group}</Text>
              {tools.map((tool) => <CheckRow key={tool.name} mono label={tool.name} hint={tool.writes ? 'writes' : undefined} checked={form.tools.includes(tool.name)} onChange={(on) => set({ tools: toggled(form.tools, tool.name, on) })} />)}
            </View>
          ))}
        </View>
      )}
      <View className="gap-2">
        <FieldLabel>Connected services</FieldLabel>
        {profiles.integrations.length === 0 ? (
          <OptionHint>No connected services yet. Owners and admins add them in Agentic → Workspace on the web.</OptionHint>
        ) : (
          <>
            {profiles.integrations.map((integration) => <CheckRow key={integration.id} label={integration.name} hint={integration.kind === 'mcp' ? 'MCP server' : 'HTTP API'} checked={form.services.includes(integration.id)} onChange={(on) => set({ services: toggled(form.services, integration.id, on) })} />)}
            <OptionHint>
              Orbit refuses every service that is not ticked, and each service's role rule still applies.{' '}
              {isDefault ? `A service added later is on for ${main.name} until you untick it.` : 'A service added later is off for this agent until you tick it.'}
            </OptionHint>
          </>
        )}
      </View>
      <SelectField label="Model" value={form.model} options={models} onChange={(model) => set({ model })} description={profiles.models ? undefined : 'The agent service is offline, so only the default model can be chosen now.'} />
      <SelectField label="Thinking level" value={form.thinking} options={levels} onChange={(thinking) => set({ thinking })} />
      <View className="gap-2">
        <FieldLabel>Answer level</FieldLabel>
        <Segmented<AnswerLevel> label="Answer level" value={form.answerLevel} options={ANSWER_LEVELS} onChange={(answerLevel) => set({ answerLevel })} />
        <OptionHint>How much it thinks and writes, for everyone who uses it. {ANSWER_LEVELS.find((option) => option.value === form.answerLevel)?.help}</OptionHint>
      </View>
      {isDefault ? (
        <OptionHint>{main.name} works in every channel and direct message, and cannot be deleted.</OptionHint>
      ) : (
        <View className="gap-2">
          <FieldLabel>Channels</FieldLabel>
          <View className="flex-row items-center justify-between">
            <Text className="text-[14px]">Every channel</Text>
            <Toggle label="Every channel" value={form.allChannels} onChange={(allChannels) => set({ allChannels })} />
          </View>
          {form.allChannels ? null : channels.map((channel) => <CheckRow key={channel.id} label={`#${channel.name}`} checked={form.channelIds.includes(channel.id)} onChange={(on) => set({ channelIds: toggled(form.channelIds, channel.id, on) })} />)}
          <OptionHint>Direct messages with it always work. A mention in another channel gets no answer there: the agent tells the person in a direct message.</OptionHint>
        </View>
      )}
      {save.isError ? <Note error>{problemDetail(save.error, 'The agent could not be saved.')}</Note> : null}
    </FormSheet>
  )
}
