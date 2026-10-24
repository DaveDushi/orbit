import { afterEach, expect, test } from 'bun:test'
import { act, fireEvent, render } from '@testing-library/react'
import { ThemeProvider } from './theme'
import { useTheme } from './themeContext'

function Controls() {
  const { theme, appearance, setAppearance } = useTheme()
  return (
    <>
      <output data-testid="theme">{theme}</output>
      <output data-testid="appearance">{JSON.stringify(appearance)}</output>
      <button type="button" onClick={() => setAppearance({ theme: 'light', style: 'flat', navStyle: 'floating' })}>change</button>
    </>
  )
}

afterEach(() => {
  localStorage.clear()
  document.documentElement.className = ''
  document.documentElement.removeAttribute('data-flat')
  document.documentElement.removeAttribute('data-floating')
})

test('a choice is saved and applied to the page', () => {
  const view = render(<ThemeProvider><Controls /></ThemeProvider>)
  expect(view.getByTestId('appearance').textContent).toBe('{"theme":"dark","style":"3d","navStyle":"default"}')
  expect(document.documentElement.classList.contains('dark')).toBe(true)

  fireEvent.click(view.getByRole('button', { name: 'change' }))

  expect(view.getByTestId('theme').textContent).toBe('light')
  expect(document.documentElement.classList.contains('dark')).toBe(false)
  expect(document.documentElement.hasAttribute('data-flat')).toBe(true)
  expect(document.documentElement.hasAttribute('data-floating')).toBe(true)
  expect([localStorage.getItem('theme'), localStorage.getItem('orbit:style'), localStorage.getItem('orbit:nav-style')]).toEqual(['light', 'flat', 'floating'])
})

test('the system theme follows the operating system', () => {
  const listeners = new Set<() => void>()
  let dark = true
  const original = window.matchMedia
  window.matchMedia = (() => ({
    get matches() { return dark },
    addEventListener: (_: string, listener: () => void) => listeners.add(listener),
    removeEventListener: (_: string, listener: () => void) => listeners.delete(listener),
  })) as unknown as typeof window.matchMedia
  try {
    localStorage.setItem('theme', 'system')
    const view = render(<ThemeProvider><Controls /></ThemeProvider>)
    expect(view.getByTestId('theme').textContent).toBe('dark')

    dark = false
    act(() => listeners.forEach((listener) => listener()))

    expect(view.getByTestId('theme').textContent).toBe('light')
    expect(document.documentElement.classList.contains('dark')).toBe(false)
    expect(localStorage.getItem('theme')).toBe('system')
  } finally {
    window.matchMedia = original
  }
})
