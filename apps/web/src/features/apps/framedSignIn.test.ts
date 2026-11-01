import { expect, test } from 'bun:test'
import { blocksCookiesInFrames } from './framedSignIn'

const SAFARI_IPHONE = 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1'
const CHROME_IPHONE = 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/130.0 Mobile/15E148 Safari/604.1'
const SAFARI_MAC = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15'
const CHROME_MAC = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36'
const EDGE = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36 Edg/130.0'
const CHROME_ANDROID = 'Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Mobile Safari/537.36'
const FIREFOX_MAC = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:130.0) Gecko/20100101 Firefox/130.0'

test('Safari and every iOS browser block cookies in frames, Chromium and Firefox do not by default', () => {
  for (const userAgent of [SAFARI_IPHONE, CHROME_IPHONE, SAFARI_MAC]) expect(blocksCookiesInFrames({ userAgent, maxTouchPoints: 0 })).toBe(true)
  for (const userAgent of [CHROME_MAC, EDGE, CHROME_ANDROID, FIREFOX_MAC]) expect(blocksCookiesInFrames({ userAgent, maxTouchPoints: 0 })).toBe(false)
})

test('an iPad that says it is a Mac is found by its touch screen', () => {
  expect(blocksCookiesInFrames({ userAgent: SAFARI_MAC, maxTouchPoints: 5 })).toBe(true)
})
