import type { ReactNode } from 'react'
import { ScrollView, StyleSheet, View, type ViewStyle } from 'react-native'
import { type Edge, SafeAreaView } from 'react-native-safe-area-context'

import { useTheme } from '../theme'

export interface ScreenProps {
  children: ReactNode
  /** Safe-area edges to pad. Screens under a navigator header or tab bar pass only the edges those bars leave uncovered. */
  edges?: readonly Edge[]
  scroll?: boolean
  /** Centers the content, for forms and empty states. */
  centered?: boolean
  style?: ViewStyle
}

export function Screen({ children, edges = ['top', 'bottom', 'left', 'right'], scroll, centered, style }: ScreenProps) {
  const { colors, spacing } = useTheme()
  const content = [styles.content, { padding: spacing[4], gap: spacing[3] }, centered && styles.centered, style]
  return (
    <SafeAreaView edges={edges} style={[styles.root, { backgroundColor: colors.background }]}>
      {scroll ? (
        <ScrollView contentContainerStyle={content} keyboardShouldPersistTaps="handled" contentInsetAdjustmentBehavior="automatic">
          {children}
        </ScrollView>
      ) : (
        <View style={content}>{children}</View>
      )}
    </SafeAreaView>
  )
}

const styles = StyleSheet.create({
  root: { flex: 1 },
  content: { flexGrow: 1 },
  centered: { justifyContent: 'center' },
})
