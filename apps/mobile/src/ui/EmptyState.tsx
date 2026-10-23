import type { ReactNode } from 'react'
import { View } from 'react-native'

import { useTheme } from '../theme'
import { Icon, type IconName } from './Icon'
import { Text } from './Text'

export interface EmptyStateProps {
  title: string
  description?: string
  icon?: IconName
  /** A button or link below the text. */
  action?: ReactNode
}

export function EmptyState({ title, description, icon, action }: EmptyStateProps) {
  const { colors, radius, spacing } = useTheme()
  return (
    <View style={{ flex: 1, alignItems: 'center', justifyContent: 'center', gap: spacing[2], padding: spacing[6] }}>
      {icon ? (
        <View style={{ width: 48, height: 48, borderRadius: radius.xl, backgroundColor: colors.muted, alignItems: 'center', justifyContent: 'center', marginBottom: spacing[2] }}>
          <Icon name={icon} size={24} color={colors.mutedForeground} />
        </View>
      ) : null}
      <Text variant="heading" style={{ textAlign: 'center' }}>
        {title}
      </Text>
      {description ? (
        <Text color="muted" style={{ textAlign: 'center', maxWidth: 300 }}>
          {description}
        </Text>
      ) : null}
      {action ? <View style={{ marginTop: spacing[2] }}>{action}</View> : null}
    </View>
  )
}
