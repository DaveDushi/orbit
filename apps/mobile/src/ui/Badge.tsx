import { View } from 'react-native'

import { type ColorName, fonts, useTheme } from '../theme'
import { Text } from './Text'

export type BadgeVariant = 'primary' | 'secondary' | 'outline' | 'destructive'

const LOOK: Record<BadgeVariant, { background?: ColorName; text: ColorName; border?: ColorName }> = {
  primary: { background: 'primary', text: 'primaryForeground' },
  secondary: { background: 'secondary', text: 'secondaryForeground' },
  outline: { text: 'foreground', border: 'border' },
  destructive: { background: 'destructive', text: 'primaryForeground' },
}

export function Badge({ label, variant = 'secondary' }: { label: string; variant?: BadgeVariant }) {
  const { colors, spacing } = useTheme()
  const look = LOOK[variant]
  return (
    <View
      style={{
        alignSelf: 'flex-start',
        minHeight: 20,
        justifyContent: 'center',
        paddingHorizontal: spacing[2],
        borderRadius: 999,
        borderWidth: 1,
        borderColor: look.border ? colors[look.border] : 'transparent',
        backgroundColor: look.background ? colors[look.background] : 'transparent',
      }}
    >
      <Text variant="caption" style={{ color: colors[look.text], fontFamily: fonts.medium }}>
        {label}
      </Text>
    </View>
  )
}
