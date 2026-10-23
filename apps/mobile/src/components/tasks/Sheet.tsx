import { type ReactNode, useState } from 'react'
import { KeyboardAvoidingView, Modal, Platform, Pressable, ScrollView, TextInput, View } from 'react-native'
import { useSafeAreaInsets } from 'react-native-safe-area-context'

import { type, useTheme } from '../../theme'
import { Icon, type IconName, ListRow, Text } from '../../ui'

/**
 * Which sheet a screen shows. A screen has one sheet whose content changes (a menu row that opens a picker swaps the
 * content in place), so iOS never has to present a modal while another one is closing. The content stays while the
 * sheet slides out.
 */
export function useSheet<S>() {
  const [sheet, setSheet] = useState<S | null>(null)
  const [visible, setVisible] = useState(false)
  return {
    sheet,
    visible,
    open: (next: S) => {
      setSheet(next)
      setVisible(true)
    },
    close: () => setVisible(false),
  }
}

/** The bottom sheet of every task picker (the `PickerSheet` look): a title over the content. Tapping outside closes it. */
export function Sheet({ visible, title, onClose, children }: { visible: boolean; title: string; onClose: () => void; children: ReactNode }) {
  const { colors, radius, spacing, scheme } = useTheme()
  const insets = useSafeAreaInsets()
  return (
    <Modal visible={visible} transparent animationType="slide" onRequestClose={onClose}>
      <KeyboardAvoidingView style={{ flex: 1 }} behavior={Platform.OS === 'ios' ? 'padding' : undefined}>
        <Pressable accessibilityLabel="Close" onPress={onClose} style={{ flex: 1, backgroundColor: scheme === 'dark' ? colors.background : colors.foreground, opacity: 0.4 }} />
        <View
          accessibilityViewIsModal
          style={{
            maxHeight: '80%',
            backgroundColor: colors.popover,
            borderTopLeftRadius: radius.xl,
            borderTopRightRadius: radius.xl,
            borderColor: colors.border,
            borderWidth: 1,
            paddingBottom: insets.bottom + spacing[2],
          }}
        >
          <Text variant="heading" numberOfLines={1} style={{ padding: spacing[4], paddingBottom: spacing[2] }}>
            {title}
          </Text>
          {children}
        </View>
      </KeyboardAvoidingView>
    </Modal>
  )
}

/** The rows of a sheet, scrolling under its title and search. */
export function SheetList({ children }: { children: ReactNode }) {
  return (
    <ScrollView keyboardShouldPersistTaps="handled" style={{ flexShrink: 1 }}>
      {children}
    </ScrollView>
  )
}

export function SheetSearch({ value, onChange, placeholder, autoFocus = false }: { value: string; onChange: (text: string) => void; placeholder: string; autoFocus?: boolean }) {
  const { colors, spacing, scheme } = useTheme()
  return (
    <View style={{ marginHorizontal: spacing[4], marginBottom: spacing[2], flexDirection: 'row', alignItems: 'center', gap: 8, height: 36, paddingHorizontal: 10, borderRadius: 10, borderWidth: 1, borderColor: colors.input }}>
      <Icon name="search" size={16} color={colors.mutedForeground} />
      <TextInput
        value={value}
        onChangeText={onChange}
        placeholder={placeholder}
        accessibilityLabel={placeholder}
        placeholderTextColor={colors.mutedForeground}
        selectionColor={colors.primary}
        keyboardAppearance={scheme}
        autoFocus={autoFocus}
        autoCorrect={false}
        autoCapitalize="none"
        style={{ flex: 1, ...type(16), lineHeight: 22, padding: 0, color: colors.foreground }}
      />
    </View>
  )
}

/** One sheet row. `checked` draws the check of a chosen row (single or multi choice). */
export function SheetRow({ label, subtitle, leading, icon, checked, onPress }: { label: string; subtitle?: string; leading?: ReactNode; icon?: IconName; checked?: boolean; onPress: () => void }) {
  const { colors } = useTheme()
  return (
    <ListRow
      title={label}
      subtitle={subtitle}
      leading={leading ?? (icon ? <Icon name={icon} size={18} color={colors.mutedForeground} /> : undefined)}
      trailing={checked ? <Icon name="check" size={18} color={colors.primary} /> : undefined}
      onPress={onPress}
    />
  )
}

/** Quiet text in a sheet: an empty list, a loading search, an error. */
export function SheetNote({ children, tone = 'muted' }: { children: ReactNode; tone?: 'muted' | 'destructive' }) {
  const { colors, spacing } = useTheme()
  return <Text style={[type(13), { paddingHorizontal: spacing[4], paddingVertical: spacing[3], color: tone === 'destructive' ? colors.destructive : colors.mutedForeground }]}>{children}</Text>
}
