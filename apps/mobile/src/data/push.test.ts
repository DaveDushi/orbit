import { describe, expect, test } from 'bun:test'

import { appPathOf, isOpenOnScreen, notificationUrl, shouldRegister, thisDeviceState, workspaceOf } from './push'

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

describe('this iPhone', () => {
  test('on only when iOS allows it and the person did not turn it off here', () => {
    expect(thisDeviceState({ ios: true, granted: true, canAskAgain: true, off: false })).toBe('on')
    expect(thisDeviceState({ ios: true, granted: true, canAskAgain: true, off: true })).toBe('off')
    expect(thisDeviceState({ ios: true, granted: false, canAskAgain: true, off: false })).toBe('off')
    expect(thisDeviceState({ ios: true, granted: false, canAskAgain: false, off: false })).toBe('blocked')
    expect(thisDeviceState({ ios: false, granted: true, canAskAgain: true, off: false })).toBe('unsupported')
  })

  test('a phone turned off here does not register again on the next start', () => {
    expect(shouldRegister({ off: true, granted: true, canAskAgain: true })).toBe(false)
    expect(shouldRegister({ off: false, granted: true, canAskAgain: false })).toBe(true)
    expect(shouldRegister({ off: false, granted: false, canAskAgain: true })).toBe(true)
    expect(shouldRegister({ off: false, granted: false, canAskAgain: false })).toBe(false)
  })
})

describe('notificationUrl', () => {
  test('a tap reads the url from the data, else from the APNs payload of the push', () => {
    // expo-notifications: `data` is the payload's `body` object (null when the server sent none)
    expect(notificationUrl({ url: '/chat/1' }, { type: 'push', payload: { url: '/activity' } })).toBe('/chat/1')
    expect(notificationUrl(null, { type: 'push', payload: { aps: {}, url: '/chat/1', tag: 'chat:1' } })).toBe('/chat/1')
    expect(appPathOf(notificationUrl(undefined, { type: 'push', payload: { url: '/chat/1?thread=2' } }))).toBe('/chat/thread/2')
    expect(notificationUrl(null, { type: 'timeInterval', payload: { url: '/chat/1' } })).toBeUndefined()
    expect(notificationUrl(null, null)).toBeUndefined()
  })
})

describe('workspaceOf and isOpenOnScreen', () => {
  test('a push names its workspace, and its banner hides only for the conversation or thread on screen', () => {
    expect(workspaceOf('/chat/1?thread=2&workspace=w-9')).toBe('w-9')
    expect(workspaceOf('/activity?workspace=w-9')).toBe('w-9')
    expect(workspaceOf('/chat/1')).toBeNull()
    expect(workspaceOf('/chat/1?workspace=../x')).toBeNull()
    expect(workspaceOf(undefined)).toBeNull()
    // The workspace does not change the screen it opens.
    expect(appPathOf('/chat/1?workspace=w')).toBe('/chat/1')
    expect(isOpenOnScreen('/chat/1?workspace=w', '/chat/1')).toBe(true)
    expect(isOpenOnScreen('/chat/1?thread=2&workspace=w', '/chat/thread/2')).toBe(true)
    expect(isOpenOnScreen('/chat/1?thread=2&workspace=w', '/chat/1')).toBe(false)
    expect(isOpenOnScreen('/chat/1', '/chat/3')).toBe(false)
    expect(isOpenOnScreen('/activity', '/activity')).toBe(false)
    expect(isOpenOnScreen('/chat/1', null)).toBe(false)
  })
})
