import { Platform, useColorScheme } from 'react-native'

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

export const palettes: Record<Scheme, Colors> = { light, dark }

export interface Theme {
  scheme: Scheme
  colors: Colors
  spacing: typeof spacing
  radius: typeof radius
  fonts: typeof fonts
}

export function useTheme(): Theme {
  const scheme: Scheme = useColorScheme() === 'dark' ? 'dark' : 'light'
  return { scheme, colors: palettes[scheme], spacing, radius, fonts }
}

export { radius }
