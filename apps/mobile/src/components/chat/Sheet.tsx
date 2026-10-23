import type { ReactNode } from 'react'
import { KeyboardAvoidingView, Modal, Platform, Pressable, ScrollView, View } from 'react-native'
import { SafeAreaView, useSafeAreaInsets } from 'react-native-safe-area-context'

import { alpha, cssColors, type, useTheme } from '../../theme'
import { Button, Icon, type IconName, Text } from '../../ui'

export interface SheetItem {
  key: string
  label: string
  icon?: IconName
  destructive?: boolean
  /** A check at the end: the current choice of a radio list. */
  selected?: boolean
  disabled?: boolean
  onPress: () => void
}

/**
 * The web's phone action sheet (`MessageActionSheet`): it rises from the bottom on the popover surface, with an optional
 * block above the rows (the quick reactions). A row closes the sheet, then runs.
 */
export function ActionSheet({ visible, title, header, items, onClose }: { visible: boolean; title?: string; header?: ReactNode; items: readonly SheetItem[]; onClose: () => void }) {
  const { colors, scheme } = useTheme()
  const insets = useSafeAreaInsets()
  return (
    <Modal visible={visible} transparent animationType="slide" onRequestClose={onClose}>
      <Pressable accessibilityLabel="Close" onPress={onClose} style={{ flex: 1, backgroundColor: alpha(scheme === 'dark' ? cssColors.black : colors.foreground, 0.4) }} />
      <View
        accessibilityViewIsModal
        style={{
          maxHeight: '80%',
          backgroundColor: colors.popover,
          borderTopLeftRadius: 12,
          borderTopRightRadius: 12,
          borderTopWidth: 1,
          borderColor: colors.border,
          padding: 8,
          paddingBottom: Math.max(8, insets.bottom),
        }}
      >
        {title ? (
          <Text numberOfLines={1} style={[type(13, 'semibold'), { paddingHorizontal: 12, paddingTop: 4, paddingBottom: 8, color: colors.mutedForeground }]}>
            {title}
          </Text>
        ) : null}
        {header}
        <ScrollView bounces={false}>
          <View style={{ paddingTop: header ? 4 : 0 }}>
            {items.map((item) => (
              <Pressable
                key={item.key}
                accessibilityRole="button"
                accessibilityLabel={item.label}
                accessibilityState={{ disabled: item.disabled, selected: item.selected }}
                disabled={item.disabled}
                onPress={() => {
                  onClose()
                  item.onPress()
                }}
                style={({ pressed }) => ({ height: 44, flexDirection: 'row', alignItems: 'center', gap: 12, paddingHorizontal: 12, borderRadius: 8, opacity: item.disabled ? 0.5 : 1, backgroundColor: pressed ? colors.muted : 'transparent' })}
              >
                {item.icon ? <Icon name={item.icon} size={20} color={item.destructive ? colors.destructive : colors.foreground} /> : null}
                <Text style={[type(14), { flex: 1, color: item.destructive ? colors.destructive : colors.popoverForeground }]}>{item.label}</Text>
                {item.selected ? <Icon name="check" size={18} color={colors.primary} /> : null}
              </Pressable>
            ))}
          </View>
        </ScrollView>
      </View>
    </Modal>
  )
}

/**
 * A dialog of the web as a phone sheet: Cancel, the title and the main action in a header, the form below. iOS shows it
 * as a page sheet that slides down to close.
 */
export function FormSheet({
  visible,
  title,
  description,
  submitLabel,
  canSubmit = true,
  busy = false,
  onSubmit,
  onClose,
  children,
}: {
  visible: boolean
  title: string
  description?: string
  /** Without it the sheet has only Done (Close). */
  submitLabel?: string
  canSubmit?: boolean
  busy?: boolean
  onSubmit?: () => void
  onClose: () => void
  children: ReactNode
}) {
  const { colors } = useTheme()
  return (
    <Modal visible={visible} animationType="slide" presentationStyle="pageSheet" onRequestClose={onClose}>
      <SafeAreaView edges={Platform.OS === 'ios' ? ['bottom', 'left', 'right'] : ['top', 'bottom', 'left', 'right']} style={{ flex: 1, backgroundColor: colors.popover }}>
        <View style={{ height: 52, flexDirection: 'row', alignItems: 'center', gap: 8, paddingHorizontal: 12, borderBottomWidth: 1, borderBottomColor: colors.border }}>
          <View style={{ flex: 1, alignItems: 'flex-start' }}>
            <Button label={submitLabel ? 'Cancel' : 'Close'} variant="ghost" onPress={onClose} />
          </View>
          <Text numberOfLines={1} accessibilityRole="header" style={[type(15, 'semibold'), { flexShrink: 1 }]}>
            {title}
          </Text>
          <View style={{ flex: 1, alignItems: 'flex-end' }}>
            {submitLabel && onSubmit ? <Button label={submitLabel} size="sm" loading={busy} disabled={!canSubmit} onPress={onSubmit} /> : null}
          </View>
        </View>
        <KeyboardAvoidingView style={{ flex: 1 }} behavior={Platform.OS === 'ios' ? 'padding' : undefined}>
          <ScrollView keyboardShouldPersistTaps="handled" contentContainerStyle={{ padding: 16, gap: 16 }}>
            {description ? <Text style={[type(14), { lineHeight: 20, color: colors.mutedForeground }]}>{description}</Text> : null}
            {children}
          </ScrollView>
        </KeyboardAvoidingView>
      </SafeAreaView>
    </Modal>
  )
}

/** A label over a row of choices, each with a line under its name: the web's Visibility toggle group. */
export function ChoiceGroup<T extends string>({ label, value, options, onChange }: { label: string; value: T; options: readonly { value: T; label: string; description: string; icon: IconName }[]; onChange: (value: T) => void }) {
  const { colors } = useTheme()
  return (
    <View style={{ gap: 8 }}>
      <Text style={[type(14, 'medium'), { lineHeight: 19.25 }]}>{label}</Text>
      <View style={{ borderRadius: 10, borderWidth: 1, borderColor: colors.input, overflow: 'hidden' }}>
        {options.map((option, index) => {
          const selected = option.value === value
          return (
            <Pressable
              key={option.value}
              accessibilityRole="radio"
              accessibilityState={{ selected }}
              accessibilityLabel={`${option.label}. ${option.description}`}
              onPress={() => onChange(option.value)}
              style={{ paddingHorizontal: 12, paddingVertical: 8, gap: 2, borderTopWidth: index === 0 ? 0 : 1, borderTopColor: colors.input, backgroundColor: selected ? colors.muted : 'transparent' }}
            >
              <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}>
                <Icon name={option.icon} size={16} color={colors.foreground} />
                <Text style={[type(14, 'medium'), { flex: 1 }]}>{option.label}</Text>
                {selected ? <Icon name="check" size={16} color={colors.primary} /> : null}
              </View>
              <Text style={[type(12), { lineHeight: 16, color: colors.mutedForeground }]}>{option.description}</Text>
            </Pressable>
          )
        })}
      </View>
    </View>
  )
}
