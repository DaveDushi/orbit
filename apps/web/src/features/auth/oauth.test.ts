import { expect, test } from 'bun:test'
import { appSignInDestination, oauthLoginDestination } from './oauth'

test('OAuth login returns only to a valid local consent request', () => {
  const request = 'a'.repeat(43)
  expect(oauthLoginDestination(`/oauth/consent?request=${request}`)).toBe(`/oauth/consent?request=${request}`)
  for (const input of [null, '//evil.test', 'https://evil.test/oauth/consent', '/oauth/consent?request=bad', `/oauth/consent?request=${request}&next=https://evil.test`, '/oauth/consent?request=%5C%5Cevil.test']) {
    const result = oauthLoginDestination(input)
    expect(result === '/' || result === `/oauth/consent?request=${request}`).toBe(true)
  }
})

test('A login returns to the authorize route only for the client of an app', () => {
  const good = '/oauth/authorize?client_id=orbit-app-1&redirect_uri=https%3A%2F%2Fapp.test%2Fcb&scope=openid'
  expect(appSignInDestination(good)).toBe(good)
  for (const input of [null, '//evil.test/oauth/authorize?client_id=orbit-app-1', 'https://evil.test/oauth/authorize?client_id=orbit-app-1', '/oauth/authorize?client_id=orbit-client-1', '/oauth/authorize/../../evil?client_id=orbit-app-1', '/oauth/authorize?']) {
    expect(appSignInDestination(input)).toBeNull()
  }
})
