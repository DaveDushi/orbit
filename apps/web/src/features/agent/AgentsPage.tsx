import { useState } from 'react'
import { toast } from 'sonner'
import { AGENT_DEFAULT_CHOICE as DEFAULT, ANSWER_LEVELS, type AgentProfile, type AgentProfiles, type AnswerLevel, type DefaultAgent } from '@orbit/api/agent'
import { ApiProblem } from '@orbit/api/problem'
import { ConfirmDeleteModal } from '@/components/common/ConfirmDeleteModal'
import { Modal } from '@/components/common/Modal'
import { SettingsCard } from '@/components/common/SettingsCard'
import { UserAvatar } from '@/components/common/UserAvatar'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { DialogClose, DialogFooter } from '@/components/ui/dialog'
import { Field, FieldDescription, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Switch } from '@/components/ui/switch'
import { Textarea } from '@/components/ui/textarea'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { useConversations } from '@/features/chat/api/queries'
import { avatarImage } from '@/features/profile/avatarImage'
import { FieldGrid } from '@/features/settings/components/SettingsParts'
import { useWorkspace } from '@/features/workspaces/workspaceContext'
import { useAgentProfiles, useDeleteAgentProfile, useSaveAgentProfile, useSaveDefaultAgent } from './api'

/** What the dialog edits: the default agent, a custom agent, or a new one. */
type Editing = { kind: 'default'; agent: DefaultAgent } | { kind: 'custom'; agent: AgentProfile | null }

/** Settings → Agentic → Agents: the default agent first, then the workspace's other agents. Owners and admins manage them. */
export function AgentsPage() {
  const { workspace } = useWorkspace()
  const profiles = useAgentProfiles(workspace.id)
  const remove = useDeleteAgentProfile(workspace.id)
  const [editing, setEditing] = useState<Editing | null>(null)
  const [deleting, setDeleting] = useState<AgentProfile | null>(null)
  const data = profiles.data
  const agents = data?.agents ?? []
  const main = data?.default_agent
  const defaultName = main?.name ?? 'the default agent'

  return (
    <SettingsCard
      title="Agents"
      description={`${defaultName} and the workspace's other agents, each with its own name, instructions, tools, connected services, model and channels. People @mention one in its channels or send it a direct message. It works with their permissions only and asks for their yes before changes.`}
      actions={data?.can_manage ? <Button size="sm" disabled={agents.length >= data.max_agents} onClick={() => setEditing({ kind: 'custom', agent: null })}>New agent</Button> : null}
    >
      {profiles.isPending ? null : profiles.isError || !data || !main ? (
        <p role="alert" className="text-sm text-destructive">The agents could not be loaded.</p>
      ) : (
        <ul className="-mx-2 flex flex-col">
          <li data-slot="agent-profile" data-default="" className="flex items-start gap-3 rounded-md px-2 py-3 not-last:border-b not-last:border-border">
            <UserAvatar user={{ name: main.name, color: 'var(--primary)', avatarUrl: main.avatar_url }} size={36} />
            <div className="flex min-w-0 flex-1 flex-col gap-1">
              <div className="flex flex-wrap items-center gap-2">
                <span className="text-sm font-medium text-foreground">{main.name}</span>
                <Badge>Default</Badge>
                {main.model ? <Badge variant="outline">{main.model}</Badge> : null}
                <Badge variant="outline">{answerLevelLabel(main.answer_level)} answers</Badge>
              </div>
              <p className="text-xs text-muted-foreground">Every channel and direct messages · {servicesLabel(data, main.integration_ids)}</p>
              {main.instructions ? <p className="line-clamp-2 text-sm text-muted-foreground">{main.instructions}</p> : null}
            </div>
            {data.can_manage ? (
              <div className="flex shrink-0 gap-1">
                <Button size="sm" variant="ghost" onClick={() => setEditing({ kind: 'default', agent: main })}>Edit</Button>
              </div>
            ) : null}
          </li>
          {agents.map((agent) => (
            <li key={agent.id} data-slot="agent-profile" className="flex items-start gap-3 rounded-md px-2 py-3 not-last:border-b not-last:border-border">
              <UserAvatar user={{ name: agent.name, color: 'var(--primary)', avatarUrl: agent.avatar_url }} size={36} />
              <div className="flex min-w-0 flex-1 flex-col gap-1">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-sm font-medium text-foreground">{agent.name}</span>
                  <Badge variant="secondary">{agent.tools.length} tools</Badge>
                  {agent.model ? <Badge variant="outline">{agent.model}</Badge> : null}
                  <Badge variant="outline">{answerLevelLabel(agent.answer_level)} answers</Badge>
                </div>
                <p className="text-xs text-muted-foreground">{channelsLabel(agent)} · {servicesLabel(data, agent.integration_ids)}</p>
                {agent.instructions ? <p className="line-clamp-2 text-sm text-muted-foreground">{agent.instructions}</p> : null}
              </div>
              {data?.can_manage ? (
                <div className="flex shrink-0 gap-1">
                  <Button size="sm" variant="ghost" onClick={() => setEditing({ kind: 'custom', agent })}>Edit</Button>
                  <Button size="sm" variant="ghost" className="text-destructive" onClick={() => setDeleting(agent)}>Delete</Button>
                </div>
              ) : null}
            </li>
          ))}
          {agents.length === 0 && data.can_manage ? <li className="px-2 py-3 text-sm text-muted-foreground">No other agents yet. Create one for a job, for example a blog writer that drafts Docs pages.</li> : null}
        </ul>
      )}
      {editing && data ? <AgentDialog profiles={data} editing={editing} onClose={() => setEditing(null)} /> : null}
      {deleting ? (
        <ConfirmDeleteModal
          title={`Delete ${deleting.name}?`}
          description="Nobody can use it any more and its routines pause. Its messages and direct messages stay readable."
          onClose={() => setDeleting(null)}
          onConfirm={() =>
            remove.mutate(deleting.id, {
              onSuccess: () => setDeleting(null),
              onError: (error) => toast.error(error instanceof ApiProblem ? error.detail : 'The agent could not be deleted.'),
            })
          }
        />
      ) : null}
    </SettingsCard>
  )
}

