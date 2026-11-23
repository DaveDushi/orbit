import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import * as WebBrowser from 'expo-web-browser'
import {
  type AgentIntegration,
  type AgentIntegrationInput,
  createAgentIntegration,
  deleteAgentIntegration,
  disconnectAgentIntegration,
  readAgentIntegrations,
  testAgentIntegration,
  toolMatches,
  updateAgentIntegration,
  withToolsSet,
} from '@orbit/api/agentIntegrations'
import { useState } from 'react'
import { View } from 'react-native'

import { CheckRow, SelectField } from '../settings/Fields'
import { ConfirmDialog, type ConfirmRequest } from '../settings/Dialog'
import { Badge, FieldLabel, Note, OptionHint, OptionText, RowIcon, SettingsCard, SettingsRow, Toggle } from '../settings/Shell'
import * as tools from '../../data/agent-tools'
import { relativeTime } from '../../data/format'
import { useOrbit } from '../../orbit-context'
import { errorMessage } from '../../session-context'
import { Button, Input, Text } from '../../ui'

const toolsKey = (workspaceId: string) => ['agent', 'integrations', workspaceId] as const

function useToolWrites() {
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  const onSettled = () => void queryClient.invalidateQueries({ queryKey: toolsKey(workspaceId) })
  return {
    create: useMutation({ mutationFn: (input: AgentIntegrationInput) => createAgentIntegration(client, workspaceId, input), onSettled }),
    update: useMutation({ mutationFn: ({ id, input }: { id: string; input: AgentIntegrationInput }) => updateAgentIntegration(client, workspaceId, id, input), onSettled }),
    remove: useMutation({ mutationFn: (id: string) => deleteAgentIntegration(client, workspaceId, id), onSettled }),
    test: useMutation({ mutationFn: (id: string) => testAgentIntegration(client, workspaceId, id), onSettled }),
    disconnect: useMutation({ mutationFn: (id: string) => disconnectAgentIntegration(client, workspaceId, id), onSettled }),
  }
}

/**
 * Agentic → Workspace → Connected services (the web's `AgentToolsPage`): MCP servers and HTTP APIs the team agent may
 * call. Owners and admins only (`integrations.manage`). The OAuth sign-in (Connect) must run in the browser session
 * that started it, so Connect opens the web page in an in-app browser; everything else is native.
 */
export function AgentToolsCard() {
  const { client, workspace, workspaceId } = useOrbit()
  const canManage = workspace.permissions.includes('integrations.manage')
  const list = useQuery({ queryKey: toolsKey(workspaceId), queryFn: () => readAgentIntegrations(client, workspaceId), enabled: canManage })
  const [adding, setAdding] = useState(false)
  if (!canManage) return null
  const items = list.data?.items
  return (
    <SettingsCard
      title="Connected services"
      description="The agent calls these services as the person who asked. Orbit keeps the credential and makes every call; changes outside Orbit need that person's yes in the thread."
      actions={adding ? null : <Button label="Add tool" variant="outline" size="sm" onPress={() => setAdding(true)} />}
      flush
    >
      {adding ? <AddTool canKeepSecrets={list.data?.can_keep_secrets !== false} onClose={() => setAdding(false)} /> : null}
      {list.isPending ? <View className="p-4"><Note loading>Loading agent tools…</Note></View> : null}
      {list.isError ? <View className="p-4"><Note error>Agent tools could not be loaded.</Note></View> : null}
      {items?.length === 0 && !adding ? <View className="p-4"><Note>No tools yet.</Note></View> : null}
      {items?.map((integration, index) => <ToolRow key={integration.id} integration={integration} first={index === 0 && !adding} />)}
    </SettingsCard>
  )
}

