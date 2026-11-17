import { expect, test } from 'bun:test'

import { appAddress, appIconName } from './apps'

test('an app opens only at an http(s) address', () => {
  expect(appAddress({ url: 'https://shop.example.com/admin' })).toBe('https://shop.example.com/admin')
  expect(appAddress({ url: 'javascript:alert(1)' })).toBeNull()
  expect(appAddress({ url: 'not a url' })).toBeNull()
})

test('a web icon the phone lacks falls back to the generic one', () => {
  expect(appIconName('chart')).toBe('chart')
  expect(appIconName('instagram')).toBe('category')
})
