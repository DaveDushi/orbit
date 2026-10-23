import type { ReactNode } from 'react'
import { Pressable, View, type ViewStyle } from 'react-native'

import { alpha, cssColors, mix, type, useTheme } from '../theme'
import { Icon, type IconName } from './Icon'
import { Text } from './Text'

/** The web's 48px pane header on a phone: 8px sides, 8px gaps, no bottom border. */
export function PaneHeader({ children, style }: { children: ReactNode; style?: ViewStyle }) {
  const { colors } = useTheme()
  return <View style={[{ minHeight: 48, flexDirection: 'row', alignItems: 'center', gap: 8, paddingHorizontal: 8, backgroundColor: colors.background }, style]}>{children}</View>
}

/** `PaneTitle`: 13px semibold. Home's topbar crumb is 14px. */
export function HeaderTitle({ children, size = 13 }: { children: ReactNode; size?: number }) {
  return (
    <Text numberOfLines={1} style={[type(size, 'semibold'), { flexShrink: 1 }]}>
      {children}
    </Text>
  )
}

export interface IconButtonProps {
  icon: IconName
  label: string
  onPress?: () => void
  /** 28 is the web's `icon-sm`, 32 its `icon`, 22 the row pickers. */
  size?: number
  iconSize?: number
  color?: string
  /** `primary` is the pink square with the web button's 2px pressed-edge. */
  variant?: 'ghost' | 'primary'
  disabled?: boolean
}

export function IconButton({ icon, label, onPress, size = 28, iconSize = 16, color, variant = 'ghost', disabled }: IconButtonProps) {
  const { colors } = useTheme()
  const primary = variant === 'primary'
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      disabled={disabled || !onPress}
      onPress={onPress}
      hitSlop={size < 32 ? (32 - size) / 2 + 4 : 4}
      style={({ pressed }) => [
        { width: size, height: size, borderRadius: size >= 32 ? 10 : 8, alignItems: 'center', justifyContent: 'center' },
        primary
          ? { backgroundColor: pressed ? alpha(colors.primary, 0.8) : colors.primary, boxShadow: pressed ? undefined : `0 2px 0 ${mix(colors.primary, 0.75, cssColors.black)}`, transform: [{ translateY: pressed ? 2 : 0 }] }
          : { backgroundColor: pressed ? colors.muted : 'transparent' },
        disabled && { opacity: 0.5 },
      ]}
    >
      <Icon name={icon} size={iconSize} color={color ?? (primary ? colors.primaryForeground : colors.foreground)} />
    </Pressable>
  )
}
