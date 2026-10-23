import type { ReactNode } from 'react'
import { Modal, Pressable, ScrollView, View } from 'react-native'
import { useSafeAreaInsets } from 'react-native-safe-area-context'

import { useTheme } from '../theme'
import { Icon } from './Icon'
import { ListRow } from './ListRow'
import { Text } from './Text'

export interface PickerOption {
  key: string
  label: string
  subtitle?: string
  leading?: ReactNode
  selected?: boolean
}

/** A bottom sheet with one choice per row. Tapping outside closes it. */
export function PickerSheet({ visible, title, options, onSelect, onClose }: { visible: boolean; title: string; options: readonly PickerOption[]; onSelect: (key: string) => void; onClose: () => void }) {
  const { colors, radius, spacing, scheme } = useTheme()
  const insets = useSafeAreaInsets()
  return (
    <Modal visible={visible} transparent animationType="slide" onRequestClose={onClose}>
      <Pressable accessibilityLabel="Close" onPress={onClose} style={{ flex: 1, backgroundColor: scheme === 'dark' ? colors.background : colors.foreground, opacity: 0.4 }} />
      <View
        style={{
          maxHeight: '70%',
          backgroundColor: colors.popover,
          borderTopLeftRadius: radius.xl,
          borderTopRightRadius: radius.xl,
          borderColor: colors.border,
          borderWidth: 1,
          paddingBottom: insets.bottom + spacing[2],
        }}
      >
        <Text variant="heading" style={{ padding: spacing[4], paddingBottom: spacing[2] }}>
          {title}
        </Text>
        <ScrollView>
          {options.map((option) => (
            <ListRow
              key={option.key}
              title={option.label}
              subtitle={option.subtitle}
              leading={option.leading}
              trailing={option.selected ? <Icon name="check" size={18} color={colors.primary} /> : undefined}
              onPress={() => onSelect(option.key)}
            />
          ))}
        </ScrollView>
      </View>
    </Modal>
  )
}
