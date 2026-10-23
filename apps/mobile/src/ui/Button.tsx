import type { ReactNode } from 'react'
import { Pressable, StyleSheet, View } from 'react-native'

import { type ColorName, useTheme } from '../theme'
import { Spinner } from './Spinner'
import { Text } from './Text'

export type ButtonVariant = 'primary' | 'secondary' | 'ghost'

const LOOK: Record<ButtonVariant, { background?: ColorName; pressed: ColorName; text: ColorName; border?: ColorName }> = {
  primary: { background: 'primary', pressed: 'primary', text: 'primaryForeground' },
  secondary: { background: 'secondary', pressed: 'accent', text: 'secondaryForeground', border: 'border' },
  ghost: { pressed: 'accent', text: 'foreground' },
}

export interface ButtonProps {
  label: string
  onPress: () => void
  variant?: ButtonVariant
  loading?: boolean
  disabled?: boolean
  /** An icon or other element before the label. */
  leading?: ReactNode
}

export function Button({ label, onPress, variant = 'primary', loading = false, disabled = false, leading }: ButtonProps) {
  const { colors, radius, spacing } = useTheme()
  const look = LOOK[variant]
  const inactive = disabled || loading
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled: inactive, busy: loading }}
      disabled={inactive}
      onPress={onPress}
      style={({ pressed }) => [
        styles.button,
        {
          borderRadius: radius.lg,
          paddingHorizontal: spacing[4],
          gap: spacing[2],
          backgroundColor: pressed ? colors[look.pressed] : look.background ? colors[look.background] : 'transparent',
          borderColor: look.border ? colors[look.border] : 'transparent',
        },
        pressed && variant === 'primary' && styles.primaryPressed,
        disabled && !loading && styles.disabled,
      ]}
    >
      {loading ? <Spinner color={colors[look.text]} /> : leading ? <View>{leading}</View> : null}
      <Text variant="label" style={{ color: colors[look.text] }}>
        {label}
      </Text>
    </Pressable>
  )
}

const styles = StyleSheet.create({
  button: { minHeight: 44, flexDirection: 'row', alignItems: 'center', justifyContent: 'center', borderWidth: 1 },
  primaryPressed: { opacity: 0.85 },
  disabled: { opacity: 0.5 },
})
