import { expect, test } from 'bun:test'
import { APP_LOADED_TIMEOUT_MILLIS, APP_LOGIN_REQUIRED_MESSAGE, APP_READY_MESSAGE, APP_READY_TIMEOUT_MILLIS, type AppFailure, isAppReadyMessage, isLoginRequiredMessage, watchAppReady } from './appReady'

const ORIGIN = 'https://products.example.com'
const ORBIT = 'https://orbit.example.com'
const frame = { name: 'the app frame' }
const good = { data: { type: APP_READY_MESSAGE }, origin: ORIGIN, source: frame } as unknown as MessageEvent
const loginRequired = { data: { type: APP_LOGIN_REQUIRED_MESSAGE }, origin: ORBIT, source: frame } as unknown as MessageEvent

test('only the app frame, from the app origin, with the right message counts', () => {
  expect(isAppReadyMessage(good, ORIGIN, frame)).toBe(true)
  for (const event of [
    { ...good, origin: 'https://evil.example.com' },
    { ...good, origin: 'https://products.example.com.evil.test' },
    { ...good, source: { name: 'another window' } },
    { ...good, source: null },
    { ...good, data: { type: 'other' } },
    { ...good, data: APP_READY_MESSAGE },
    { ...good, data: null },
    { ...good, data: undefined },
  ]) expect(isAppReadyMessage(event as MessageEvent, ORIGIN, frame)).toBe(false)
  // The frame is not there yet: a message with no source must not match it.
  expect(isAppReadyMessage({ ...good, source: null } as MessageEvent, ORIGIN, null)).toBe(false)
})

test('login required counts only from Orbit’s origin in the app frame', () => {
  expect(isLoginRequiredMessage(loginRequired, ORBIT, frame)).toBe(true)
  for (const event of [
    // The app itself cannot say it, nor another site, another window, or Orbit's own top page.
    { ...loginRequired, origin: ORIGIN },
    { ...loginRequired, origin: 'https://orbit.example.com.evil.test' },
    { ...loginRequired, source: { name: 'another window' } },
    { ...loginRequired, source: null },
    { ...loginRequired, data: { type: APP_READY_MESSAGE } },
    { ...loginRequired, data: APP_LOGIN_REQUIRED_MESSAGE },
  ]) expect(isLoginRequiredMessage(event as MessageEvent, ORBIT, frame)).toBe(false)
  expect(isLoginRequiredMessage({ ...loginRequired, source: null } as MessageEvent, ORBIT, null)).toBe(false)
})

function fakeHost() {
  let listener: ((event: MessageEvent) => void) | null = null
  let timer: { callback: () => void; millis: number } | null = null
  const host = {
    addEventListener: (_: 'message', next: (event: MessageEvent) => void) => { listener = next },
    removeEventListener: () => { listener = null },
    setTimeout: (callback: () => void, millis: number) => { timer = { callback, millis }; return 1 },
    clearTimeout: () => { timer = null },
  }
  return { host, send: (event: MessageEvent) => listener?.(event), fire: () => timer?.callback(), millis: () => timer?.millis, listening: () => listener !== null, timing: () => timer !== null }
}

function watch(knownReady = false) {
  const fake = fakeHost()
  const seen: (AppFailure | 'ready')[] = []
  const app = watchAppReady(fake.host, { appOrigin: ORIGIN, orbitOrigin: ORBIT, frame: () => frame, knownReady }, () => seen.push('ready'), (failure) => seen.push(failure))
  return { ...fake, ...app, seen }
}

test('without a signal the timeout fires, so a blank frame cannot stay without a way out', () => {
  const { fire, millis, seen } = watch()
  expect(millis()).toBe(APP_READY_TIMEOUT_MILLIS)
  fire()
  expect(seen).toEqual(['timeout'])
})

test('an app never seen sending the signal keeps 8 s after a load: the bar, no card', () => {
  const { loaded, fire, millis, seen } = watch(false)
  loaded()
  expect(millis()).toBe(APP_READY_TIMEOUT_MILLIS)
  fire()
  expect(seen).toEqual(['timeout'])
})

test('a known app is silent 3 s after its last load: a failure with the card', () => {
  const { loaded, fire, millis, seen } = watch(true)
  expect(millis()).toBe(APP_READY_TIMEOUT_MILLIS) // no load yet
  loaded()
  expect(millis()).toBe(APP_LOADED_TIMEOUT_MILLIS)
  loaded() // a redirect inside the frame: the time starts again
  expect(millis()).toBe(APP_LOADED_TIMEOUT_MILLIS)
  fire()
  expect(seen).toEqual(['silent'])
})

test('a known app that never loads fails after 8 s', () => {
  const { fire, seen } = watch(true)
  fire()
  expect(seen).toEqual(['silent'])
})

test('loads after the signal do not start the time again', () => {
  const { send, loaded, timing, seen } = watch(true)
  loaded()
  send(good)
  loaded()
  expect(timing()).toBe(false)
  expect(seen).toEqual(['ready'])
})

test('login required and a refused frame fail at once, for every app, and stop the time', () => {
  const login = watch(false)
  login.send({ ...loginRequired, origin: ORIGIN } as MessageEvent) // from the app: ignored
  expect(login.seen).toEqual([])
  login.send(loginRequired)
  expect(login.seen).toEqual(['login-required'])
  expect(login.timing()).toBe(false)
  login.loaded() // the callback page loads after it
  expect(login.timing()).toBe(false)

  const blocked = watch(true)
  blocked.loaded()
  blocked.blocked()
  expect(blocked.seen).toEqual(['frame-blocked'])
  expect(blocked.timing()).toBe(false)
  // A late signal still counts.
  blocked.send(good)
  expect(blocked.seen).toEqual(['frame-blocked', 'ready'])
})

test('a wrong message does not stop the timeout', () => {
  const { send, timing, seen } = watch()
  send({ ...good, origin: 'https://evil.example.com' } as MessageEvent)
  expect(seen).toEqual([])
  expect(timing()).toBe(true)
})

test('the signal stops the timeout, a late signal still counts, and stop ends the watch', () => {
  const first = watch()
  first.send(good)
  expect(first.timing()).toBe(false)
  expect(first.seen).toEqual(['ready'])

  const late = watch()
  late.fire()
  late.send(good)
  expect(late.seen).toEqual(['timeout', 'ready'])

  const stopped = watch()
  stopped.stop()
  expect(stopped.listening()).toBe(false)
  expect(stopped.timing()).toBe(false)
})
