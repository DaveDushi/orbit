import { expect, test } from 'bun:test'
import { oauthLoginDestination } from './oauth'

test('OAuth login returns only to a valid local consent request', () => {
  const request = 'a'.repeat(43)
  expect(oauthLoginDestination(`/oauth/consent?request=${request}`)).toBe(`/oauth/consent?request=${request}`)
  for (const input of [null, '//evil.test', 'https://evil.test/oauth/consent', '/oauth/consent?request=bad', `/oauth/consent?request=${request}&next=https://evil.test`, '/oauth/consent?request=%5C%5Cevil.test']) {
    const result = oauthLoginDestination(input)
    expect(result === '/' || result === `/oauth/consent?request=${request}`).toBe(true)
  }
})
