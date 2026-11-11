import { useState } from 'react'
import { toast } from 'sonner'
import { AGENT_DEFAULT_CHOICE as DEFAULT, type AgentProfile, type AgentProfiles } from '@orbit/api/agent'
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
import { useConversations } from '@/features/chat/api/queries'
import { avatarImage } from '@/features/profile/avatarImage'
import { FieldGrid } from '@/features/settings/components/SettingsParts'
import { useWorkspace } from '@/features/workspaces/workspaceContext'
import { useAgentProfiles, useAgentStatus, useDeleteAgentProfile, useSaveAgentProfile } from './api'

/** Settings → Agentic → Agents: the workspace's agents besides the default one. Owners and admins manage them. */
export function AgentsPage() {
  const { workspace } = useWorkspace()
  const status = useAgentStatus().data
  const defaultName = status?.enabled ? status.name : 'the default agent'
  const profiles = useAgentProfiles(workspace.id)
  const remove = useDeleteAgentProfile(workspace.id)
  const [editing, setEditing] = useState<AgentProfile | 'new' | null>(null)
  const [deleting, setDeleting] = useState<AgentProfile | null>(null)
  const data = profiles.data
  const agents = data?.agents ?? []

  return (
    <SettingsCard
      title="Agents"
      description={`Agents besides ${defaultName}, each with its own name, instructions, tools, model and channels. People @mention one in its channels or send it a direct message. It works with their permissions only and asks for their yes before changes, like ${defaultName}.`}
      actions={data?.can_manage ? <Button size="sm" disabled={agents.length >= data.max_agents} onClick={() => setEditing('new')}>New agent</Button> : null}
    >
      {profiles.isPending ? null : profiles.isError ? (
        <p role="alert" className="text-sm text-destructive">The agents could not be loaded.</p>
      ) : agents.length === 0 ? (
        <p className="text-sm text-muted-foreground">No agents yet.{data?.can_manage ? ' Create one for a job, for example a blog writer that drafts Docs pages.' : ''}</p>
      ) : (
        <ul className="-mx-2 flex flex-col">
          {agents.map((agent) => (
            <li key={agent.id} data-slot="agent-profile" className="flex items-start gap-3 rounded-md px-2 py-3 not-last:border-b not-last:border-border">
              <UserAvatar user={{ name: agent.name, color: 'var(--primary)', avatarUrl: agent.avatar_url }} size={36} />
              <div className="flex min-w-0 flex-1 flex-col gap-1">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-sm font-medium text-foreground">{agent.name}</span>
                  <Badge variant="secondary">{agent.tools.length} tools</Badge>
                  {agent.model ? <Badge variant="outline">{agent.model}</Badge> : null}
                </div>
                <p className="text-xs text-muted-foreground">{channelsLabel(agent)}</p>
                {agent.instructions ? <p className="line-clamp-2 text-sm text-muted-foreground">{agent.instructions}</p> : null}
              </div>
              {data?.can_manage ? (
                <div className="flex shrink-0 gap-1">
                  <Button size="sm" variant="ghost" onClick={() => setEditing(agent)}>Edit</Button>
                  <Button size="sm" variant="ghost" className="text-destructive" onClick={() => setDeleting(agent)}>Delete</Button>
                </div>
              ) : null}
            </li>
          ))}
        </ul>
      )}
      {editing && data ? <AgentDialog profiles={data} agent={editing === 'new' ? null : editing} onClose={() => setEditing(null)} /> : null}
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

function channelsLabel(agent: AgentProfile): string {
  if (agent.all_channels) return 'Every channel and direct messages'
  if (agent.channels.length === 0) return 'Direct messages only'
  return `${agent.channels.map((channel) => `#${channel.name}`).join(', ')} and direct messages`
}

/** The tools by group, in the server's order. */
function toolGroups(profiles: AgentProfiles) {
  const groups = new Map<string, AgentProfiles['tools']>()
  for (const tool of profiles.tools) groups.set(tool.group, [...(groups.get(tool.group) ?? []), tool])
  return [...groups.entries()]
}

/** Create or edit one agent. The avatar is uploaded after the agent is saved. */
function AgentDialog({ profiles, agent, onClose }: { profiles: AgentProfiles; agent: AgentProfile | null; onClose: () => void }) {
  const { workspace } = useWorkspace()
  const save = useSaveAgentProfile(workspace.id)
  const channels = (useConversations().data ?? []).filter((conversation) => conversation.kind !== 'dm' && !conversation.archived && (conversation.kind === 'public' || conversation.isMember))
  const [name, setName] = useState(agent?.name ?? '')
  const [instructions, setInstructions] = useState(agent?.instructions ?? '')
  const [tools, setTools] = useState<string[]>(agent?.tools ?? [])
  const [model, setModel] = useState(agent?.model ?? DEFAULT)
  const [thinking, setThinking] = useState(agent?.thinking ?? DEFAULT)
  const [allChannels, setAllChannels] = useState(agent?.all_channels ?? false)
  const [channelIds, setChannelIds] = useState<string[]>(agent?.channels.map((channel) => channel.id) ?? [])
  const [avatar, setAvatar] = useState<{ blob: Blob; url: string } | null | undefined>(undefined)
  const models = [{ value: DEFAULT, label: 'Default (as Agentic → Admin)' }, ...(profiles.models?.models ?? []).map((option) => ({ value: option.id, label: option.name }))]
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
    save.mutate(
      {
        id: agent?.id ?? null,
        input: {
          name: name.trim(),
          instructions,
          tools,
          model: model === DEFAULT ? null : model,
          thinking: thinking === DEFAULT ? null : thinking,
          all_channels: allChannels,
          channel_ids: channelIds,
        },
        avatar: avatar === undefined ? undefined : (avatar?.blob ?? null),
      },
      { onSuccess: (saved) => { toast(`${saved.name} saved`); onClose() } },
    )
  }

  return (
    <Modal title={agent ? `Edit ${agent.name}` : 'New agent'} onClose={onClose} className="sm:max-w-2xl">
      <form id="agent-profile-form" onSubmit={submit} className="flex max-h-[70vh] flex-col gap-5 overflow-y-auto pr-1">
        <div className="flex items-center gap-3">
          <UserAvatar user={{ name: name || '?', color: 'var(--primary)', avatarUrl }} size={48} />
          <div className="flex flex-wrap gap-2">
            <Button type="button" size="sm" variant="outline" render={<label />} nativeButton={false}>
              Upload avatar
              <input type="file" accept="image/png,image/jpeg,image/webp" className="sr-only" onChange={(event) => void pickAvatar(event.target.files?.[0])} />
            </Button>
            {avatarUrl ? <Button type="button" size="sm" variant="ghost" onClick={() => setAvatar(null)}>Remove</Button> : null}
          </div>
        </div>
        <Field>
          <FieldLabel htmlFor="agent-profile-name">Name</FieldLabel>
          <Input id="agent-profile-name" required maxLength={40} value={name} onChange={(event) => { save.reset(); setName(event.target.value) }} />
          <FieldDescription>Its @handle in chat. 2 to 40 characters: letters, digits, single spaces and - _ . ' It cannot be anyone else's name.</FieldDescription>
        </Field>
        <Field>
          <FieldLabel htmlFor="agent-profile-instructions">Instructions</FieldLabel>
          <Textarea id="agent-profile-instructions" className="min-h-32" maxLength={profiles.max_instructions_chars} value={instructions} onChange={(event) => setInstructions(event.target.value)} />
          <FieldDescription>Its role, tone and how it works. Orbit's own rules stay: it acts with the person's permissions and asks before changes.</FieldDescription>
        </Field>
        <fieldset className="flex flex-col gap-3">
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
        <fieldset className="flex flex-col gap-2">
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
        </fieldset>
        {save.isError ? <p role="alert" className="text-sm text-destructive">{save.error instanceof ApiProblem ? save.error.detail : 'The agent could not be saved.'}</p> : null}
      </form>
      <DialogFooter>
        <DialogClose render={<Button type="button" variant="outline" />}>Cancel</DialogClose>
        <Button type="submit" form="agent-profile-form" disabled={save.isPending || name.trim().length < 2}>{save.isPending ? 'Saving…' : agent ? 'Save' : 'Create agent'}</Button>
      </DialogFooter>
    </Modal>
  )
}
