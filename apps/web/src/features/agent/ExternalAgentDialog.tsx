import { useState } from 'react'
import { toast } from 'sonner'
import { type AgentProfile, type ExternalAgentScope, type IssuedAgentToken } from '@orbit/api/agent'
import { mcpSetup } from '@orbit/api/mcp'
import { ApiProblem } from '@orbit/api/problem'
import { CopyCodeButton } from '@/components/common/CopyCodeButton'
import { Modal } from '@/components/common/Modal'
import { UserAvatar } from '@/components/common/UserAvatar'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { DialogClose, DialogFooter } from '@/components/ui/dialog'
import { Field, FieldDescription, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Switch } from '@/components/ui/switch'
import { useOAuthGrants } from '@/features/auth/oauth'
import { useConversations } from '@/features/chat/api/queries'
import { avatarImage } from '@/features/profile/avatarImage'
import { useProjects } from '@/features/tasks/api/projects'
import { useWorkspace } from '@/features/workspaces/workspaceContext'
import { useAgentProfiles, useAgentPushSecret, useCreateAgentToken, useRevokeAgentCredential, useSaveExternalAgent } from './api'
import { deliveryLabel, lastSeenLabel, PUSH_STATE_LABELS, SCOPE_ROWS } from './externalAgent'

const DAYS = [30, 90, 365] as const
const DEFAULT_SCOPES: ExternalAgentScope[] = ['tasks:read', 'chat:read', 'chat:write']

const date = (millis: number) => new Date(millis).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' })

/**
 * New agent → External, and an external agent's card: an outside client (Claude Code, Codex, a server) runs it through
 * Orbit's MCP as the agent's bot user. Its card is the most it can do. After it is created, the dialog shows how to
 * connect it.
 */