const answerLevelLabel = (level: AnswerLevel) => ANSWER_LEVELS.find((option) => option.value === level)?.label ?? 'Normal'

function channelsLabel(agent: AgentProfile): string {
  if (agent.all_channels) return 'Every channel and direct messages'
  if (agent.channels.length === 0) return 'Direct messages only'
  return `${agent.channels.map((channel) => `#${channel.name}`).join(', ')} and direct messages`
}

/** How many of the workspace's connected services the agent may use (owners and admins see them). */
function servicesLabel(profiles: AgentProfiles, ids: string[]): string {
  if (!profiles.can_manage || profiles.integrations.length === 0) return 'No connected services'
  const on = profiles.integrations.filter((integration) => ids.includes(integration.id)).length
  if (on === profiles.integrations.length) return 'All connected services'
  return on === 0 ? 'No connected services' : `${on} of ${profiles.integrations.length} connected services`
}

/** The tool that calls connected services: on exactly when a service is ticked, so it is not in the tool list. */
const INTEGRATION_TOOL = 'use_integration'

/** The tools by group, in the server's order. */
function toolGroups(profiles: AgentProfiles) {
  const groups = new Map<string, AgentProfiles['tools']>()
  for (const tool of profiles.tools) {
    if (tool.name !== INTEGRATION_TOOL) groups.set(tool.group, [...(groups.get(tool.group) ?? []), tool])
  }
  return [...groups.entries()]
}

