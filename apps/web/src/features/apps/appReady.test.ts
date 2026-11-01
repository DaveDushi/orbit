import { expect, test } from 'bun:test'
import { APP_READY_MESSAGE, APP_READY_TIMEOUT_MILLIS, isAppReadyMessage, watchAppReady } from './appReady'

const ORIGIN = 'https://products.example.com'
const frame = { name: 'the app frame' }
const good = { data: { type: APP_READY_MESSAGE }, origin: ORIGIN, source: frame } as unknown as MessageEvent

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

test('without a signal the timeout fires, so a blank frame cannot stay without a way out', () => {
  const { host, fire, millis } = fakeHost()
  const seen: string[] = []
  watchAppReady(host, ORIGIN, () => frame, () => seen.push('ready'), () => seen.push('timeout'))
  expect(millis()).toBe(APP_READY_TIMEOUT_MILLIS)
  fire()
  expect(seen).toEqual(['timeout'])
})

test('a wrong message does not stop the timeout', () => {
  const { host, send, timing } = fakeHost()
  const seen: string[] = []
  watchAppReady(host, ORIGIN, () => frame, () => seen.push('ready'), () => seen.push('timeout'))
  send({ ...good, origin: 'https://evil.example.com' } as MessageEvent)
  expect(seen).toEqual([])
  expect(timing()).toBe(true)
})

test('the signal stops the timeout, a late signal still counts, and stop ends the watch', () => {
  const { host, send, fire, timing, listening } = fakeHost()
  const seen: string[] = []
  const stop = watchAppReady(host, ORIGIN, () => frame, () => seen.push('ready'), () => seen.push('timeout'))
  send(good)
  expect(timing()).toBe(false)
  expect(seen).toEqual(['ready'])

  const late = fakeHost()
  const lateSeen: string[] = []
  watchAppReady(late.host, ORIGIN, () => frame, () => lateSeen.push('ready'), () => lateSeen.push('timeout'))
  late.fire()
  late.send(good)
  expect(lateSeen).toEqual(['timeout', 'ready'])

  stop()
  expect(listening()).toBe(false)
  fire()
})
