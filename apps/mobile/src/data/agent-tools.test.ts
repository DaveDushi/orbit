import { expect, test } from 'bun:test'

import { canConnect, connectionStatus, credentialFor, EMPTY_TOOL, integrationStatus, toggledList, type ToolForm, toolFormReady, toolInput } from './agent-tools'

const form = (fields: Partial<ToolForm>): ToolForm => ({ ...EMPTY_TOOL, name: 'Cloudflare', url: 'https://mcp.example.com/mcp', ...fields })
const DAY = 24 * 60 * 60 * 1000

test('a tool needs a name, a URL and a role; OAuth needs its three fields; Connect needs an app key', () => {
  expect(toolFormReady(form({}), true)).toBe(true)
  expect(toolFormReady(form({ name: ' ' }), true)).toBe(false)
  expect(toolFormReady(form({ roles: [] }), true)).toBe(false)
  expect(toolFormReady(form({ kind: 'http', credential: 'oauth', tokenUrl: 'https://x/token', clientId: 'id' }), true)).toBe(false)
  expect(toolFormReady(form({ kind: 'http', credential: 'oauth', tokenUrl: 'https://x/token', clientId: 'id', clientSecret: 's' }), true)).toBe(true)
  expect(toolFormReady(form({ credential: 'connect' }), false)).toBe(false)
  expect(toolFormReady(form({ credential: 'connect' }), true)).toBe(true)
})

test('the body has only the chosen credential', () => {
  expect(toolInput(form({ secret: 'Bearer x', header: ' ' }))).toEqual({ kind: 'mcp', name: 'Cloudflare', url: 'https://mcp.example.com/mcp', allowed_roles: ['owner', 'admin', 'member'], auth_header: 'Authorization', secret: 'Bearer x' })
  expect(toolInput(form({ credential: 'connect', secret: 'Bearer x' }))).not.toHaveProperty('secret')
  expect(toolInput(form({}))).not.toHaveProperty('auth_header')
  const oauth = toolInput(form({ kind: 'http', credential: 'oauth', tokenUrl: ' https://x/token ', clientId: 'id', clientSecret: 's', secret: 'ignored', testPath: 'me', graphqlPath: '/graphql' }))
  expect(oauth).toMatchObject({ oauth_token_url: 'https://x/token', oauth_client_id: 'id', oauth_client_secret: 's', test_path: 'me', graphql_read_path: '/graphql' })
  expect(oauth).not.toHaveProperty('secret')
  // a GraphQL path belongs to HTTP APIs only
  expect(toolInput(form({ graphqlPath: '/graphql' }))).not.toHaveProperty('graphql_read_path')
})

test('a kind change keeps only a credential that kind has', () => {
  expect(credentialFor('http', 'connect')).toBe('header')
  expect(credentialFor('mcp', 'oauth')).toBe('header')
  expect(credentialFor('http', 'oauth')).toBe('oauth')
})

test('status line and Connect', () => {
  const base = { kind: 'mcp' as const, allowed_methods: [], tools: null, auth_type: 'none' as const, connection: null, secret_set: false }
  expect(integrationStatus(base)).toBe('Not tested · No credential')
  expect(integrationStatus({ ...base, tools: [{ name: 'a', title: null, description: '', read_only: true, enabled: true }, { name: 'b', title: null, description: '', read_only: false, enabled: false }], secret_set: true })).toBe('1 of 2 tools on · Credential set')
  expect(integrationStatus({ ...base, kind: 'http', allowed_methods: ['GET', 'POST'], auth_type: 'oauth' })).toBe('GET, POST · OAuth client')
  expect(canConnect({ kind: 'mcp', auth_type: 'none' })).toBe(true)
  expect(canConnect({ kind: 'mcp', auth_type: 'header' })).toBe(false)
  expect(canConnect({ kind: 'http', auth_type: 'none' })).toBe(false)
})

test('connectionStatus warns two weeks before the sign-in ends', () => {
  const now = Date.parse('2026-11-17T12:00:00Z')
  const connection = { account: 'ana', connected_at: now, grant_expires_at: null, needs_reconnect: false }
  expect(connectionStatus(connection, now)).toEqual({ text: 'Connected as ana', warning: false })
  expect(connectionStatus({ ...connection, needs_reconnect: true }, now).warning).toBe(true)
  expect(connectionStatus({ ...connection, grant_expires_at: now - 1 }, now).warning).toBe(true)
  expect(connectionStatus({ ...connection, grant_expires_at: now + DAY }, now).text).toContain('ends in 1 day')
  expect(connectionStatus({ ...connection, grant_expires_at: now + 30 * DAY }, now).warning).toBe(false)
})

test('toggledList never empties the list', () => {
  expect(toggledList(['owner'], 'owner', false)).toBeNull()
  expect(toggledList(['owner'], 'admin', true)).toEqual(['owner', 'admin'])
  expect(toggledList(['GET', 'POST'], 'POST', false)).toEqual(['GET'])
})
