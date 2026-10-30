import { expect, test } from 'bun:test'
import { openSignInPopup, watchSignInPopup } from './signInPopup'

function fakeHost() {
  const listeners = new Set<() => void>()
  let tick: (() => void) | null = null
  const host = {
    addEventListener: (_: string, listener: () => void) => listeners.add(listener),
    removeEventListener: (_: string, listener: () => void) => listeners.delete(listener),
    setInterval: (callback: () => void) => { tick = callback; return 1 },
    clearInterval: () => { tick = null },
  }
  return { host: host as unknown as Parameters<typeof watchSignInPopup>[0], tick: () => tick?.(), focus: () => [...listeners].forEach((listener) => listener()), running: () => tick !== null }
}

test('the frame reloads once when the popup closes, and once on the return to the page', () => {
  const { host, tick, focus, running } = fakeHost()
  const popup = { closed: false }
  let reloads = 0
  watchSignInPopup(host, popup, () => { reloads += 1 })
  tick()
  expect(reloads).toBe(0)
  focus() // a look at Orbit while the sign-in goes on
  expect(reloads).toBe(0)
  popup.closed = true
  tick()
  tick()
  expect(reloads).toBe(1)
  focus()
  expect(reloads).toBe(2)
  expect(running()).toBe(false)
  focus()
  expect(reloads).toBe(2)
})

test('a popup that only looks closed (COOP) still ends with a reload after the sign-in', () => {
  const { host, tick, focus } = fakeHost()
  const popup = { closed: true }
  let reloads = 0
  watchSignInPopup(host, popup, () => { reloads += 1 })
  tick() // too early: the person has not signed in yet
  focus() // back from the popup, signed in
  expect(reloads).toBe(2)
})

test('stopping ends the watch', () => {
  const { host, tick, focus } = fakeHost()
  const popup = { closed: false }
  let reloads = 0
  const stop = watchSignInPopup(host, popup, () => { reloads += 1 })
  stop()
  popup.closed = true
  tick()
  focus()
  expect(reloads).toBe(0)
})

test('a blocked popup is reported, and an open one cannot reach Orbit through opener', () => {
  expect(openSignInPopup({ open: () => null } as never, 'https://app.test')).toBeNull()
  const popup = { opener: {}, location: { href: 'about:blank' } }
  const opened: unknown[] = []
  const host = { open: (...args: unknown[]) => { opened.push(args[0]); return popup } }
  expect(openSignInPopup(host as never, 'https://app.test/') as unknown).toBe(popup)
  expect(popup.opener).toBeNull()
  expect(popup.location.href).toBe('https://app.test/')
  expect(opened).toEqual(['about:blank'])
})
