import { Pressable, View } from 'react-native'

import { alpha, cssColors, mix, type, useTheme } from '../theme'
import { Icon, type IconName } from './Icon'
import { Spinner } from './Spinner'
import { Text } from './Text'

export type ButtonVariant = 'primary' | 'outline' | 'secondary' | 'ghost' | 'destructive'
/** The web's `default` (32), `sm` (28) and `xs` (24) heights; `lg` (44) is the touch size of forms like sign-in. */
export type ButtonSize = 'xs' | 'sm' | 'default' | 'lg'

const HEIGHT: Record<ButtonSize, number> = { xs: 24, sm: 28, default: 32, lg: 44 }
const TEXT: Record<ButtonSize, number> = { xs: 12, sm: 12.8, default: 14, lg: 15 }

export interface ButtonProps {
  label: string
  onPress?: () => void
  variant?: ButtonVariant
  size?: ButtonSize
  icon?: IconName
  loading?: boolean
  disabled?: boolean
  /** Stretch to the row, as a form's main action. */
  block?: boolean
  /** A quiet ghost button: muted text at 70%, like the web's "Add icon" or "Add relation". */
  muted?: boolean
}

/**
 * The web `Button`: the same variants, and the physical 2px edge under every filled one that drops when pressed.
 * A disabled button is at half opacity, without its edge.
 */
export function Button({ label, onPress, variant = 'primary', size = 'default', icon, loading = false, disabled = false, block = false, muted = false }: ButtonProps) {
  const { colors, scheme } = useTheme()
  const dark = scheme === 'dark'
  const inactive = disabled || loading || !onPress
  const look = {
    primary: { background: colors.primary, pressed: alpha(colors.primary, 0.8), text: colors.primaryForeground, border: 'transparent', edge: mix(colors.primary, 0.75, cssColors.black) },
    outline: dark
      ? { background: alpha(colors.input, 0.3), pressed: alpha(colors.input, 0.5), text: colors.foreground, border: colors.input, edge: alpha(cssColors.white, 0.08) }
      : { background: colors.background, pressed: colors.muted, text: colors.foreground, border: colors.border, edge: alpha(cssColors.black, 0.22) },
    secondary: { background: colors.secondary, pressed: mix(colors.secondary, 0.95, colors.foreground), text: colors.secondaryForeground, border: 'transparent', edge: alpha(dark ? cssColors.white : cssColors.black, dark ? 0.08 : 0.22) },
    ghost: { background: 'transparent', pressed: dark ? alpha(colors.muted, 0.5) : colors.muted, text: colors.foreground, border: 'transparent', edge: undefined },
    destructive: { background: alpha(colors.destructive, dark ? 0.2 : 0.1), pressed: alpha(colors.destructive, dark ? 0.3 : 0.2), text: colors.destructive, border: 'transparent', edge: alpha(colors.destructive, 0.4) },
  }[variant]
  const text = muted ? alpha(colors.mutedForeground, 0.7) : look.text
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled: inactive, busy: loading }}
      disabled={inactive}
      onPress={onPress}
      style={({ pressed }) => ({
        height: HEIGHT[size],
        alignSelf: block ? 'stretch' : undefined,
        flexDirection: 'row',
        alignItems: 'center',
        justifyContent: 'center',
        gap: 6,
        paddingHorizontal: size === 'xs' ? 8 : 10,
        borderRadius: size === 'default' || size === 'lg' ? 10 : 8,
        borderWidth: 1,
        borderColor: look.border,
        backgroundColor: pressed ? look.pressed : look.background,
        opacity: disabled && !loading ? 0.5 : 1,
        boxShadow: look.edge && !pressed && !disabled ? `0 2px 0 ${look.edge}` : undefined,
        transform: [{ translateY: pressed && look.edge ? 2 : 0 }],
      })}
    >
      {loading ? <Spinner color={text} /> : icon ? <View><Icon name={icon} size={size === 'default' || size === 'lg' ? 16 : 14} color={text} /></View> : null}
      <Text style={[type(TEXT[size], 'medium'), { lineHeight: Math.round(TEXT[size] * 1.4), color: text }]}>{label}</Text>
    </Pressable>
  )
}
