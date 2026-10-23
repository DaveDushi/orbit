import { Pressable, View } from 'react-native'

import { fonts, useTheme } from '../theme'
import { Text } from './Text'

export function Segmented<T extends string>({ options, value, onChange }: { options: readonly { value: T; label: string }[]; value: T; onChange: (value: T) => void }) {
  const { colors, radius, spacing } = useTheme()
  return (
    <View accessibilityRole="tablist" style={{ flexDirection: 'row', padding: 2, borderRadius: radius.lg, backgroundColor: colors.muted }}>
      {options.map((option) => {
        const selected = option.value === value
        return (
          <Pressable
            key={option.value}
            accessibilityRole="tab"
            accessibilityState={{ selected }}
            onPress={() => onChange(option.value)}
            style={{
              flex: 1,
              alignItems: 'center',
              paddingVertical: spacing[1.5],
              borderRadius: radius.md,
              backgroundColor: selected ? colors.background : 'transparent',
              borderWidth: 1,
              borderColor: selected ? colors.border : 'transparent',
            }}
          >
            <Text variant="label" style={{ fontFamily: selected ? fonts.semibold : fonts.medium, color: selected ? colors.foreground : colors.mutedForeground }}>
              {option.label}
            </Text>
          </Pressable>
        )
      })}
    </View>
  )
}
