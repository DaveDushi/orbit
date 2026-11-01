import { toast } from 'sonner'
import { cn } from 'cn'
import { useState, type ComponentProps, type FormEvent } from 'react'
import { Category, Code, Trash } from 'reicon-react'
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
const AUTH_TYPES = [
  { value: 'header', label: 'Fixed header credential' },
  { value: 'oauth', label: 'OAuth client credentials' },
] as const
const SHOPIFY_HEADER = 'X-Shopify-Access-Token'
const SHOPIFY_TEST_PATH = 'shop.json'
const SHOPIFY_GRAPHQL_PATH = '/graphql.json'
const ROLES: AgentIntegrationRole[] = ['owner', 'admin', 'member']
const METHODS: AgentIntegrationMethod[] = ['GET', 'HEAD', 'POST', 'PUT', 'PATCH', 'DELETE']

const message = (error: unknown, fallback: string) => (error instanceof Error && error.message ? error.message : fallback)
/** The token URL of a Shopify store's base URL. Shopify is only a preset: the values stay editable. */
function shopifyTokenUrl(url: string) {
  try {
    const { host } = new URL(url.trim())
    return host.endsWith('.myshopify.com') ? `https://${host}/admin/oauth/access_token` : null
  } catch {
    return null
  }
}
const DOCS_URL = 'https://github.com/DaveDushi/orbit/blob/main/docs/agent.md#agent-tools-integrations'
const toggled = <T,>(list: readonly T[], value: T, on: boolean) => (on ? [...list, value] : list.filter((item) => item !== value))

