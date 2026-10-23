import { useSyncExternalStore } from 'react'
import { Platform, type TextStyle, useColorScheme } from 'react-native'

import { alpha, mix } from '../data/color'
import { storage } from '../storage'
import { dark, light, radius } from './tokens.gen'

export type ColorName = keyof typeof light
export type Colors = Record<ColorName, string>
export type Scheme = 'light' | 'dark'

/** 4px steps, matching Tailwind's spacing scale on the web (`p-1` = 4px). */
export const spacing = { 0: 0, 0.5: 2, 1: 4, 1.5: 6, 2: 8, 2.5: 10, 3: 12, 4: 16, 5: 20, 6: 24, 8: 32, 10: 40, 12: 48 } as const

/** Noto Sans faces loaded in the root layout. Each weight is its own family, so set `fontFamily`, not `fontWeight`. */
export const fonts = {
  regular: 'NotoSans_400Regular',
  medium: 'NotoSans_500Medium',
  semibold: 'NotoSans_600SemiBold',
  bold: 'NotoSans_700Bold',
  mono: Platform.select({ ios: 'Menlo', default: 'monospace' }),
} as const

export type Weight = 'regular' | 'medium' | 'semibold' | 'bold'

/** A web text size: the body's 1.45 line height, as `text-[13px] font-medium` renders in the browser. */
export const type = (size: number, weight: Weight = 'regular'): TextStyle => ({ fontSize: size, lineHeight: Math.round(size * 145) / 100, fontFamily: fonts[weight] })

export const palettes: Record<Scheme, Colors> = { light, dark }

/** The CSS keywords the web mixes with: `color-mix(…, black 25%)` button edges, the `bg-black/50` scrim. */
export const cssColors = { black: '#000000', white: '#ffffff' } as const

/** Tailwind green-500, amber-500 and red-500: the web `StatusMark` draws presence with these in both themes. */
export const presenceColors = { online: '#00c950', idle: '#fe9a00', dnd: '#fb2c36' } as const

export { alpha, mix }

export interface Theme {
  scheme: Scheme
  colors: Colors
  spacing: typeof spacing
  radius: typeof radius
  fonts: typeof fonts
}

const SCHEME_KEY = 'orbit.theme'
/** `null` follows the system; the Profile header's theme button sets one explicitly, like the web toggle. */
let preference: Scheme | null = null
const listeners = new Set<() => void>()

const subscribe = (listener: () => void) => {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

export function setSchemePreference(scheme: Scheme | null) {
  preference = scheme
  listeners.forEach((listener) => listener())
  void (scheme ? storage.set(SCHEME_KEY, scheme) : storage.remove(SCHEME_KEY))
}

/** Reads the stored choice once at startup. */
export async function loadSchemePreference(): Promise<void> {
  const stored = await storage.get(SCHEME_KEY).catch(() => null)
  if (stored === 'light' || stored === 'dark') setSchemePreference(stored)
}

export function useTheme(): Theme {
  const system = useColorScheme()
  const chosen = useSyncExternalStore(subscribe, () => preference)
  const scheme: Scheme = chosen ?? (system === 'dark' ? 'dark' : 'light')
  return { scheme, colors: palettes[scheme], spacing, radius, fonts }
}

export { radius }
