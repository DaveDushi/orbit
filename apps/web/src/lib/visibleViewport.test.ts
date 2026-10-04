import { expect, test } from 'bun:test'
import { applyVisibleViewport, keyboardOpen, shellHeight, visibleViewportHeight } from './visibleViewport'

test('visible height prefers the visual viewport so Safari chrome is excluded', () => {
  expect(visibleViewportHeight({ height: 612.4 }, 844)).toBe(612)
  expect(visibleViewportHeight(null, 740)).toBe(740)
})

test('an installed app fills 100vh when the visual viewport is only a status bar short', () => {
  expect(shellHeight({ standalone: true, visual: 812, inner: 812, large: 874 })).toBe(874)
  expect(shellHeight({ standalone: false, visual: 612, inner: 844, large: 844 })).toBe(612)
})

test('an installed app still shrinks when the keyboard covers the page', () => {
  expect(shellHeight({ standalone: true, visual: 430, inner: 812, large: 874 })).toBe(430)
})

test('the app height CSS variable is the visible viewport in pixels', () => {
  const properties = new Map<string, string>()
  applyVisibleViewport(
    { style: { setProperty: (name, value) => { properties.set(name, value) } } },
    612,
  )
  expect(properties.get('--app-height')).toBe('612px')
})

test('the keyboard counts as open only when it covers much more than Safari toolbars', () => {
  expect(keyboardOpen({ visual: 430, inner: 812, large: 874 })).toBe(true)
  expect(keyboardOpen({ visual: 664, inner: 664, large: 745 })).toBe(false)
  expect(keyboardOpen({ visual: null, inner: 812, large: 812 })).toBe(false)
})
