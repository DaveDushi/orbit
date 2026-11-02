import { expect, test } from 'bun:test'
import { type AppFrameEvent, type AppFrameState, appBarView, initialAppFrame, nextAppFrame, readAppBarClosed, readAppKnownReady, rememberAppBarClosed, rememberAppKnownReady } from './appBar'

const open = { embedded: true, ...initialAppFrame, closed: false }

test('an open bar follows the ready rule', () => {
  expect(appBarView(open)).toEqual({ bar: false, notice: null, card: null })
  expect(appBarView({ ...open, ready: true })).toEqual({ bar: false, notice: null, card: null })
  expect(appBarView({ ...open, ready: false })).toEqual({ bar: true, notice: 'help', card: null })
  expect(appBarView({ ...open, ready: true, signingIn: true })).toEqual({ bar: true, notice: 'help', card: null })
  expect(appBarView({ ...open, ready: true, popupBlocked: true })).toEqual({ bar: true, notice: 'popup-blocked', card: null })
})

test('a closed bar stays closed after the timeout and during a sign-in', () => {
  const closed = { ...open, closed: true }
  expect(appBarView({ ...closed, ready: false })).toEqual({ bar: false, notice: null, card: null })
  expect(appBarView({ ...closed, ready: false, signingIn: true })).toEqual({ bar: false, notice: null, card: null })
})

test('a closed bar still says that the popup was blocked', () => {
  expect(appBarView({ ...open, ready: false, closed: true, popupBlocked: true })).toEqual({ bar: false, notice: 'popup-blocked', card: null })
})

test('an app in its own tab keeps the bar: it has no frame and no corner menu', () => {
  expect(appBarView({ ...open, embedded: false, closed: true })).toEqual({ bar: true, notice: null, card: null })
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

test('a known-ready app is kept per workspace and per app, apart from the closed bar', () => {
  const host = fakeHost()
  expect(readAppKnownReady(host, 'w1', 'creators')).toBe(false)
  rememberAppKnownReady(host, 'w1', 'creators')
  expect(readAppKnownReady(host, 'w1', 'creators')).toBe(true)
  expect(readAppKnownReady(host, 'w1', 'reports')).toBe(false)
  expect(readAppKnownReady(host, 'w2', 'creators')).toBe(false)
  expect(readAppBarClosed(host, 'w1', 'creators')).toBe(false)
})

const run = (...events: AppFrameEvent[]) => events.reduce(nextAppFrame, initialAppFrame)
const view = (state: AppFrameState, closed = false) => appBarView({ embedded: true, closed, ...state })

test('a failure shows the card, which replaces the help line in the open bar', () => {
  for (const failure of ['silent', 'login-required', 'frame-blocked'] as const) {
    expect(view(run({ type: 'failed', failure }))).toEqual({ bar: true, notice: null, card: failure })
  }
  // The silence of an app never seen sending the signal: today's bar, no card.
  expect(view(run({ type: 'failed', failure: 'timeout' }))).toEqual({ bar: true, notice: 'help', card: null })
  // A timeout does not replace the reason of a card already shown.
  expect(run({ type: 'failed', failure: 'login-required' }, { type: 'failed', failure: 'timeout' }).card).toBe('login-required')
})

test('with the bar closed the card still shows, and the closed choice stays', () => {
  const failed = run({ type: 'failed', failure: 'silent' })
  expect(view(failed, true)).toEqual({ bar: false, notice: null, card: 'silent' })
  expect(view(nextAppFrame(failed, { type: 'show-app' }), true)).toEqual({ bar: false, notice: null, card: null })
})

test('the ready signal hides the card at once, also a late one', () => {
  const state = run({ type: 'failed', failure: 'frame-blocked' }, { type: 'ready' })
  expect(view(state)).toEqual({ bar: false, notice: null, card: null })
})

test('Reload and Sign in hide the card and start the rule again', () => {
  const failed = run({ type: 'failed', failure: 'login-required' })
  expect(nextAppFrame(failed, { type: 'reload' })).toEqual(initialAppFrame)
  expect(view(nextAppFrame(failed, { type: 'reload' }))).toEqual({ bar: false, notice: null, card: null })
  // Sign in: the card goes while the popup is open; the bar stays with the help line.
  const signingIn = nextAppFrame(failed, { type: 'sign-in', opened: true })
  expect(view(signingIn)).toEqual({ bar: true, notice: 'help', card: null })
  // The popup closes: the frame reloads, and a new failure shows the card again.
  expect(view(run({ type: 'failed', failure: 'login-required' }, { type: 'sign-in', opened: true }, { type: 'reload' }, { type: 'failed', failure: 'silent' })).card).toBe('silent')
})

test('a blocked popup keeps the card, which says so', () => {
  const blocked = run({ type: 'failed', failure: 'silent' }, { type: 'sign-in', opened: false })
  expect(blocked.popupBlocked).toBe(true)
  expect(view(blocked)).toEqual({ bar: true, notice: null, card: 'silent' })
  expect(view(blocked, true)).toEqual({ bar: false, notice: null, card: 'silent' })
  // Show the app: the message moves above the frame.
  expect(view(nextAppFrame(blocked, { type: 'show-app' }), true)).toEqual({ bar: false, notice: 'popup-blocked', card: null })
})

test('Show the app hides the card until the next failure', () => {
  const shown = run({ type: 'failed', failure: 'silent' }, { type: 'show-app' })
  expect(view(shown)).toEqual({ bar: true, notice: 'help', card: null })
  expect(view(nextAppFrame(shown, { type: 'failed', failure: 'frame-blocked' })).card).toBe('frame-blocked')
})

test('a browser without storage does not break the page', () => {
  const failing = { localStorage: { getItem: () => { throw new Error('denied') }, setItem: () => { throw new Error('denied') } } }
  expect(readAppBarClosed(failing, 'w1', 'creators')).toBe(false)
  expect(() => rememberAppBarClosed(failing, 'w1', 'creators')).not.toThrow()
  const noAccess = { get localStorage(): Storage { throw new Error('denied') } }
  expect(readAppBarClosed(noAccess, 'w1', 'creators')).toBe(false)
})
