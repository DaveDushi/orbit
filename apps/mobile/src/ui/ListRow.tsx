import type { ReactNode } from 'react'
import { Pressable, StyleSheet, View } from 'react-native'

import { useTheme } from '../theme'
import { Text } from './Text'

export interface ListRowProps {
  title: string
  subtitle?: string
  leading?: ReactNode
  trailing?: ReactNode
  onPress?: () => void
  /** A 1px line under the row, inset to the text. */
  separator?: boolean
}

export function ListRow({ title, subtitle, leading, trailing, onPress, separator = false }: ListRowProps) {
  const { colors, spacing } = useTheme()
  return (
    <Pressable
      accessibilityRole={onPress ? 'button' : undefined}
      disabled={!onPress}
      onPress={onPress}
      style={({ pressed }) => [styles.row, { paddingLeft: spacing[4], gap: spacing[3], backgroundColor: pressed ? colors.accent : 'transparent' }]}
    >
      {leading}
      <View style={[styles.body, { paddingRight: spacing[4], gap: spacing[3] }, separator && { borderBottomWidth: 1, borderBottomColor: colors.border }]}>
        <View style={styles.text}>
          <Text variant="label" numberOfLines={1}>
            {title}
          </Text>
          {subtitle ? (
            <Text variant="caption" color="muted" numberOfLines={1}>
              {subtitle}
            </Text>
          ) : null}
        </View>
        {trailing}
      </View>
    </Pressable>
  )
}

const styles = StyleSheet.create({
  row: { flexDirection: 'row', alignItems: 'center' },
  body: { flex: 1, minHeight: 52, flexDirection: 'row', alignItems: 'center', alignSelf: 'stretch' },
  text: { flex: 1, gap: 2, paddingVertical: 8 },
})
