import { toast } from 'sonner'
import { useState, type FormEvent } from 'react'
import { Category, Code } from 'reicon-react'
import type { AgentIntegration, AgentIntegrationKind, AgentIntegrationMethod, AgentIntegrationRole } from '@orbit/api/agentIntegrations'
import { confirmAction } from '@/components/common/confirmAction'
import { SettingsCard } from '@/components/common/SettingsCard'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Field, FieldDescription, FieldLabel, FieldLegend, FieldSet } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { PasswordInput } from '@/components/ui/password-input'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Switch } from '@/components/ui/switch'
import { relativeTime } from '@/lib/format'
import { FieldGrid, RequiredMark, RowIcon, SettingsRow } from '@/features/settings/components/SettingsParts'
import { useCan } from '@/features/workspaces/permissions'
import { useWorkspace } from '@/features/workspaces/workspaceContext'
import { useAgentIntegrations, useCreateAgentIntegration, useDeleteAgentIntegration, useTestAgentIntegration, useUpdateAgentIntegration } from './api'

const KINDS = [
  { value: 'mcp', label: 'Remote MCP server' },
  { value: 'http', label: 'HTTP API' },
] as const
const ROLES: AgentIntegrationRole[] = ['owner', 'admin', 'member']
const METHODS: AgentIntegrationMethod[] = ['GET', 'HEAD', 'POST', 'PUT', 'PATCH', 'DELETE']

const message = (error: unknown, fallback: string) => (error instanceof Error && error.message ? error.message : fallback)
const toggled = <T,>(list: readonly T[], value: T, on: boolean) => (on ? [...list, value] : list.filter((item) => item !== value))

/** Workspace settings → Agent tools: MCP servers and HTTP APIs the team agent may call. Owners and admins only. */
export function AgentToolsPage() {
  const { workspace } = useWorkspace()
  const canManage = useCan('integrations.manage')
  const integrations = useAgentIntegrations(workspace.id, canManage)

  if (!canManage) return <SettingsCard title="Agent tools"><p className="m-0 text-muted-foreground">Only workspace owners and administrators can set up agent tools.</p></SettingsCard>

  return <>
    <AddIntegration canKeepSecrets={integrations.data?.can_keep_secrets !== false} />
    {integrations.isPending ? <p className="m-0 text-muted-foreground">Loading agent tools…</p> : null}
    {integrations.isError ? <p className="m-0 text-destructive" role="alert">Agent tools could not be loaded.</p> : null}
    {integrations.data?.items.map((integration) => <IntegrationCard key={integration.id} integration={integration} />)}
  </>
}

