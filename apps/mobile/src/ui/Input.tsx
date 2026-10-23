import { useState } from 'react'
import { StyleSheet, TextInput, type TextInputProps, View } from 'react-native'

import { useTheme } from '../theme'
import { Text } from './Text'

export interface InputProps extends Omit<TextInputProps, 'style'> {
  label?: string
  error?: string
}

export function Input({ label, error, onFocus, onBlur, ...props }: InputProps) {
  const { colors, fonts, radius, spacing, scheme } = useTheme()
  const [focused, setFocused] = useState(false)
  const borderColor = error ? colors.destructive : focused ? colors.ring : colors.input
  return (
    <View style={{ gap: spacing[1.5] }}>
      {label ? <Text variant="label">{label}</Text> : null}
      <TextInput
        placeholderTextColor={colors.mutedForeground}
        selectionColor={colors.primary}
        keyboardAppearance={scheme}
        accessibilityLabel={label}
        {...props}
        onFocus={(event) => {
          setFocused(true)
          onFocus?.(event)
        }}
        onBlur={(event) => {
          setFocused(false)
          onBlur?.(event)
        }}
        style={[
          styles.input,
          {
            borderColor,
            borderRadius: radius.lg,
            paddingHorizontal: spacing[3],
            color: colors.foreground,
            fontFamily: fonts.regular,
            // the web's `dark:bg-input/30`: a faint fill so the field reads on the dark canvas
            backgroundColor: scheme === 'dark' ? colors.input : 'transparent',
          },
        ]}
      />
      {error ? (
        <Text variant="caption" color="destructive">
          {error}
        </Text>
      ) : null}
    </View>
  )
}

const styles = StyleSheet.create({
  input: { minHeight: 44, borderWidth: 1, fontSize: 15 },
})