export function ExternalAgentDialog({ agent, onSaved, onClose }: { agent: AgentProfile | null; onSaved: (agent: AgentProfile) => void; onClose: () => void }) {
  const { workspace } = useWorkspace()
  const save = useSaveExternalAgent(workspace.id)
  const card = agent?.external
  const channels = (useConversations().data ?? []).filter((conversation) => conversation.kind !== 'dm' && !conversation.archived && (conversation.kind === 'public' || conversation.isMember))
  const projects = useProjects(workspace.id).data ?? []
  const [name, setName] = useState(agent?.name ?? '')
  const [channelIds, setChannelIds] = useState<string[]>(agent?.channels.map((channel) => channel.id) ?? [])
  const [projectIds, setProjectIds] = useState<string[]>(card?.projects.map((project) => project.id) ?? [])
  const [scopes, setScopes] = useState<ExternalAgentScope[]>(card?.scopes ?? DEFAULT_SCOPES)
  const [allowDms, setAllowDms] = useState(card?.allow_dms ?? true)
  const [pushUrl, setPushUrl] = useState(card?.push_url ?? '')
  const [avatar, setAvatar] = useState<{ blob: Blob; url: string } | null | undefined>(undefined)
  const avatarUrl = avatar === undefined ? agent?.avatar_url : (avatar?.url ?? null)
  const toggle = <T,>(list: T[], item: T, on: boolean) => (on ? [...list, item] : list.filter((own) => own !== item))
  const setScope = (scope: ExternalAgentScope, on: boolean) =>
    setScopes((list) => {
      const next = toggle(list, scope, on)
      // Write includes read.
      const row = SCOPE_ROWS.find((item) => item.write === scope || item.read === scope)
      if (on && row && scope === row.write && !next.includes(row.read)) next.push(row.read)
      if (!on && row && scope === row.read) return next.filter((own) => own !== row.write)
      return next
    })

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
        input: { name: name.trim(), channel_ids: channelIds, project_ids: projectIds, scopes: scopes.filter((scope) => scope !== 'tasks:read'), allow_dms: allowDms, push_url: pushUrl.trim() },
        avatar: avatar === undefined ? undefined : (avatar?.blob ?? null),
      },
      {
        onSuccess: (saved) => {
          toast(`${saved.name} saved`)
          setAvatar(undefined)
          onSaved(saved)
        },
      },
    )
  }

  return (
    <Modal title={agent ? `Edit ${agent.name}` : 'New external agent'} onClose={onClose} className="sm:max-w-2xl">
      <form id="external-agent-form" onSubmit={submit} className="flex max-h-[70vh] flex-col gap-5 overflow-y-auto pr-1">
        <p className="text-sm text-muted-foreground">
          An outside agent, such as Claude Code, Codex or your own server, runs it. It connects to Orbit's MCP as this agent, reads what people send it and
          answers in the thread. Orbit keeps it to this card, whatever its prompt says.
        </p>
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
          <FieldLabel htmlFor="external-agent-name">Name</FieldLabel>
          <Input id="external-agent-name" required maxLength={40} value={name} onChange={(event) => { save.reset(); setName(event.target.value) }} />
          <FieldDescription>Its @handle in chat. 2 to 40 characters: letters, digits, single spaces and - _ . ' It cannot be anyone else's name.</FieldDescription>
        </Field>
        <p role="note" data-slot="external-agent-warning" className="rounded-md border border-border bg-muted/50 px-3 py-2 text-sm text-foreground">
          Messages that mention it in these channels, and its direct messages, go to the connected client, outside Orbit.
        </p>
        <fieldset className="flex flex-col gap-2">
          <legend className="mb-1 text-sm font-medium text-foreground">Channels</legend>
          {channels.length === 0 ? <p className="text-xs text-muted-foreground">No channels yet.</p> : (
            <div className="grid grid-cols-1 gap-1.5 min-[700px]:grid-cols-2">
              {channels.map((channel) => (
                <label key={channel.id} className="flex items-center gap-2 text-sm text-foreground">
                  <Checkbox checked={channelIds.includes(channel.id)} onCheckedChange={(on) => setChannelIds((list) => toggle(list, channel.id, on))} />
                  #{channel.name}
                  {channel.kind === 'private' ? <span className="text-[11px] text-muted-foreground">private</span> : null}
                </label>
              ))}
            </div>
          )}
          <label className="flex items-center gap-2 text-sm text-foreground">
            <Switch checked={allowDms} onCheckedChange={setAllowDms} />
            Allow direct messages
          </label>
          <p className="text-xs text-muted-foreground">It works only in the ticked channels: it never joins another one. A mention elsewhere gets an answer in a direct message that names its channels.</p>
        </fieldset>
        <fieldset className="flex flex-col gap-2">
          <legend className="mb-1 text-sm font-medium text-foreground">Projects</legend>
          {projects.length === 0 ? <p className="text-xs text-muted-foreground">No projects yet.</p> : (
            <div className="grid grid-cols-1 gap-1.5 min-[700px]:grid-cols-2">
              {projects.map((project) => (
                <label key={project.id} className="flex items-center gap-2 text-sm text-foreground">
                  <Checkbox checked={projectIds.includes(project.id)} onCheckedChange={(on) => setProjectIds((list) => toggle(list, project.id, on))} />
                  {project.name}
                </label>
              ))}
            </div>
          )}
          <p className="text-xs text-muted-foreground">Its tasks, and the docs of their teamspaces. A private project also needs the agent on the project's member list.</p>
        </fieldset>
        <fieldset className="flex flex-col gap-2">
          <legend className="mb-1 text-sm font-medium text-foreground">Permissions</legend>
          {SCOPE_ROWS.map((row) => (
            <div key={row.label} className="flex flex-wrap items-center gap-x-4 gap-y-1">
              <span className="w-14 text-sm text-foreground">{row.label}</span>
              <label className="flex items-center gap-2 text-sm text-foreground">
                <Checkbox checked={scopes.includes(row.read)} disabled={row.read === 'tasks:read'} onCheckedChange={(on) => setScope(row.read, on)} aria-label={`${row.label} read`} />
                Read
              </label>
              <label className="flex items-center gap-2 text-sm text-foreground">
                <Checkbox checked={scopes.includes(row.write)} onCheckedChange={(on) => setScope(row.write, on)} aria-label={`${row.label} write`} />
                Write
              </label>
              <span className="basis-full text-xs text-muted-foreground min-[700px]:basis-auto">{row.hint}</span>
            </div>
          ))}
          <p className="text-xs text-muted-foreground">Writes need no yes from a person: these ticks are the limit. Every write is in the audit log. No Drive access.</p>
        </fieldset>
        <fieldset className="flex flex-col gap-2">
          <legend className="mb-1 text-sm font-medium text-foreground">Push (for a server)</legend>
          <Input id="external-agent-push-url" aria-label="Push URL" type="url" placeholder="https://agent.example.com/orbit/events" value={pushUrl} onChange={(event) => { save.reset(); setPushUrl(event.target.value) }} />
          <p className="text-xs text-muted-foreground">
            Orbit sends each message for the agent to this https address at once, signed like Slack events (X-Orbit-Signature). If it does not answer with 2xx, Orbit tries again after
            10 s, 60 s and 5 min; then the message waits for wait_for_messages. Leave it empty to only pull.
          </p>
          {agent ? <Push agent={agent} /> : null}
        </fieldset>
        {agent ? <Connection agent={agent} /> : null}
        {save.isError ? <p role="alert" className="text-sm text-destructive">{save.error instanceof ApiProblem ? save.error.detail : 'The agent could not be saved.'}</p> : null}
      </form>
      <DialogFooter>
        <DialogClose render={<Button type="button" variant="outline" />}>{agent ? 'Close' : 'Cancel'}</DialogClose>
        <Button type="submit" form="external-agent-form" disabled={save.isPending || name.trim().length < 2}>{save.isPending ? 'Saving…' : agent ? 'Save' : 'Create agent'}</Button>
      </DialogFooter>
    </Modal>
  )
}

