/**
 * The web's agent tools page (`AgentToolsPage`) without React: the add form's body, the list's status lines and the
 * sign-in status, so Bun tests check them.
 */
import type { AgentIntegration, AgentIntegrationConnection, AgentIntegrationInput, AgentIntegrationKind, AgentIntegrationMethod, AgentIntegrationRole } from '@orbit/api/agentIntegrations'

import { fullDate } from './format'

export const ROLES: readonly AgentIntegrationRole[] = ['owner', 'admin', 'member']
export const METHODS: readonly AgentIntegrationMethod[] = ['GET', 'HEAD', 'POST', 'PUT', 'PATCH', 'DELETE']

export const KINDS: readonly { value: AgentIntegrationKind; label: string }[] = [
  { value: 'mcp', label: 'Remote MCP server' },
  { value: 'http', label: 'HTTP API' },
]

/** MCP: a fixed header or a shared OAuth sign-in (Connect). HTTP: a fixed header or OAuth client credentials. */
export type ToolCredential = 'header' | 'connect' | 'oauth'

export const credentialChoices = (kind: AgentIntegrationKind): { value: ToolCredential; label: string }[] =>
  kind === 'mcp'
    ? [
        { value: 'header', label: 'Fixed header credential' },
        { value: 'connect', label: 'Sign in with OAuth (Connect)' },
      ]
    : [
        { value: 'header', label: 'Fixed header credential' },
        { value: 'oauth', label: 'OAuth client credentials' },
      ]

export interface ToolForm {
  kind: AgentIntegrationKind
  credential: ToolCredential
  name: string
  url: string
  header: string
  secret: string
  tokenUrl: string
  clientId: string
  clientSecret: string
  testPath: string
  graphqlPath: string
  roles: AgentIntegrationRole[]
}

export const EMPTY_TOOL: ToolForm = { kind: 'mcp', credential: 'header', name: '', url: '', header: 'Authorization', secret: '', tokenUrl: '', clientId: '', clientSecret: '', testPath: '', graphqlPath: '', roles: ['owner', 'admin', 'member'] }

/** The credential that fits the kind: a kind change keeps `header`, and drops the other kind's choice. */
export const credentialFor = (kind: AgentIntegrationKind, credential: ToolCredential): ToolCredential => (credentialChoices(kind).some((choice) => choice.value === credential) ? credential : 'header')

const oauthOf = (form: ToolForm) => form.kind === 'http' && form.credential === 'oauth'

/** Name, URL and a role are needed; OAuth client credentials need all three fields; Connect needs an app key. */
export function toolFormReady(form: ToolForm, canKeepSecrets: boolean): boolean {
  if (!form.name.trim() || !form.url.trim() || form.roles.length === 0) return false
  if (oauthOf(form) && (!form.tokenUrl.trim() || !form.clientId.trim() || !form.clientSecret.trim())) return false
  return !(form.kind === 'mcp' && form.credential === 'connect' && !canKeepSecrets)
}

/** The web's create body: only the fields of the chosen credential, and a secret only when one was typed. */
export function toolInput(form: ToolForm): AgentIntegrationInput {
  const header = form.header.trim() || 'Authorization'
  const connect = form.kind === 'mcp' && form.credential === 'connect'
  return {
    kind: form.kind,
    name: form.name.trim(),
    url: form.url.trim(),
    allowed_roles: form.roles,
    ...(form.kind === 'http' && form.graphqlPath.trim() ? { graphql_read_path: form.graphqlPath.trim() } : {}),
    ...(oauthOf(form)
      ? { auth_header: header, oauth_token_url: form.tokenUrl.trim(), oauth_client_id: form.clientId.trim(), oauth_client_secret: form.clientSecret.trim(), ...(form.testPath.trim() ? { test_path: form.testPath.trim() } : {}) }
      : form.secret && !connect
        ? { auth_header: header, secret: form.secret }
        : {}),
  }
}

/** The list's status line: what the agent can do with this tool, and its credential. */
export function integrationStatus(integration: Pick<AgentIntegration, 'kind' | 'allowed_methods' | 'tools' | 'auth_type' | 'connection' | 'secret_set'>): string {
  const reach =
    integration.kind === 'http'
      ? integration.allowed_methods.join(', ')
      : integration.tools
        ? `${integration.tools.filter((tool) => tool.enabled).length} of ${integration.tools.length} tools on`
        : 'Not tested'
  const credential = integration.auth_type === 'oauth' ? 'OAuth client' : integration.connection ? 'OAuth sign-in' : integration.secret_set ? 'Credential set' : 'No credential'
  return `${reach} · ${credential}`
}

/** A fixed credential replaces the sign-in, so only MCP servers without one offer Connect. */
export const canConnect = (integration: Pick<AgentIntegration, 'kind' | 'auth_type'>) => integration.kind === 'mcp' && integration.auth_type !== 'header'

/** Admins see a warning this long before a sign-in ends. Matches the server's notice. */
const EXPIRY_WARNING_MS = 14 * 24 * 60 * 60 * 1000

/** What the list says about an MCP server's sign-in, and whether admins must act soon (the web's `connectionStatus`). */
export function connectionStatus(connection: AgentIntegrationConnection, now: number): { text: string; warning: boolean } {
  const who = connection.account ? `Connected as ${connection.account}` : 'Connected'
  if (connection.needs_reconnect) return { text: 'Sign-in stopped working. Reconnect it.', warning: true }
  const end = connection.grant_expires_at
  if (end === null) return { text: who, warning: false }
  const until = fullDate(new Date(end).toISOString())
  if (end <= now) return { text: `Sign-in ended on ${until}. Reconnect it.`, warning: true }
  if (end - now <= EXPIRY_WARNING_MS) {
    const days = Math.ceil((end - now) / (24 * 60 * 60 * 1000))
    return { text: `${who} · sign-in ends in ${days} ${days === 1 ? 'day' : 'days'} (${until}). Reconnect before then.`, warning: true }
  }
  return { text: `${who} · until ${until}`, warning: false }
}

/** A role or method list after one box changes; the last one cannot be removed (`null`: keep the list). */
export function toggledList<T>(list: readonly T[], value: T, on: boolean): T[] | null {
  const next = on ? [...list.filter((item) => item !== value), value] : list.filter((item) => item !== value)
  return next.length ? next : null
}
