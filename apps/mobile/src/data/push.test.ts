import { describe, expect, test } from 'bun:test'

import { appPathOf } from './push'

describe('appPathOf', () => {
  test('a chat notification opens its conversation, or the thread of a reply', () => {
    expect(appPathOf('/chat/0193a')).toBe('/chat/0193a')
    expect(appPathOf('/chat/0193a?thread=0193b')).toBe('/chat/thread/0193b')
  })

  test('inbox and test notifications open their screens', () => {
    expect(appPathOf('/activity')).toBe('/activity')
    expect(appPathOf('/profile/notifications')).toBe('/settings/notifications')
  })

  test('anything else, or no url, opens Home', () => {
    expect(appPathOf('/tasks/1')).toBe('/')
    expect(appPathOf('https://evil.example/chat/1')).toBe('/')
    expect(appPathOf('/chat/../settings')).toBe('/')
    expect(appPathOf(undefined)).toBe('/')
  })
})