/** The push signing secret (Reveal, Rotate) and the newest push tries. */
function Push({ agent }: { agent: AgentProfile }) {
  const { workspace } = useWorkspace()
  const secret = useAgentPushSecret(workspace.id)
  // The card while the dialog is open: tries come in the background.
  const card = (useAgentProfiles(workspace.id, true, 5_000).data?.agents.find((own) => own.id === agent.id) ?? agent).external
  const deliveries = card?.deliveries ?? []
  const failed = (error: unknown) => toast.error(error instanceof ApiProblem ? error.detail : 'That did not work. Try again.')

  return (
    <div data-slot="external-agent-push" className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-[13px] font-medium text-foreground">Signing secret</span>
        <Button type="button" size="sm" variant="outline" disabled={secret.isPending} onClick={() => secret.mutate({ id: agent.id, rotate: false }, { onError: failed })}>Reveal</Button>
        <Button type="button" size="sm" variant="ghost" disabled={secret.isPending} onClick={() => secret.mutate({ id: agent.id, rotate: true }, { onSuccess: () => toast('New signing secret: pushes use it now.'), onError: failed })}>Rotate</Button>
        {card?.push_state ? <Badge variant={card.push_state === 'failing' ? 'destructive' : card.push_state === 'retrying' ? 'outline' : 'secondary'}>{PUSH_STATE_LABELS[card.push_state]}</Badge> : null}
      </div>
      {secret.data ? <CodeBlock text={secret.data} /> : null}
      <p className="text-xs text-muted-foreground">Check X-Orbit-Signature: v0= and the hex HMAC-SHA256 of v0:{'{timestamp}'}:{'{body}'} with this secret; refuse a timestamp older than 5 minutes. Use event_id to drop repeats.</p>
      {deliveries.length ? (
        <div className="flex flex-col gap-1">
          <p className="text-[13px] font-medium text-foreground">Last pushes</p>
          <ul data-slot="external-agent-deliveries" className="flex flex-col">
            {deliveries.map((delivery) => (
              <li key={`${delivery.event_id}-${delivery.attempt}`} className="flex flex-wrap items-center gap-x-3 gap-y-0.5 py-1 text-xs not-last:border-b not-last:border-border">
                <span className="text-muted-foreground">{new Date(delivery.created_at).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit', second: '2-digit' })}</span>
                <span className="font-mono text-foreground">{delivery.event_id}</span>
                <span className={delivery.status !== null && delivery.status >= 200 && delivery.status < 300 ? 'text-foreground' : 'text-destructive'}>{deliveryLabel(delivery)}</span>
                <span className="ml-auto text-muted-foreground">try {delivery.attempt} of 4</span>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </div>
  )
}

/** How to connect the agent, its tokens and its OAuth connections. */
function Connection({ agent }: { agent: AgentProfile }) {
  const { workspace } = useWorkspace()
  const { url } = mcpSetup(window.location.origin, useOAuthGrants().data?.mcp_url)
  const create = useCreateAgentToken(workspace.id)
  const revoke = useRevokeAgentCredential(workspace.id)
  const [days, setDays] = useState<(typeof DAYS)[number]>(90)
  const [issued, setIssued] = useState<IssuedAgentToken | null>(null)
  const card = agent.external
  const server = `orbit-${agent.name.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '')}`
  const failed = (error: unknown) => toast.error(error instanceof ApiProblem ? error.detail : 'That did not work. Try again.')

  return (
    <section data-slot="external-agent-connection" className="flex flex-col gap-4 border-t border-border pt-4">
      <div className="flex flex-wrap items-baseline justify-between gap-2">
        <h3 className="text-sm font-medium text-foreground">Connection</h3>
        <span className="text-xs text-muted-foreground">{lastSeenLabel(card?.last_seen_at)}</span>
      </div>
      <div className="flex flex-col gap-1.5">
        <p className="text-[13px] font-medium text-foreground">Connect with OAuth (Claude Code, Codex)</p>
        <CodeBlock text={`claude mcp add --scope user --transport http ${server} ${url}`} />
        <p className="text-xs text-muted-foreground">
          Then in Claude Code: /mcp → Authenticate (or codex mcp add {server} --url {url} and codex mcp login {server}). On Orbit's page, an owner or admin chooses
          “Connect as {agent.name}”.
        </p>
      </div>
      <div className="flex flex-col gap-2">
        <p className="text-[13px] font-medium text-foreground">Agent token (for a server)</p>
        <div className="flex flex-wrap items-center gap-2">
          <Select items={DAYS.map((value) => ({ value: String(value), label: `${value} days` }))} value={String(days)} onValueChange={(value) => setDays(Number(value) as (typeof DAYS)[number])}>
            <SelectTrigger className="w-32" aria-label="Token expiry"><SelectValue /></SelectTrigger>
            <SelectContent>
              {DAYS.map((value) => <SelectItem key={value} value={String(value)}>{value} days</SelectItem>)}
            </SelectContent>
          </Select>
          <Button type="button" size="sm" variant="outline" disabled={create.isPending} onClick={() => create.mutate({ id: agent.id, days }, { onSuccess: setIssued, onError: failed })}>
            {create.isPending ? 'Creating…' : 'Create token'}
          </Button>
        </div>
        {issued ? (
          <div className="flex flex-col gap-1.5">
            <CodeBlock text={issued.token} />
            <p className="text-xs text-muted-foreground">Copy it now: Orbit shows it only once. Send it as Authorization: Bearer to {url}, for example:</p>
            <CodeBlock text={`claude mcp add --transport http ${server} ${url} --header "Authorization: Bearer ${issued.token}"`} />
          </div>
        ) : null}
        {card?.tokens.length ? (
          <ul className="flex flex-col">
            {card.tokens.map((token) => (
              <li key={token.id} className="flex flex-wrap items-center gap-x-3 gap-y-1 py-1.5 text-sm not-last:border-b not-last:border-border">
                <span className="font-mono text-xs text-foreground">{token.prefix}…</span>
                <span className="text-xs text-muted-foreground">Expires {date(token.expires_at)} · {token.last_used_at ? `used ${date(token.last_used_at)}` : 'never used'}</span>
                <Button type="button" size="sm" variant="ghost" className="ml-auto text-destructive" onClick={() => revoke.mutate({ id: agent.id, kind: 'tokens', credentialId: token.id }, { onError: failed })}>Revoke</Button>
              </li>
            ))}
          </ul>
        ) : null}
      </div>
      {card?.connections.length ? (
        <div className="flex flex-col gap-1.5">
          <p className="text-[13px] font-medium text-foreground">OAuth connections</p>
          <ul className="flex flex-col">
            {card.connections.map((connection) => (
              <li key={connection.id} className="flex flex-wrap items-center gap-x-3 gap-y-1 py-1.5 text-sm not-last:border-b not-last:border-border">
                <span className="text-foreground">{connection.client_name}</span>
                <span className="text-xs text-muted-foreground">{connection.approved_by ? `Allowed by ${connection.approved_by}` : 'Allowed'} · {date(connection.created_at)}</span>
                <Button type="button" size="sm" variant="ghost" className="ml-auto text-destructive" onClick={() => revoke.mutate({ id: agent.id, kind: 'connections', credentialId: connection.id }, { onError: failed })}>Disconnect</Button>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </section>
  )
}

function CodeBlock({ text }: { text: string }) {
  return (
    <div className="group relative rounded-md bg-muted/50 py-2.5 pr-20 pl-3">
      <pre className="font-mono text-xs leading-5 break-all whitespace-pre-wrap text-foreground">{text}</pre>
      <CopyCodeButton getText={() => text} className="absolute top-1.5 right-1.5" />
    </div>
  )
}