/** Opens the web's agent tools page, where Connect runs the OAuth sign-in; the list reloads when it closes. */
function useOpenConnect() {
  const { session, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  return async () => {
    await WebBrowser.openBrowserAsync(`${session.server}/settings/agentic/workspace?workspace=${encodeURIComponent(workspaceId)}`, { presentationStyle: WebBrowser.WebBrowserPresentationStyle.PAGE_SHEET })
    void queryClient.invalidateQueries({ queryKey: toolsKey(workspaceId) })
  }
}

function AddTool({ canKeepSecrets, onClose }: { canKeepSecrets: boolean; onClose: () => void }) {
  const { create, test } = useToolWrites()
  const openConnect = useOpenConnect()
  const [form, setForm] = useState<tools.ToolForm>(tools.EMPTY_TOOL)
  const [result, setResult] = useState<string | null>(null)
  const set = (fields: Partial<tools.ToolForm>) => setForm((current) => ({ ...current, ...fields }))
  const oauth = form.kind === 'http' && form.credential === 'oauth'
  const connect = form.kind === 'mcp' && form.credential === 'connect'
  const busy = create.isPending || test.isPending

  const submit = async () => {
    if (!tools.toolFormReady(form, canKeepSecrets) || busy) return
    const created = await create.mutateAsync(tools.toolInput(form)).catch(() => null)
    if (!created) return
    if (connect) {
      onClose()
      await openConnect()
    } else if (created.kind === 'mcp') {
      // an MCP server's tools are known only after a test
      const outcome = await test.mutateAsync(created.id).catch((error: unknown) => ({ ok: false, detail: errorMessage(error) }))
      if (outcome.ok) onClose()
      else setResult(`Added, but the test failed: ${outcome.detail}`)
    } else onClose()
  }

  if (result) {
    return (
      <View className="gap-3 p-4">
        <Note error>{result}</Note>
        <View className="flex-row"><Button label="Close" variant="outline" onPress={onClose} /></View>
      </View>
    )
  }
  return (
    <View className="gap-4 p-4">
      <SelectField label="Kind" value={form.kind} options={tools.KINDS} onChange={(value) => set({ kind: value as tools.ToolForm['kind'], credential: tools.credentialFor(value as tools.ToolForm['kind'], form.credential) })} />
      <Input label="Name" required maxLength={80} value={form.name} placeholder={form.kind === 'mcp' ? 'Cloudflare' : 'Shopify'} onChangeText={(name) => set({ name })} />
      <Input
        label={form.kind === 'mcp' ? 'Server URL' : 'Base URL'}
        required
        autoCapitalize="none"
        autoCorrect={false}
        keyboardType="url"
        maxLength={2048}
        value={form.url}
        placeholder={form.kind === 'mcp' ? 'https://mcp.example.com/mcp' : 'https://api.example.com/v1'}
        onChangeText={(url) => set({ url })}
        description={`${form.kind === 'mcp' ? 'The Streamable HTTP endpoint.' : 'Every request of the agent stays below this URL.'} HTTPS only; local and private addresses are refused.`}
      />
      <SelectField
        label="Credential"
        value={form.credential}
        options={tools.credentialChoices(form.kind)}
        onChange={(value) => set({ credential: value as tools.ToolCredential })}
        description={connect ? (canKeepSecrets ? 'After Add, sign in to the service in the web app and authorize Orbit. Everyone who may use this tool shares that one sign-in. Orbit keeps the tokens and renews them.' : 'This server has no ORBIT_APP_KEY, so it cannot keep a sign-in.') : undefined}
      />
      {oauth ? (
        <>
          <Input label="Token URL" required autoCapitalize="none" autoCorrect={false} keyboardType="url" maxLength={2048} value={form.tokenUrl} placeholder="https://api.example.com/oauth/token" onChangeText={(tokenUrl) => set({ tokenUrl })} description="Orbit gets and renews the access token here. It can be on another host than the base URL." />
          <Input label="Client ID" required autoCapitalize="none" autoCorrect={false} editable={canKeepSecrets} value={form.clientId} onChangeText={(clientId) => set({ clientId })} />
          <Input label="Client secret" required secureTextEntry autoCapitalize="none" editable={canKeepSecrets} value={form.clientSecret} onChangeText={(clientSecret) => set({ clientSecret })} description={canKeepSecrets ? 'Orbit never shows the client ID or secret again.' : 'This server has no ORBIT_APP_KEY, so it cannot keep secrets.'} />
          <Input label="Token header" autoCapitalize="none" autoCorrect={false} maxLength={100} value={form.header} onChangeText={(header) => set({ header })} description="In Authorization, the value is “Bearer <token>”. In other headers, the bare token." />
          <Input label="Test path" autoCapitalize="none" autoCorrect={false} maxLength={2048} value={form.testPath} placeholder="me" onChangeText={(testPath) => set({ testPath })} description="Below the base URL. Test does a GET of it. Empty: Test checks only the token." />
        </>
      ) : connect ? null : (
        <>
          <Input label="Secret" secureTextEntry autoCapitalize="none" editable={canKeepSecrets} value={form.secret} placeholder="Bearer …" onChangeText={(secret) => set({ secret })} description={canKeepSecrets ? 'The full header value. Shared by everyone who may use this tool. Orbit never shows it again.' : 'This server has no ORBIT_APP_KEY, so it cannot keep secrets.'} />
          <Input label="Credential header" autoCapitalize="none" autoCorrect={false} maxLength={100} value={form.header} onChangeText={(header) => set({ header })} />
        </>
      )}
      {form.kind === 'http' && !connect ? (
        <Input label="GraphQL path" autoCapitalize="none" autoCorrect={false} maxLength={2048} value={form.graphqlPath} placeholder="/graphql" onChangeText={(graphqlPath) => set({ graphqlPath })} description="Optional. After you add the tool, a switch can let GraphQL reads at this exact path run without approval. It is off until you turn it on." />
      ) : null}
      <RolesField value={form.roles} onChange={(roles) => set({ roles })} />
      {create.isError ? <Note error>{errorMessage(create.error)}</Note> : null}
      <View className="flex-row justify-end gap-2">
        <Button label="Cancel" variant="ghost" disabled={busy} onPress={onClose} />
        <Button label={connect ? 'Add and connect' : 'Add'} loading={busy} disabled={!tools.toolFormReady(form, canKeepSecrets)} onPress={() => void submit()} />
      </View>
    </View>
  )
}

function RolesField({ value, onChange }: { value: readonly tools.ToolForm['roles'][number][]; onChange: (roles: tools.ToolForm['roles']) => void }) {
  return (
    <View className="gap-1">
      <FieldLabel>Who may use it</FieldLabel>
      <View className="flex-row flex-wrap gap-x-5">
        {tools.ROLES.map((role) => (
          <CheckRow key={role} label={`${role.charAt(0).toUpperCase()}${role.slice(1)}s`} checked={value.includes(role)} onChange={(on) => onChange(on ? [...value.filter((item) => item !== role), role] : value.filter((item) => item !== role))} />
        ))}
      </View>
    </View>
  )
}

function ToolRow({ integration, first }: { integration: AgentIntegration; first: boolean }) {
  const { test, remove } = useToolWrites()
  const openConnect = useOpenConnect()
  const [open, setOpen] = useState(false)
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null)
  const [now] = useState(Date.now)
  const signIn = integration.connection ? tools.connectionStatus(integration.connection, now) : null
  const outcome = test.data ?? (test.isError ? { ok: false, detail: errorMessage(test.error) } : null)

  return (
    <View className={first ? '' : 'border-t border-border'}>
      <SettingsRow first>
        <RowIcon icon={integration.kind === 'mcp' ? 'category' : 'code'} />
        <View className="flex-1 min-w-0 gap-0.5">
          <View className="flex-row flex-wrap items-center gap-2">
            <Text numberOfLines={1} className="font-noto-medium text-[13px] shrink">{integration.name}</Text>
            <Badge label={integration.kind === 'mcp' ? 'MCP server' : 'HTTP API'} tone="secondary" />
          </View>
          <Text numberOfLines={1} className="font-mono text-[12px] text-muted-foreground">{integration.url}</Text>
          <Text className="text-[12px] text-muted-foreground/85">{tools.integrationStatus(integration)}</Text>
          {signIn ? <Text className={`text-[12px] ${signIn.warning ? 'font-noto-medium text-destructive' : 'text-muted-foreground'}`}>{signIn.text}</Text> : null}
          {outcome ? <Text className={`text-[12px] ${outcome.ok ? 'text-muted-foreground' : 'text-destructive'}`}>{outcome.detail}</Text> : null}
        </View>
      </SettingsRow>
      <View className="flex-row flex-wrap gap-1.5 px-4 pb-3">
        {tools.canConnect(integration) ? <Button label={integration.connection ? 'Reconnect' : 'Connect'} size="sm" variant={signIn?.warning || !integration.connection ? 'primary' : 'outline'} onPress={() => void openConnect()} /> : null}
        <Button label={test.isPending ? 'Testing…' : 'Test'} size="sm" variant="outline" disabled={test.isPending} onPress={() => test.mutate(integration.id)} />
        <Button label={open ? 'Close' : 'Edit'} size="sm" variant="ghost" onPress={() => setOpen(!open)} />
        <Button
          label="Remove"
          size="sm"
          variant="ghost"
          disabled={remove.isPending}
          onPress={() => setConfirm({ title: `Remove ${integration.name}?`, description: 'The agent loses these tools at once. The saved secret is deleted.', confirmLabel: 'Remove', danger: true, onConfirm: () => remove.mutate(integration.id) })}
        />
      </View>
      {open ? <ToolDetails integration={integration} /> : null}
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </View>
  )
}

