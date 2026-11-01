import { expect, test } from 'bun:test'
import { appBarView, readAppBarClosed, rememberAppBarClosed } from './appBar'

const open = { embedded: true, ready: null, signingIn: false, popupBlocked: false, closed: false }

test('an open bar follows the ready rule', () => {
  expect(appBarView(open)).toEqual({ bar: false, notice: null })
  expect(appBarView({ ...open, ready: true })).toEqual({ bar: false, notice: null })
  expect(appBarView({ ...open, ready: false })).toEqual({ bar: true, notice: 'help' })
  expect(appBarView({ ...open, ready: true, signingIn: true })).toEqual({ bar: true, notice: 'help' })
  expect(appBarView({ ...open, ready: true, popupBlocked: true })).toEqual({ bar: true, notice: 'popup-blocked' })
})

test('a closed bar stays closed after the timeout and during a sign-in', () => {
  const closed = { ...open, closed: true }
  expect(appBarView({ ...closed, ready: false })).toEqual({ bar: false, notice: null })
  expect(appBarView({ ...closed, ready: false, signingIn: true })).toEqual({ bar: false, notice: null })
})

test('a closed bar still says that the popup was blocked', () => {
  expect(appBarView({ ...open, ready: false, closed: true, popupBlocked: true })).toEqual({ bar: false, notice: 'popup-blocked' })
})

test('an app in its own tab keeps the bar: it has no frame and no corner menu', () => {
  expect(appBarView({ ...open, embedded: false, closed: true })).toEqual({ bar: true, notice: null })
})

function fakeHost() {
  const items = new Map<string, string>()
  return { localStorage: { getItem: (key: string) => items.get(key) ?? null, setItem: (key: string, value: string) => void items.set(key, value) } }
}

test('the choice is kept per workspace and per app', () => {
  const host = fakeHost()
  expect(readAppBarClosed(host, 'w1', 'creators')).toBe(false)
  rememberAppBarClosed(host, 'w1', 'creators')
  expect(readAppBarClosed(host, 'w1', 'creators')).toBe(true)
  expect(readAppBarClosed(host, 'w1', 'reports')).toBe(false)
  expect(readAppBarClosed(host, 'w2', 'creators')).toBe(false)
})

test('a browser without storage does not break the page', () => {
  const failing = { localStorage: { getItem: () => { throw new Error('denied') }, setItem: () => { throw new Error('denied') } } }
  expect(readAppBarClosed(failing, 'w1', 'creators')).toBe(false)
  expect(() => rememberAppBarClosed(failing, 'w1', 'creators')).not.toThrow()
  const noAccess = { get localStorage(): Storage { throw new Error('denied') } }
  expect(readAppBarClosed(noAccess, 'w1', 'creators')).toBe(false)
})
