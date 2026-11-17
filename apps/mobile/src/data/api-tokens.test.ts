import { expect, test } from 'bun:test'

import { accessLevel, isExpired, levelAvailable, serviceAccountScopes, type TokenForm, tokenBody, tokenFormReady, tokenResources, withAccessLevel } from './api-tokens'

const [tasks, docs, chat] = tokenResources(false)
const form = (fields: Partial<TokenForm>): TokenForm => ({ name: 'Bot', scopes: ['tasks:read'], projectIds: [], allProjects: false, serviceAccount: false, expiration: 'never', ...fields })

test('access levels follow the scopes, and write includes read', () => {
  expect(accessLevel(tasks, ['tasks:read'])).toBe('read')
  expect(accessLevel(tasks, ['tasks:read', 'tasks:write'])).toBe('write')
  expect(accessLevel(docs, ['tasks:read'])).toBe('none')
  expect(withAccessLevel(docs, ['tasks:read'], 'write')).toEqual(['tasks:read', 'docs:read', 'docs:write'])
  expect(withAccessLevel(tasks, ['tasks:read', 'tasks:write', 'chat:read'], 'none')).toEqual(['chat:read'])
})

test('chat has no write level, and a service account gets task access only', () => {
  expect(levelAvailable(chat, 'write')).toBe(false)
  expect(levelAvailable(chat, 'read')).toBe(true)
  const [, serviceDocs] = tokenResources(true)
  expect(levelAvailable(serviceDocs, 'read')).toBe(false)
  expect(levelAvailable(serviceDocs, 'none')).toBe(true)
  expect(serviceAccountScopes(['docs:read', 'tasks:write', 'tasks:read'])).toEqual(['tasks:write', 'tasks:read'])
  expect(serviceAccountScopes(['docs:read'])).toEqual(['tasks:read'])
})

test('a token with task access needs projects; one without needs none', () => {
  expect(tokenFormReady(form({}))).toBe(false)
  expect(tokenFormReady(form({ projectIds: ['p1'] }))).toBe(true)
  expect(tokenFormReady(form({ allProjects: true }))).toBe(true)
  expect(tokenFormReady(form({ scopes: ['docs:read'] }))).toBe(true)
  expect(tokenFormReady(form({ name: '  ', allProjects: true }))).toBe(false)
  expect(tokenFormReady(form({ scopes: [] }))).toBe(false)
})

test('the body drops the projects without task access and turns the expiration into days', () => {
  expect(tokenBody(form({ name: ' Bot ', projectIds: ['p1'], expiration: '30' }))).toEqual({ name: 'Bot', project_ids: ['p1'], all_projects: false, scopes: ['tasks:read'], service_account: false, expires_in_days: 30 })
  expect(tokenBody(form({ scopes: ['docs:read'], projectIds: ['p1'], allProjects: true }))).toMatchObject({ project_ids: [], all_projects: false, expires_in_days: null })
  expect(tokenBody(form({ projectIds: ['p1'], allProjects: true }))).toMatchObject({ project_ids: [], all_projects: true })
})

test('isExpired', () => {
  expect(isExpired(null)).toBe(false)
  expect(isExpired('2026-01-01T00:00:00Z', Date.parse('2026-01-02T00:00:00Z'))).toBe(true)
  expect(isExpired('2026-01-03T00:00:00Z', Date.parse('2026-01-02T00:00:00Z'))).toBe(false)
})