/** Create or edit one agent, or edit the default agent. The avatar is uploaded after the agent is saved. */
function AgentDialog({ profiles, editing, onClose }: { profiles: AgentProfiles; editing: Editing; onClose: () => void }) {
  const { workspace } = useWorkspace()
  const saveCustom = useSaveAgentProfile(workspace.id)
  const saveDefault = useSaveDefaultAgent(workspace.id)
  const save = editing.kind === 'default' ? saveDefault : saveCustom
  const channels = (useConversations().data ?? []).filter((conversation) => conversation.kind !== 'dm' && !conversation.archived && (conversation.kind === 'public' || conversation.isMember))
  const isDefault = editing.kind === 'default'
  const custom = editing.kind === 'custom' ? editing.agent : null
  const agent = editing.agent
  // The default agent's name and avatar are the installation's.
  const canRename = editing.kind === 'custom' || editing.agent.can_rename
  const [name, setName] = useState(agent?.name ?? '')
  const [instructions, setInstructions] = useState(agent?.instructions ?? '')
  const [tools, setTools] = useState<string[]>((custom?.tools ?? []).filter((tool) => tool !== INTEGRATION_TOOL))
  const [services, setServices] = useState<string[]>(agent?.integration_ids ?? [])
  const [model, setModel] = useState(agent?.model ?? DEFAULT)
  const [thinking, setThinking] = useState(agent?.thinking ?? DEFAULT)
  const [answerLevel, setAnswerLevel] = useState<AnswerLevel>(agent?.answer_level ?? 'normal')
  const [allChannels, setAllChannels] = useState(custom?.all_channels ?? false)
  const [channelIds, setChannelIds] = useState<string[]>(custom?.channels.map((channel) => channel.id) ?? [])
  const [avatar, setAvatar] = useState<{ blob: Blob; url: string } | null | undefined>(undefined)
  const models = [{ value: DEFAULT, label: 'Default (the Default model below)' }, ...(profiles.models?.models ?? []).map((option) => ({ value: option.id, label: option.name }))]
  if (agent?.model && !models.some((option) => option.value === agent.model)) models.push({ value: agent.model, label: agent.model })
  const levels = [{ value: DEFAULT, label: 'Default' }, ...profiles.thinking_levels.map((level) => ({ value: level, label: level }))]
  const toggle = (list: string[], item: string, on: boolean) => (on ? [...list, item] : list.filter((own) => own !== item))
  const avatarUrl = avatar === undefined ? agent?.avatar_url : (avatar?.url ?? null)

  async function pickAvatar(file: File | undefined) {
    if (!file) return
    try {
      const blob = await avatarImage(file)
      setAvatar({ blob, url: URL.createObjectURL(blob) })
    } catch {
      toast.error('This image cannot be used. Pick a PNG, JPEG or WebP image.')
    }
  }

  function submit(event: React.FormEvent) {
    event.preventDefault()
    const choice = { instructions, model: model === DEFAULT ? null : model, thinking: thinking === DEFAULT ? null : thinking, answer_level: answerLevel, integration_ids: services }
    const done = { onSuccess: (saved: { name: string }) => { toast(`${saved.name} saved`); onClose() } }
    const newAvatar = avatar === undefined ? undefined : (avatar?.blob ?? null)
    if (editing.kind === 'default') {
      const renamed = name.trim() !== editing.agent.name
      saveDefault.mutate({ input: { ...choice, ...(renamed ? { name: name.trim() } : {}) }, avatar: newAvatar }, done)
      return
    }
    saveCustom.mutate(
      {
        id: custom?.id ?? null,
        input: {
          ...choice,
          name: name.trim(),
          tools: services.length ? [...tools, INTEGRATION_TOOL] : tools,
          all_channels: allChannels,
          channel_ids: channelIds,
        },
        avatar: newAvatar,
      },
      done,
    )
  }

  return (
    <Modal title={agent ? `Edit ${agent.name}` : 'New agent'} onClose={onClose} className="sm:max-w-2xl">
      <form id="agent-profile-form" onSubmit={submit} className="flex max-h-[70vh] flex-col gap-5 overflow-y-auto pr-1">
        <div className="flex items-center gap-3">
          <UserAvatar user={{ name: name || '?', color: 'var(--primary)', avatarUrl }} size={48} />
          {canRename ? (
            <div className="flex flex-wrap gap-2">
              <Button type="button" size="sm" variant="outline" render={<label />} nativeButton={false}>
                Upload avatar
                <input type="file" accept="image/png,image/jpeg,image/webp" className="sr-only" onChange={(event) => void pickAvatar(event.target.files?.[0])} />
              </Button>
              {avatarUrl ? <Button type="button" size="sm" variant="ghost" onClick={() => setAvatar(null)}>Remove</Button> : null}
            </div>
          ) : null}
        </div>
        <Field>
          <FieldLabel htmlFor="agent-profile-name">Name</FieldLabel>
          <Input id="agent-profile-name" required maxLength={40} disabled={!canRename} value={name} onChange={(event) => { save.reset(); setName(event.target.value) }} />
          <FieldDescription>
            Its @handle in chat. 2 to 40 characters: letters, digits, single spaces and - _ . ' It cannot be anyone else's name.
            {isDefault ? (canRename ? ' The name and avatar are the same in every workspace; mentions and earlier messages show the new name.' : ' The name and avatar are the same in every workspace, so only an installation administrator changes them.') : ''}
          </FieldDescription>
        </Field>
        <Field>
          <FieldLabel htmlFor="agent-profile-instructions">Instructions</FieldLabel>
          <Textarea id="agent-profile-instructions" className="min-h-32" maxLength={profiles.max_instructions_chars} value={instructions} onChange={(event) => setInstructions(event.target.value)} />
          <FieldDescription>
            Its role, tone and how it works{isDefault ? ' in this workspace, besides the team instructions (Agentic → Workspace) that every agent follows' : ''}. Orbit's own rules stay: it acts with the person's permissions and asks before changes.
          </FieldDescription>
        </Field>
        {isDefault ? null : <fieldset className="flex flex-col gap-3">
          <legend className="mb-1 text-sm font-medium text-foreground">Tools</legend>
          <p className="-mt-1 text-xs text-muted-foreground">Orbit refuses every tool that is not ticked, whatever the instructions say.</p>
          <div className="grid grid-cols-1 gap-4 min-[700px]:grid-cols-2">
            {toolGroups(profiles).map(([group, items]) => (
              <div key={group} className="flex flex-col gap-1.5">
                <span className="text-xs font-medium text-muted-foreground">{group}</span>
                {items.map((tool) => (
                  <label key={tool.name} className="flex items-center gap-2 text-sm text-foreground">
                    <Checkbox checked={tools.includes(tool.name)} onCheckedChange={(on) => setTools((list) => toggle(list, tool.name, on))} />
                    <span className="font-mono text-xs">{tool.name}</span>
                    {tool.writes ? <span className="text-[11px] text-muted-foreground">writes</span> : null}
                  </label>
                ))}
              </div>
            ))}
          </div>
        </fieldset>}
        <fieldset className="flex flex-col gap-2">
          <legend className="mb-1 text-sm font-medium text-foreground">Connected services</legend>
          {profiles.integrations.length === 0 ? (
            <p className="text-xs text-muted-foreground">No connected services yet. Owners and admins add them in Agentic → Workspace.</p>
          ) : (
            <>
              <div className="grid grid-cols-1 gap-1.5 min-[700px]:grid-cols-2">
                {profiles.integrations.map((integration) => (
                  <label key={integration.id} className="flex items-center gap-2 text-sm text-foreground">
                    <Checkbox checked={services.includes(integration.id)} onCheckedChange={(on) => setServices((list) => toggle(list, integration.id, on))} />
                    {integration.name}
                    <span className="text-[11px] text-muted-foreground">{integration.kind === 'mcp' ? 'MCP server' : 'HTTP API'}</span>
                  </label>
                ))}
              </div>
              <p className="text-xs text-muted-foreground">
                Orbit refuses every service that is not ticked, and each service's role rule still applies.{' '}
                {isDefault ? `A service added later is on for ${agent?.name ?? 'the default agent'} until you untick it.` : 'A service added later is off for this agent until you tick it.'}
              </p>
            </>
          )}
        </fieldset>
        <FieldGrid>
          <Field>
            <FieldLabel htmlFor="agent-profile-model">Model</FieldLabel>
            <Select items={models} value={model} onValueChange={(value) => setModel(value ?? DEFAULT)}>
              <SelectTrigger id="agent-profile-model"><SelectValue /></SelectTrigger>
              <SelectContent>
                {models.map((option) => <SelectItem key={option.value} value={option.value}>{option.label}</SelectItem>)}
              </SelectContent>
            </Select>
            {profiles.models ? null : <FieldDescription>The agent service is offline, so only the default model can be chosen now.</FieldDescription>}
          </Field>
          <Field>
            <FieldLabel htmlFor="agent-profile-thinking">Thinking level</FieldLabel>
            <Select items={levels} value={thinking} onValueChange={(value) => setThinking(value ?? DEFAULT)}>
              <SelectTrigger id="agent-profile-thinking"><SelectValue /></SelectTrigger>
              <SelectContent>
                {levels.map((option) => <SelectItem key={option.value} value={option.value}>{option.label}</SelectItem>)}
              </SelectContent>
            </Select>
          </Field>
        </FieldGrid>
        <Field>
          <FieldLabel id="agent-profile-answer-level">Answer level</FieldLabel>
          <ToggleGroup
            aria-labelledby="agent-profile-answer-level"
            variant="outline"
            value={[answerLevel]}
            onValueChange={(value: string[]) => {
              const next = value[0] as AnswerLevel | undefined
              if (next) setAnswerLevel(next)
            }}
          >
            {ANSWER_LEVELS.map((option) => <ToggleGroupItem key={option.value} value={option.value} className="px-3">{option.label}</ToggleGroupItem>)}
          </ToggleGroup>
          <FieldDescription>How much it thinks and writes, for everyone who uses it. {ANSWER_LEVELS.find((option) => option.value === answerLevel)?.help}</FieldDescription>
        </Field>
        {isDefault ? <p className="text-xs text-muted-foreground">{agent?.name} works in every channel and direct message, and cannot be deleted.</p> : <fieldset className="flex flex-col gap-2">
          <legend className="mb-1 text-sm font-medium text-foreground">Channels</legend>
          <label className="flex items-center gap-2 text-sm text-foreground">
            <Switch checked={allChannels} onCheckedChange={setAllChannels} />
            Every channel
          </label>
          {allChannels ? null : (
            <div className="grid grid-cols-1 gap-1.5 min-[700px]:grid-cols-2">
              {channels.map((channel) => (
                <label key={channel.id} className="flex items-center gap-2 text-sm text-foreground">
                  <Checkbox checked={channelIds.includes(channel.id)} onCheckedChange={(on) => setChannelIds((list) => toggle(list, channel.id, on))} />
                  #{channel.name}
                </label>
              ))}
            </div>
          )}
          <p className="text-xs text-muted-foreground">Direct messages with it always work. A mention in another channel gets no answer there: the agent tells the person in a direct message.</p>
        </fieldset>}
        {save.isError ? <p role="alert" className="text-sm text-destructive">{save.error instanceof ApiProblem ? save.error.detail : 'The agent could not be saved.'}</p> : null}
      </form>
      <DialogFooter>
        <DialogClose render={<Button type="button" variant="outline" />}>Cancel</DialogClose>
        <Button type="submit" form="agent-profile-form" disabled={save.isPending || name.trim().length < 2}>{save.isPending ? 'Saving…' : agent ? 'Save' : 'Create agent'}</Button>
      </DialogFooter>
    </Modal>
  )
}