function AddIntegration({ canKeepSecrets }: { canKeepSecrets: boolean }) {
  const { workspace } = useWorkspace()
  const create = useCreateAgentIntegration(workspace.id)
  const test = useTestAgentIntegration(workspace.id)
  const [kind, setKind] = useState<AgentIntegrationKind>('mcp')
  const [name, setName] = useState('')
  const [url, setUrl] = useState('')
  const [header, setHeader] = useState('Authorization')
  const [secret, setSecret] = useState('')
  const [roles, setRoles] = useState<AgentIntegrationRole[]>(['owner', 'admin', 'member'])

  async function submit(event: FormEvent) {
    event.preventDefault()
    if (!name.trim() || !url.trim() || roles.length === 0) return
    const created = await create.mutateAsync({
      kind,
      name: name.trim(),
      url: url.trim(),
      allowed_roles: roles,
      ...(secret ? { auth_header: header.trim() || 'Authorization', secret } : {}),
    })
    setName('')
    setUrl('')
    setSecret('')
    // An MCP server's tools are known only after a test.
    if (created.kind === 'mcp') {
      const result = await test.mutateAsync(created.id)
      if (result.ok) toast(result.detail)
      else toast.error(result.detail)
    }
  }

  return <SettingsCard title="Add an agent tool" description="The team agent can call these services as the person who asked it. Orbit keeps the credential and makes every call; changes outside Orbit need that person's yes in the chat thread.">
    <form onSubmit={(event) => void submit(event)}>
      <FieldGrid>
        <Field>
          <FieldLabel htmlFor="agent-tool-kind">Kind</FieldLabel>
          <Select items={KINDS} value={kind} onValueChange={(value) => setKind(value as AgentIntegrationKind)}>
            <SelectTrigger id="agent-tool-kind" aria-label="Kind"><SelectValue /></SelectTrigger>
            <SelectContent>{KINDS.map((option) => <SelectItem key={option.value} value={option.value}>{option.label}</SelectItem>)}</SelectContent>
          </Select>
        </Field>
        <Field><FieldLabel htmlFor="agent-tool-name">Name <RequiredMark /></FieldLabel><Input id="agent-tool-name" value={name} maxLength={80} onChange={(event) => setName(event.target.value)} placeholder={kind === 'mcp' ? 'Cloudflare' : 'Shopify'} /></Field>
        <Field className="min-[900px]:col-span-2">
          <FieldLabel htmlFor="agent-tool-url">{kind === 'mcp' ? 'Server URL' : 'Base URL'} <RequiredMark /></FieldLabel>
          <Input id="agent-tool-url" value={url} maxLength={2048} onChange={(event) => setUrl(event.target.value)} placeholder={kind === 'mcp' ? 'https://mcp.example.com/mcp' : 'https://api.example.com/v1'} />
          <FieldDescription>{kind === 'mcp' ? 'The Streamable HTTP endpoint of the server.' : 'Every request of the agent stays below this URL.'} HTTPS only; local and private addresses are refused.</FieldDescription>
        </Field>
        <Field><FieldLabel htmlFor="agent-tool-header">Credential header</FieldLabel><Input id="agent-tool-header" value={header} maxLength={100} onChange={(event) => setHeader(event.target.value)} /></Field>
        <Field>
          <FieldLabel htmlFor="agent-tool-secret">Secret</FieldLabel>
          <PasswordInput id="agent-tool-secret" value={secret} autoComplete="off" disabled={!canKeepSecrets} onChange={(event) => setSecret(event.target.value)} placeholder="Bearer …" />
          <FieldDescription>{canKeepSecrets ? 'The full header value, for example “Bearer <token>”. Shared by everyone who may use this tool. Orbit never shows it again.' : 'This server has no ORBIT_APP_KEY, so it cannot keep secrets.'}</FieldDescription>
        </Field>
        <RolesField value={roles} onChange={setRoles} />
        <Button className="justify-self-start" type="submit" disabled={create.isPending || test.isPending || !name.trim() || !url.trim() || roles.length === 0}>Add</Button>
        {create.isError ? <p className="text-destructive min-[900px]:col-span-2" role="alert">{message(create.error, 'The tool could not be added.')}</p> : null}
      </FieldGrid>
    </form>
  </SettingsCard>
}

function RolesField({ value, onChange }: { value: AgentIntegrationRole[]; onChange: (roles: AgentIntegrationRole[]) => void }) {
  return <FieldSet className="flex-row flex-wrap gap-[18px]">
    <FieldLegend variant="label">Who may use it</FieldLegend>
    {ROLES.map((role) => <Label key={role} className="font-normal capitalize"><Checkbox checked={value.includes(role)} onCheckedChange={(on) => onChange(toggled(value, role, on))} /> {role}s</Label>)}
  </FieldSet>
}

