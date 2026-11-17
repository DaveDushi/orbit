import { expect, test } from 'bun:test'

import { improvementContext } from './improvements'

test('the context passes the server check: no space in the revision, a whole-number viewport', () => {
  expect(improvementContext('/chat/c1', 390.4, 844, '1.0.0')).toEqual({ path: '/chat/c1', viewport: '390x844', revision: 'ios-app-1.0.0' })
  expect(improvementContext('tasks', 0, 844.6, undefined)).toEqual({ path: '/tasks', viewport: '1x845', revision: 'ios-app-development' })
  const odd = improvementContext('/a b', 390, 844, '1.0 beta+2')
  expect(odd.path).toBe('/ab')
  expect(odd.revision).toMatch(/^[\w.-]+$/)
})