/** Workspace settings → Agent → Tools: MCP servers and HTTP APIs the team agent may call. Owners and admins only. */
export function AgentToolsPage() {
  const { workspace } = useWorkspace()
  const canManage = useCan('integrations.manage')
  const integrations = useAgentIntegrations(workspace.id, canManage)
  const [adding, setAdding] = useState(false)

  if (!canManage) return <SettingsCard title="Tools"><p className="m-0 text-muted-foreground">Only workspace owners and administrators can set up agent tools.</p></SettingsCard>

  const items = integrations.data?.items
  return <SettingsCard
    title="Connected services"
    description={<>The agent calls these services as the person who asked. Orbit keeps the credential and makes every call; changes outside Orbit need that person's yes in the thread. <a className="text-primary underline-offset-4 hover:underline" href={DOCS_URL} target="_blank" rel="noreferrer">Docs</a></>}
    actions={adding ? null : <Button variant="outline" type="button" onClick={() => setAdding(true)}>Add tool</Button>}
    flush
  >
    <div className="flex flex-col divide-y">
      {adding ? <AddIntegration canKeepSecrets={integrations.data?.can_keep_secrets !== false} onClose={() => setAdding(false)} /> : null}
      {integrations.isPending ? <SettingsRow className="text-muted-foreground">Loading agent tools…</SettingsRow> : null}
      {integrations.isError ? <SettingsRow className="text-destructive" role="alert">Agent tools could not be loaded.</SettingsRow> : null}
      {items?.length === 0 && !adding ? <SettingsRow className="text-muted-foreground">No tools yet.</SettingsRow> : null}
      {items?.map((integration) => <IntegrationRow key={integration.id} integration={integration} />)}
    </div>
  </SettingsCard>
}

/** Fields that most tools do not need. Collapsed fields stay in the form, so they keep their values and are still sent. */
function Advanced({ fields, className, children, ...props }: ComponentProps<'details'> & { fields: string }) {
  return <details className={cn('min-w-0', className)} {...props}>
    <summary className="cursor-pointer text-[13px] font-medium text-foreground select-none">Advanced <span className="font-normal text-muted-foreground">{fields}</span></summary>
    <div className="mt-3">{children}</div>
  </details>
}

function AddIntegration({ canKeepSecrets, onClose }: { canKeepSecrets: boolean; onClose: () => void }) {
  const { workspace } = useWorkspace()
  const create = useCreateAgentIntegration(workspace.id)
  const test = useTestAgentIntegration(workspace.id)
  const [kind, setKind] = useState<AgentIntegrationKind>('mcp')
  const [name, setName] = useState('')
  const [url, setUrl] = useState('')
  const [header, setHeader] = useState('Authorization')
  const [secret, setSecret] = useState('')
  const [roles, setRoles] = useState<AgentIntegrationRole[]>(['owner', 'admin', 'member'])
  const [authType, setAuthType] = useState<'header' | 'oauth'>('header')
  const [tokenUrl, setTokenUrl] = useState('')
  const [clientId, setClientId] = useState('')
  const [clientSecret, setClientSecret] = useState('')
  const [testPath, setTestPath] = useState('')
  const [graphqlPath, setGraphqlPath] = useState('')
  const oauth = kind === 'http' && authType === 'oauth'
  const oauthIncomplete = oauth && (!tokenUrl.trim() || !clientId.trim() || !clientSecret.trim())

  /** Fills in or takes back the Shopify values, and leaves what the admin typed. */
  function applyPreset(before: string | null, after: string | null) {
    if (before === after) return
    if (!tokenUrl || tokenUrl === before) setTokenUrl(after ?? '')
    if (after && header === 'Authorization') setHeader(SHOPIFY_HEADER)
    if (!after && header === SHOPIFY_HEADER) setHeader('Authorization')
    if (after && !testPath) setTestPath(SHOPIFY_TEST_PATH)
    if (!after && testPath === SHOPIFY_TEST_PATH) setTestPath('')
    if (after && !graphqlPath) setGraphqlPath(SHOPIFY_GRAPHQL_PATH)
    if (!after && graphqlPath === SHOPIFY_GRAPHQL_PATH) setGraphqlPath('')
  }

  function changeUrl(next: string) {
    setUrl(next)
    if (oauth) applyPreset(shopifyTokenUrl(url), shopifyTokenUrl(next))
  }

  function changeAuthType(next: 'header' | 'oauth') {
    setAuthType(next)
    applyPreset(next === 'oauth' ? null : shopifyTokenUrl(url), next === 'oauth' ? shopifyTokenUrl(url) : null)
  }

  async function submit(event: FormEvent) {
    event.preventDefault()
    if (!name.trim() || !url.trim() || roles.length === 0 || oauthIncomplete) return
    const created = await create.mutateAsync({
      kind,
      name: name.trim(),
      url: url.trim(),
      allowed_roles: roles,
      ...(kind === 'http' && graphqlPath.trim() ? { graphql_read_path: graphqlPath.trim() } : {}),
      ...(oauth
        ? { auth_header: header.trim() || 'Authorization', oauth_token_url: tokenUrl.trim(), oauth_client_id: clientId.trim(), oauth_client_secret: clientSecret.trim(), ...(testPath.trim() ? { test_path: testPath.trim() } : {}) }
        : secret ? { auth_header: header.trim() || 'Authorization', secret } : {}),
    })
    try {
      // An MCP server's tools are known only after a test.
      if (created.kind === 'mcp') {
        const result = await test.mutateAsync(created.id)
        if (result.ok) toast(result.detail)
        else toast.error(result.detail)
      }
    } finally {
      onClose()
    }
  }

  return <form className="p-4" onSubmit={(event) => void submit(event)}>
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
        <Input id="agent-tool-url" value={url} maxLength={2048} onChange={(event) => changeUrl(event.target.value)} placeholder={kind === 'mcp' ? 'https://mcp.example.com/mcp' : 'https://api.example.com/v1'} />
        <FieldDescription>{kind === 'mcp' ? 'The Streamable HTTP endpoint.' : 'Every request of the agent stays below this URL.'} HTTPS only; local and private addresses are refused.</FieldDescription>
      </Field>
      {kind === 'http' ? <Field className="min-[900px]:col-span-2">
        <FieldLabel htmlFor="agent-tool-auth">Credential</FieldLabel>
        <Select items={AUTH_TYPES} value={authType} onValueChange={(value) => changeAuthType(value as 'header' | 'oauth')}>
          <SelectTrigger id="agent-tool-auth" aria-label="Credential"><SelectValue /></SelectTrigger>
          <SelectContent>{AUTH_TYPES.map((option) => <SelectItem key={option.value} value={option.value}>{option.label}</SelectItem>)}</SelectContent>
        </Select>
      </Field> : null}
      {oauth ? <>
        <Field className="min-[900px]:col-span-2">
          <FieldLabel htmlFor="agent-tool-token-url">Token URL <RequiredMark /></FieldLabel>
          <Input id="agent-tool-token-url" value={tokenUrl} maxLength={2048} onChange={(event) => setTokenUrl(event.target.value)} placeholder="https://api.example.com/oauth/token" />
          <FieldDescription>Orbit gets and renews the access token here. It can be on another host than the base URL.</FieldDescription>
        </Field>
        <Field><FieldLabel htmlFor="agent-tool-client-id">Client ID <RequiredMark /></FieldLabel><Input id="agent-tool-client-id" value={clientId} autoComplete="off" disabled={!canKeepSecrets} onChange={(event) => setClientId(event.target.value)} /></Field>
        <Field>
          <FieldLabel htmlFor="agent-tool-client-secret">Client secret <RequiredMark /></FieldLabel>
          <PasswordInput id="agent-tool-client-secret" value={clientSecret} autoComplete="off" disabled={!canKeepSecrets} onChange={(event) => setClientSecret(event.target.value)} />
          <FieldDescription>{canKeepSecrets ? 'Orbit never shows the client ID or secret again.' : 'This server has no ORBIT_APP_KEY, so it cannot keep secrets.'}</FieldDescription>
        </Field>
      </> : <Field className="min-[900px]:col-span-2">
        <FieldLabel htmlFor="agent-tool-secret">Secret</FieldLabel>
        <PasswordInput id="agent-tool-secret" value={secret} autoComplete="off" disabled={!canKeepSecrets} onChange={(event) => setSecret(event.target.value)} placeholder="Bearer …" />
        <FieldDescription>{canKeepSecrets ? 'The full header value. Shared by everyone who may use this tool. Orbit never shows it again.' : 'This server has no ORBIT_APP_KEY, so it cannot keep secrets.'}</FieldDescription>
      </Field>}
      <Advanced className="min-[900px]:col-span-2" fields={oauth ? 'Token header, test path, GraphQL path' : kind === 'http' ? 'Credential header, GraphQL path' : 'Credential header'}>
        <FieldGrid>
          {oauth ? <>
            <Field><FieldLabel htmlFor="agent-tool-header">Token header</FieldLabel><Input id="agent-tool-header" value={header} maxLength={100} onChange={(event) => setHeader(event.target.value)} /><FieldDescription>In Authorization, the value is “Bearer &lt;token&gt;”. In other headers, the bare token.</FieldDescription></Field>
            <Field><FieldLabel htmlFor="agent-tool-test-path">Test path</FieldLabel><Input id="agent-tool-test-path" value={testPath} maxLength={2048} onChange={(event) => setTestPath(event.target.value)} placeholder="me" /><FieldDescription>Below the base URL. Test does a GET of it. Empty: Test checks only the token.</FieldDescription></Field>
          </> : <Field><FieldLabel htmlFor="agent-tool-header">Credential header</FieldLabel><Input id="agent-tool-header" value={header} maxLength={100} onChange={(event) => setHeader(event.target.value)} /></Field>}
          {kind === 'http' ? <Field className="min-[900px]:col-span-2"><FieldLabel htmlFor="agent-tool-graphql-path">GraphQL path</FieldLabel><Input id="agent-tool-graphql-path" value={graphqlPath} maxLength={2048} onChange={(event) => setGraphqlPath(event.target.value)} placeholder="/graphql" /><FieldDescription>Optional. After you add the tool, a switch can let GraphQL reads at this exact path run without approval. It is off until you turn it on.</FieldDescription></Field> : null}
        </FieldGrid>
      </Advanced>
      <RolesField value={roles} onChange={setRoles} />
      <div className="flex items-end justify-start gap-2 min-[900px]:justify-end">
        <Button variant="ghost" type="button" onClick={onClose}>Cancel</Button>
        <Button type="submit" disabled={create.isPending || test.isPending || !name.trim() || !url.trim() || roles.length === 0 || oauthIncomplete}>Add</Button>
      </div>
      {create.isError ? <p className="text-destructive min-[900px]:col-span-2" role="alert">{message(create.error, 'The tool could not be added.')}</p> : null}
    </FieldGrid>
  </form>
}

function RolesField({ value, onChange }: { value: AgentIntegrationRole[]; onChange: (roles: AgentIntegrationRole[]) => void }) {
  return <FieldSet className="flex-row flex-wrap gap-[18px]">
    <FieldLegend variant="label">Who may use it</FieldLegend>
    {ROLES.map((role) => <Label key={role} className="font-normal capitalize"><Checkbox checked={value.includes(role)} onCheckedChange={(on) => onChange(toggled(value, role, on))} /> {role}s</Label>)}
  </FieldSet>
}

/** The list's status line: what the agent can do with this tool, and its credential. */
function integrationStatus(integration: AgentIntegration) {
  const reach = integration.kind === 'http'
    ? integration.allowed_methods.join(', ')
    : integration.tools ? `${integration.tools.filter((tool) => tool.enabled).length} of ${integration.tools.length} tools on` : 'Not tested'
  const credential = integration.auth_type === 'oauth' ? 'OAuth client' : integration.secret_set ? 'Credential set' : 'No credential'
  return `${reach} · ${credential}`
}

function IntegrationRow({ integration }: { integration: AgentIntegration }) {
  const { workspace } = useWorkspace()
  const test = useTestAgentIntegration(workspace.id)
  const remove = useDeleteAgentIntegration(workspace.id)
  const [open, setOpen] = useState(false)

  async function runTest() {
    const result = await test.mutateAsync(integration.id)
    if (result.ok) toast(result.detail)
    else toast.error(result.detail)
  }

  return <div data-slot="integration-row">
    <SettingsRow>
      <RowIcon>{integration.kind === 'mcp' ? <Category /> : <Code />}</RowIcon>
      <div className="flex min-w-48 flex-1 flex-col gap-0.5">
        <span className="flex flex-wrap items-center gap-2 text-[13px] font-medium text-foreground">{integration.name}<Badge variant="secondary">{integration.kind === 'mcp' ? 'MCP server' : 'HTTP API'}</Badge></span>
        <code className="truncate text-xs text-muted-foreground">{integration.url}</code>
        <span className="text-xs text-muted-foreground/70">{integrationStatus(integration)}</span>
      </div>
      <div className="flex shrink-0 items-center gap-1">
        <Button variant="outline" type="button" disabled={test.isPending} onClick={() => void runTest()}>{test.isPending ? 'Testing…' : 'Test'}</Button>
        <Button variant="ghost" type="button" aria-label={`${open ? 'Close' : 'Edit'} ${integration.name}`} aria-expanded={open} onClick={() => setOpen(!open)}>{open ? 'Close' : 'Edit'}</Button>
        <Button variant="ghost" size="icon" className="text-muted-foreground" type="button" aria-label={`Remove ${integration.name}`} disabled={remove.isPending} onClick={async () => {
          if (await confirmAction({ title: `Remove ${integration.name}?`, description: 'The agent loses these tools at once. The saved secret is deleted.', confirmLabel: 'Remove', danger: true })) remove.mutate(integration.id)
        }}><Trash /></Button>
      </div>
    </SettingsRow>
    {open ? <IntegrationDetails integration={integration} /> : null}
  </div>
}

function IntegrationDetails({ integration }: { integration: AgentIntegration }) {
  const { workspace } = useWorkspace()
  const update = useUpdateAgentIntegration(workspace.id)
  const [secret, setSecret] = useState('')
  const [clientId, setClientId] = useState('')
  const [clientSecret, setClientSecret] = useState('')
  const [testPath, setTestPath] = useState(integration.test_path ?? '')
  const [graphqlPath, setGraphqlPath] = useState(integration.graphql_read_path ?? '')
  const oauth = integration.auth_type === 'oauth'
  const enabled = integration.tools?.filter((tool) => tool.enabled).map((tool) => tool.name) ?? []

  const save = (input: Parameters<typeof update.mutate>[0]['input'], done?: string) =>
    update.mutate({ id: integration.id, input }, {
      onSuccess: () => { if (done) toast(done) },
      onError: (error) => toast.error(message(error, 'The change could not be saved.')),
    })

  return <div className="flex flex-col divide-y border-t bg-muted/20">
    <SettingsRow>
      <div className="flex min-w-0 flex-1 flex-col gap-0.5 text-xs text-muted-foreground">
        <span>Tools named <code>{integration.slug}__…</code></span>
        {oauth ? <code className="[overflow-wrap:anywhere]">Token URL: {integration.oauth_token_url}</code> : null}
        <span>{oauth ? `OAuth client credentials, token in ${integration.auth_header}` : integration.secret_set ? `Credential set in ${integration.auth_header}` : 'No credential'} · changed {relativeTime(new Date(integration.updated_at).toISOString())}</span>
      </div>
    </SettingsRow>
    <SettingsRow>
      <RolesField value={integration.allowed_roles} onChange={(roles) => { if (roles.length) save({ allowed_roles: roles }) }} />
    </SettingsRow>
    {oauth ? <SettingsRow>
      <form className="flex w-full flex-wrap items-end gap-2" onSubmit={(event) => { event.preventDefault(); if (clientId.trim() && clientSecret.trim()) save({ oauth_client_id: clientId.trim(), oauth_client_secret: clientSecret.trim() }, 'Client replaced'); setClientId(''); setClientSecret('') }}>
        <Field className="min-w-48 flex-1">
          <FieldLabel htmlFor={`agent-tool-client-id-${integration.id}`}>Replace client ID</FieldLabel>
          <Input id={`agent-tool-client-id-${integration.id}`} value={clientId} autoComplete="off" onChange={(event) => setClientId(event.target.value)} placeholder="Saved; type to replace" />
        </Field>
        <Field className="min-w-48 flex-1">
          <FieldLabel htmlFor={`agent-tool-client-secret-${integration.id}`}>Replace client secret</FieldLabel>
          <PasswordInput id={`agent-tool-client-secret-${integration.id}`} value={clientSecret} autoComplete="off" onChange={(event) => setClientSecret(event.target.value)} placeholder="Saved; type to replace" />
        </Field>
        <Button variant="outline" type="submit" disabled={!clientId.trim() || !clientSecret.trim() || update.isPending}>Save client</Button>
      </form>
    </SettingsRow> : <SettingsRow>
      <form className="flex w-full flex-wrap items-end gap-2" onSubmit={(event) => { event.preventDefault(); if (secret) save({ secret, auth_header: integration.auth_header ?? 'Authorization' }, 'Secret replaced'); setSecret('') }}>
        <Field className="min-w-60 flex-1">
          <FieldLabel htmlFor={`agent-tool-secret-${integration.id}`}>Replace secret</FieldLabel>
          <PasswordInput id={`agent-tool-secret-${integration.id}`} value={secret} autoComplete="off" onChange={(event) => setSecret(event.target.value)} placeholder={integration.secret_set ? 'Saved; type to replace' : 'Bearer …'} />
        </Field>
        <Button variant="outline" type="submit" disabled={!secret || update.isPending}>Save secret</Button>
        {integration.secret_set ? <Button variant="ghost" type="button" disabled={update.isPending} onClick={() => save({ secret: '' }, 'Secret removed')}>Remove secret</Button> : null}
      </form>
    </SettingsRow>}
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
    {integration.kind === 'http' ? <SettingsRow className="flex-nowrap">
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="text-[13px] font-medium text-foreground">Run GraphQL reads without approval</span>
        <span className="text-xs text-muted-foreground/70">Only one query at exactly the GraphQL path. Mutations and other POST paths still need a yes. Reads can return customer data to the agent and the thread.{integration.graphql_reads_enabled && !integration.allowed_methods.includes('POST') ? ' Allow POST above, or the agent cannot send them.' : ''}{integration.graphql_read_path ? '' : ' Set a GraphQL path under Advanced first.'}</span>
      </div>
      <Switch aria-label="Run GraphQL reads without approval" checked={integration.graphql_reads_enabled} disabled={update.isPending || !integration.graphql_read_path} onCheckedChange={(on) => save({ graphql_reads_enabled: on })} />
    </SettingsRow> : null}
    {integration.kind === 'http' ? <SettingsRow>
      <Advanced className="w-full" fields="Test path, GraphQL path">
        <div className="flex flex-col gap-3">
          <form className="flex w-full flex-wrap items-end gap-2" onSubmit={(event) => { event.preventDefault(); save({ test_path: testPath.trim() }, 'Test path saved') }}>
            <Field className="min-w-60 flex-1">
              <FieldLabel htmlFor={`agent-tool-test-path-${integration.id}`}>Test path</FieldLabel>
              <Input id={`agent-tool-test-path-${integration.id}`} value={testPath} maxLength={2048} onChange={(event) => setTestPath(event.target.value)} placeholder={oauth ? 'Empty: Test checks only the token' : 'Empty: Test does a GET of the base URL'} />
            </Field>
            <Button variant="outline" type="submit" disabled={testPath.trim() === (integration.test_path ?? '') || update.isPending}>Save test path</Button>
          </form>
          <form className="flex w-full flex-wrap items-end gap-2" onSubmit={(event) => { event.preventDefault(); save({ graphql_read_path: graphqlPath.trim() }, 'GraphQL path saved') }}>
            <Field className="min-w-60 flex-1">
              <FieldLabel htmlFor={`agent-tool-graphql-path-${integration.id}`}>GraphQL path</FieldLabel>
              <Input id={`agent-tool-graphql-path-${integration.id}`} value={graphqlPath} maxLength={2048} onChange={(event) => setGraphqlPath(event.target.value)} placeholder="/graphql" />
            </Field>
            <Button variant="outline" type="submit" disabled={graphqlPath.trim() === (integration.graphql_read_path ?? '') || update.isPending}>Save GraphQL path</Button>
          </form>
        </div>
      </Advanced>
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
}