function IntegrationCard({ integration }: { integration: AgentIntegration }) {
  const { workspace } = useWorkspace()
  const update = useUpdateAgentIntegration(workspace.id)
  const test = useTestAgentIntegration(workspace.id)
  const remove = useDeleteAgentIntegration(workspace.id)
  const [secret, setSecret] = useState('')
  const enabled = integration.tools?.filter((tool) => tool.enabled).map((tool) => tool.name) ?? []

  const save = (input: Parameters<typeof update.mutate>[0]['input'], done?: string) =>
    update.mutate({ id: integration.id, input }, {
      onSuccess: () => { if (done) toast(done) },
      onError: (error) => toast.error(message(error, 'The change could not be saved.')),
    })

  async function runTest() {
    const result = await test.mutateAsync(integration.id)
    if (result.ok) toast(result.detail)
    else toast.error(result.detail)
  }

  const actions = <>
    <Button variant="outline" type="button" disabled={test.isPending} onClick={() => void runTest()}>{test.isPending ? 'Testing…' : 'Test'}</Button>
    <Button variant="ghost" type="button" disabled={remove.isPending} onClick={async () => {
      if (await confirmAction({ title: `Remove ${integration.name}?`, description: 'The agent loses these tools at once. The saved secret is deleted.', confirmLabel: 'Remove', danger: true })) remove.mutate(integration.id)
    }}>Remove</Button>
  </>

  return <SettingsCard title={integration.name} description={`${integration.kind === 'mcp' ? 'MCP server' : 'HTTP API'} · tools named ${integration.slug}__…`} actions={actions} flush>
    <div className="flex flex-col divide-y">
      <SettingsRow className="flex-nowrap">
        <RowIcon>{integration.kind === 'mcp' ? <Category /> : <Code />}</RowIcon>
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <code className="text-[13px] [overflow-wrap:anywhere] text-foreground">{integration.url}</code>
          <span className="text-xs text-muted-foreground/70">{integration.secret_set ? `Credential set in ${integration.auth_header}` : 'No credential'} · changed {relativeTime(new Date(integration.updated_at).toISOString())}</span>
        </div>
      </SettingsRow>
      <SettingsRow>
        <RolesField value={integration.allowed_roles} onChange={(roles) => { if (roles.length) save({ allowed_roles: roles }) }} />
      </SettingsRow>
      <SettingsRow>
        <form className="flex w-full flex-wrap items-end gap-2" onSubmit={(event) => { event.preventDefault(); if (secret) save({ secret, auth_header: integration.auth_header ?? 'Authorization' }, 'Secret replaced'); setSecret('') }}>
          <Field className="min-w-60 flex-1">
            <FieldLabel htmlFor={`agent-tool-secret-${integration.id}`}>Replace secret</FieldLabel>
            <PasswordInput id={`agent-tool-secret-${integration.id}`} value={secret} autoComplete="off" onChange={(event) => setSecret(event.target.value)} placeholder={integration.secret_set ? 'Saved; type to replace' : 'Bearer …'} />
          </Field>
          <Button variant="outline" type="submit" disabled={!secret || update.isPending}>Save secret</Button>
          {integration.secret_set ? <Button variant="ghost" type="button" disabled={update.isPending} onClick={() => save({ secret: '' }, 'Secret removed')}>Remove secret</Button> : null}
        </form>
      </SettingsRow>
      {integration.kind === 'http' ? <SettingsRow>
        <FieldSet className="flex-row flex-wrap gap-[18px]">
          <FieldLegend variant="label">Allowed methods</FieldLegend>
          {METHODS.map((method) => <Label key={method} className="font-normal"><Checkbox checked={integration.allowed_methods.includes(method)} onCheckedChange={(on) => {
            const methods = toggled(integration.allowed_methods, method, on)
            if (methods.length) save({ allowed_methods: methods })
          }} /> {method}</Label>)}
          <FieldDescription className="basis-full">Methods other than GET and HEAD run only after the person who asked says yes in the thread.</FieldDescription>
        </FieldSet>
      </SettingsRow> : null}
      {integration.kind === 'mcp' && !integration.tools ? <SettingsRow className="text-muted-foreground">Test the server to see its tools.</SettingsRow> : null}
      {integration.tools?.length === 0 ? <SettingsRow className="text-muted-foreground">The server lists no tools.</SettingsRow> : null}
      {integration.tools?.map((tool) => <SettingsRow className="flex-nowrap" key={tool.name}>
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="flex flex-wrap items-center gap-2 text-[13px] font-medium text-foreground"><code>{tool.name}</code>{tool.read_only ? <Badge variant="secondary">Read-only</Badge> : <Badge variant="outline">Needs approval</Badge>}</span>
          {tool.description ? <span className="line-clamp-2 text-xs text-muted-foreground/70">{tool.description}</span> : null}
        </div>
        <Switch aria-label={`Allow ${tool.name}`} checked={tool.enabled} disabled={update.isPending} onCheckedChange={(on) => save({ enabled_tools: toggled(enabled, tool.name, on) })} />
      </SettingsRow>)}
    </div>
  </SettingsCard>
}