function ToolDetails({ integration }: { integration: AgentIntegration }) {
  const { update, disconnect } = useToolWrites()
  const [secret, setSecret] = useState('')
  const [clientId, setClientId] = useState('')
  const [clientSecret, setClientSecret] = useState('')
  const [testPath, setTestPath] = useState(integration.test_path ?? '')
  const [graphqlPath, setGraphqlPath] = useState(integration.graphql_read_path ?? '')
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null)
  const oauth = integration.auth_type === 'oauth'
  const enabled = integration.tools?.filter((tool) => tool.enabled).map((tool) => tool.name) ?? []
  const [filter, setFilter] = useState('')
  const filtering = filter.trim() !== ''
  const shown = integration.tools?.filter((tool) => toolMatches(tool, filter)) ?? []
  const setShown = (on: boolean) => save({ enabled_tools: withToolsSet(integration.tools ?? [], shown.map((tool) => tool.name), on) })
  const save = (input: AgentIntegrationInput) => update.mutate({ id: integration.id, input })
  const credential = oauth
    ? `OAuth client credentials, token in ${integration.auth_header}`
    : integration.connection
      ? 'OAuth sign-in, shared by everyone who may use this tool'
      : integration.secret_set
        ? `Credential set in ${integration.auth_header}`
        : 'No credential'

  return (
    <View className="gap-4 border-t border-border bg-muted/20 p-4">
      <View className="gap-0.5">
        <Text className="text-[12px] text-muted-foreground">Tools named {integration.slug}__…</Text>
        {oauth ? <Text className="font-mono text-[12px] text-muted-foreground">Token URL: {integration.oauth_token_url}</Text> : null}
        <Text className="text-[12px] text-muted-foreground">{credential} · changed {relativeTime(new Date(integration.updated_at).toISOString())}</Text>
      </View>
      {integration.connection ? (
        <View className="flex-row">
          <Button
            label="Disconnect"
            variant="outline"
            size="sm"
            disabled={disconnect.isPending}
            onPress={() => setConfirm({ title: `Disconnect ${integration.name}?`, description: 'Orbit deletes the tokens and the agent cannot use these tools until an admin connects again. To end the grant at the service too, remove Orbit in your account there.', confirmLabel: 'Disconnect', danger: true, onConfirm: () => disconnect.mutate(integration.id) })}
          />
        </View>
      ) : null}
      <RolesField value={integration.allowed_roles} onChange={(roles) => roles.length && save({ allowed_roles: roles })} />
      {integration.connection ? null : oauth ? (
        <View className="gap-2">
          <Input label="Replace client ID" autoCapitalize="none" autoCorrect={false} value={clientId} placeholder="Saved; type to replace" onChangeText={setClientId} />
          <Input label="Replace client secret" secureTextEntry autoCapitalize="none" value={clientSecret} placeholder="Saved; type to replace" onChangeText={setClientSecret} />
          <View className="flex-row">
            <Button
              label="Save client"
              variant="outline"
              size="sm"
              disabled={!clientId.trim() || !clientSecret.trim() || update.isPending}
              onPress={() => {
                save({ oauth_client_id: clientId.trim(), oauth_client_secret: clientSecret.trim() })
                setClientId('')
                setClientSecret('')
              }}
            />
          </View>
        </View>
      ) : (
        <View className="gap-2">
          <Input label="Replace secret" secureTextEntry autoCapitalize="none" value={secret} placeholder={integration.secret_set ? 'Saved; type to replace' : 'Bearer …'} onChangeText={setSecret} />
          <View className="flex-row gap-2">
            <Button
              label="Save secret"
              variant="outline"
              size="sm"
              disabled={!secret || update.isPending}
              onPress={() => {
                save({ secret, auth_header: integration.auth_header ?? 'Authorization' })
                setSecret('')
              }}
            />
            {integration.secret_set ? <Button label="Remove secret" variant="ghost" size="sm" disabled={update.isPending} onPress={() => save({ secret: '' })} /> : null}
          </View>
        </View>
      )}
      {integration.kind === 'http' ? (
        <>
          <View className="gap-1">
            <FieldLabel>Allowed methods</FieldLabel>
            <View className="flex-row flex-wrap gap-x-5">
              {tools.METHODS.map((method) => (
                <CheckRow
                  key={method}
                  label={method}
                  mono
                  checked={integration.allowed_methods.includes(method)}
                  onChange={(on) => {
                    const methods = tools.toggledList(integration.allowed_methods, method, on)
                    if (methods) save({ allowed_methods: methods })
                  }}
                />
              ))}
            </View>
            <OptionHint>Methods other than GET and HEAD run only after the person who asked says yes in the thread.</OptionHint>
          </View>
          <View className="flex-row items-center gap-3">
            <OptionText title="Run GraphQL reads without approval">
              <OptionHint>
                Only one query at exactly the GraphQL path. Mutations and other POST paths still need a yes. Reads can return customer data to the agent and the thread.
                {integration.graphql_reads_enabled && !integration.allowed_methods.includes('POST') ? ' Allow POST above, or the agent cannot send them.' : ''}
                {integration.graphql_read_path ? '' : ' Set a GraphQL path below first.'}
              </OptionHint>
            </OptionText>
            <Toggle label="Run GraphQL reads without approval" value={integration.graphql_reads_enabled} disabled={update.isPending || !integration.graphql_read_path} onChange={(on) => save({ graphql_reads_enabled: on })} />
          </View>
          <View className="gap-2">
            <Input label="Test path" autoCapitalize="none" autoCorrect={false} maxLength={2048} value={testPath} placeholder={oauth ? 'Empty: Test checks only the token' : 'Empty: Test does a GET of the base URL'} onChangeText={setTestPath} />
            <View className="flex-row"><Button label="Save test path" variant="outline" size="sm" disabled={testPath.trim() === (integration.test_path ?? '') || update.isPending} onPress={() => save({ test_path: testPath.trim() })} /></View>
            <Input label="GraphQL path" autoCapitalize="none" autoCorrect={false} maxLength={2048} value={graphqlPath} placeholder="/graphql" onChangeText={setGraphqlPath} />
            <View className="flex-row"><Button label="Save GraphQL path" variant="outline" size="sm" disabled={graphqlPath.trim() === (integration.graphql_read_path ?? '') || update.isPending} onPress={() => save({ graphql_read_path: graphqlPath.trim() })} /></View>
          </View>
        </>
      ) : null}
      {integration.kind === 'mcp' && !integration.tools ? <Note>Test the server to see its tools.</Note> : null}
      {integration.tools?.length === 0 ? <Note>The server lists no tools.</Note> : null}
      {integration.tools?.length ? (
        <View className="gap-2">
          <Input value={filter} onChangeText={setFilter} placeholder="Filter tools" accessibilityLabel={`Filter the tools of ${integration.name}`} autoCapitalize="none" autoCorrect={false} clearButtonMode="while-editing" />
          <View className="flex-row flex-wrap items-center gap-1.5">
            <Text className="mr-auto text-[12px] text-muted-foreground">{enabled.length} of {integration.tools.length} on</Text>
            <Button label={filtering ? `Turn on ${shown.length} shown` : 'Turn all on'} size="sm" variant="outline" disabled={update.isPending || shown.every((tool) => tool.enabled)} onPress={() => setShown(true)} />
            <Button label={filtering ? `Turn off ${shown.length} shown` : 'Turn all off'} size="sm" variant="ghost" disabled={update.isPending || !shown.some((tool) => tool.enabled)} onPress={() => setShown(false)} />
          </View>
        </View>
      ) : null}
      {filtering && shown.length === 0 ? <Note>No tools match “{filter.trim()}”.</Note> : null}
      {shown.map((tool) => (
        <View key={tool.name} className="flex-row items-center gap-3">
          <View className="flex-1 min-w-0 gap-0.5">
            <View className="flex-row flex-wrap items-center gap-2">
              <Text className="font-mono text-[13px]">{tool.name}</Text>
              <Badge label={tool.read_only ? 'Read-only' : 'Needs approval'} tone={tool.read_only ? 'secondary' : 'outline'} />
            </View>
            {tool.description ? <Text numberOfLines={2} className="text-[12px] text-muted-foreground/85">{tool.description}</Text> : null}
          </View>
          <Toggle label={`Allow ${tool.name}`} value={tool.enabled} disabled={update.isPending} onChange={(on) => save({ enabled_tools: on ? [...enabled, tool.name] : enabled.filter((name) => name !== tool.name) })} />
        </View>
      ))}
      {update.isError ? <Note error>The change could not be saved. {errorMessage(update.error)}</Note> : null}
      {disconnect.isError ? <Note error>The sign-in could not be removed. {errorMessage(disconnect.error)}</Note> : null}
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </View>
  )
}
