import { Text as NativeText, type TextProps as NativeTextProps, type TextStyle } from 'react-native'

import { type ColorName, fonts, useTheme } from '../theme'

export type TextVariant = 'title' | 'heading' | 'body' | 'label' | 'caption' | 'mono'
export type TextColor = 'default' | 'muted' | 'primary' | 'destructive'

const VARIANTS: Record<TextVariant, TextStyle> = {
  title: { fontFamily: fonts.semibold, fontSize: 24, lineHeight: 30, letterSpacing: -0.3 },
  heading: { fontFamily: fonts.semibold, fontSize: 17, lineHeight: 22 },
  body: { fontFamily: fonts.regular, fontSize: 15, lineHeight: 21 },
  label: { fontFamily: fonts.medium, fontSize: 14, lineHeight: 19 },
  caption: { fontFamily: fonts.regular, fontSize: 12, lineHeight: 16 },
  mono: { fontFamily: fonts.mono, fontSize: 13, lineHeight: 18 },
}

const COLORS: Record<TextColor, ColorName> = {
  default: 'foreground',
  muted: 'mutedForeground',
  primary: 'primary',
  destructive: 'destructive',
}

export interface TextProps extends NativeTextProps {
  variant?: TextVariant
  color?: TextColor
}

export function Text({ variant = 'body', color = 'default', style, ...props }: TextProps) {
  const { colors } = useTheme()
  return <NativeText {...props} style={[VARIANTS[variant], { color: colors[COLORS[color]] }, style]} />
}
