import { Pressable, TextInput, View } from 'react-native'
import { SafeAreaView } from 'react-native-safe-area-context'

import { useTheme } from '../theme'
import { Icon, Spinner } from '../ui'

export interface ComposerProps {
  value: string
  onChangeText: (text: string) => void
  onSend: () => void
  placeholder: string
  sending?: boolean
}

/** A growing text field with a send button, pinned to the bottom edge of the screen. */
export function Composer({ value, onChangeText, onSend, placeholder, sending = false }: ComposerProps) {
  const { colors, fonts, radius, spacing, scheme } = useTheme()
  const canSend = value.trim().length > 0 && !sending
  return (
    <SafeAreaView edges={['bottom', 'left', 'right']} style={{ borderTopWidth: 1, borderTopColor: colors.border, backgroundColor: colors.background }}>
      <View style={{ flexDirection: 'row', alignItems: 'flex-end', gap: spacing[2], padding: spacing[2], paddingLeft: spacing[3] }}>
        <TextInput
          value={value}
          onChangeText={onChangeText}
          placeholder={placeholder}
          placeholderTextColor={colors.mutedForeground}
          selectionColor={colors.primary}
          keyboardAppearance={scheme}
          multiline
          accessibilityLabel={placeholder}
          style={{
            flex: 1,
            maxHeight: 140,
            minHeight: 40,
            paddingHorizontal: spacing[3],
            paddingTop: spacing[2.5],
            paddingBottom: spacing[2.5],
            borderWidth: 1,
            borderColor: colors.input,
            borderRadius: radius.xl,
            color: colors.foreground,
            fontFamily: fonts.regular,
            fontSize: 15,
          }}
        />
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Send"
          accessibilityState={{ disabled: !canSend, busy: sending }}
          disabled={!canSend}
          onPress={onSend}
          style={{ width: 40, height: 40, borderRadius: 20, alignItems: 'center', justifyContent: 'center', backgroundColor: canSend ? colors.primary : colors.muted }}
        >
          {sending ? <Spinner color={colors.primaryForeground} /> : <Icon name="send" size={20} color={canSend ? colors.primaryForeground : colors.mutedForeground} />}
        </Pressable>
      </View>
    </SafeAreaView>
  )
}
