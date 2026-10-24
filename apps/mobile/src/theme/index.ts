import { useSyncExternalStore } from 'react'
import { Platform, useColorScheme } from 'react-native'
import { Uniwind } from 'uniwind'

import { alpha, mix } from '../data/color'
import { storage } from '../storage'
import { dark, light } from './tokens.gen'

export type ColorName = keyof typeof light
export type Colors = Record<ColorName, string>
export type Scheme = 'light' | 'dark'

/** Noto Sans faces loaded in the root layout. Each weight is its own family, so set `fontFamily`, not `fontWeight`. */
export const fonts = {
  regular: 'NotoSans_400Regular',
  medium: 'NotoSans_500Medium',
  semibold: 'NotoSans_600SemiBold',
  bold: 'NotoSans_700Bold',
  mono: Platform.select({ ios: 'Menlo', default: 'monospace' }),
} as const

export const palettes: Record<Scheme, Colors> = { light, dark }

/** The CSS keywords the web mixes with: `color-mix(…, black 25%)` button edges, the `bg-black/50` scrim. */
export const cssColors = { black: '#000000', white: '#ffffff' } as const

/** Tailwind green-500, amber-500 and red-500: the web `StatusMark` draws presence with these in both themes. */
export const presenceColors = { online: '#00c950', idle: '#fe9a00', dnd: '#fb2c36' } as const

export { alpha, mix }

export interface Theme {
  scheme: Scheme
  colors: Colors
  fonts: typeof fonts
}

const SCHEME_KEY = 'orbit.theme'
const STYLE_KEY = 'orbit.style'
/** `null` follows the system; the Profile header's theme button sets one explicitly, like the web toggle. */
let preference: Scheme | null = null
/** The web's Appearance → Style: `3d` buttons rest on a raised edge, `flat` ones have none. */
export type ButtonStyle = '3d' | 'flat'
let buttonStyle: ButtonStyle = '3d'
const listeners = new Set<() => void>()

const subscribe = (listener: () => void) => {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

export function setSchemePreference(scheme: Scheme | null) {
  preference = scheme
  // Uniwind's `dark:` classes follow the same choice ('system' follows the device again).
  Uniwind.setTheme(scheme ?? 'system')
  listeners.forEach((listener) => listener())
  void (scheme ? storage.set(SCHEME_KEY, scheme) : storage.remove(SCHEME_KEY))
}

export function setButtonStyle(style: ButtonStyle) {
  buttonStyle = style
  listeners.forEach((listener) => listener())
  void storage.set(STYLE_KEY, style)
}

/** Reads the stored choices once at startup. */
export async function loadAppearance(): Promise<void> {
  const [scheme, style] = await Promise.all([storage.get(SCHEME_KEY).catch(() => null), storage.get(STYLE_KEY).catch(() => null)])
  if (scheme === 'light' || scheme === 'dark') setSchemePreference(scheme)
  if (style === 'flat') setButtonStyle(style)
}

/** The choice in Settings → Appearance: `system` when none is set. */
export function useSchemePreference(): Scheme | 'system' {
  return useSyncExternalStore(subscribe, () => preference ?? 'system')
}

export function useButtonStyle(): ButtonStyle {
  return useSyncExternalStore(subscribe, () => buttonStyle)
}

export function useTheme(): Theme {
  const system = useColorScheme()
  const chosen = useSyncExternalStore(subscribe, () => preference)
  const scheme: Scheme = chosen ?? (system === 'dark' ? 'dark' : 'light')
  return { scheme, colors: palettes[scheme], fonts }
}
