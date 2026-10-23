import { useState } from 'react'
import { TextInput, type TextInputProps, View } from 'react-native'

import { alpha, type, useTheme } from '../theme'
import { Text } from './Text'

export interface InputProps extends Omit<TextInputProps, 'style'> {
  label?: string
  /** The pink `*` after the label. */
  required?: boolean
  /** Quiet help under the field; `error` replaces it. */
  description?: string
  error?: string
}

/** The web `Input` at phone size: a 32px field (16px text, so iOS does not zoom) under a 14px label. */
export function Input({ label, required = false, description, error, multiline, onFocus, onBlur, ...props }: InputProps) {
  const { colors, scheme } = useTheme()
  const [focused, setFocused] = useState(false)
  return (
    <View style={{ gap: 8 }}>
      {label ? (
        <Text style={[type(14, 'medium'), { lineHeight: 19.25 }]}>
          {label}
          {required ? <Text style={[type(14, 'semibold'), { lineHeight: 19.25, color: colors.primary }]}>{'  *'}</Text> : null}
        </Text>
      ) : null}
      <TextInput
        placeholderTextColor={colors.mutedForeground}
        selectionColor={colors.primary}
        keyboardAppearance={scheme}
        accessibilityLabel={label}
        multiline={multiline}
        {...props}
        onFocus={(event) => {
          setFocused(true)
          onFocus?.(event)
        }}
        onBlur={(event) => {
          setFocused(false)
          onBlur?.(event)
        }}
        style={{
          minHeight: multiline ? 76 : 32,
          paddingHorizontal: 10,
          paddingVertical: multiline ? 6 : 3,
          borderRadius: 10,
          borderWidth: 1,
          borderColor: error ? colors.destructive : focused ? colors.ring : colors.input,
          // the web's `dark:bg-input/30`: a faint fill so the field reads on the dark canvas
          backgroundColor: scheme === 'dark' ? alpha(colors.input, 0.3) : 'transparent',
          ...type(16),
          lineHeight: 24,
          textAlignVertical: multiline ? 'top' : 'center',
          color: colors.foreground,
        }}
      />
      {error || description ? <Text style={[type(14), { lineHeight: 21, color: error ? colors.destructive : colors.mutedForeground }]}>{error ?? description}</Text> : null}
    </View>
  )
}
