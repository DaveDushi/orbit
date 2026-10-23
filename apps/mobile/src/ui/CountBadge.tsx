import { View } from 'react-native'

import { fonts, useTheme } from '../theme'
import { Text } from './Text'

/** The web's pink count pill: 16px tall, `99+` beyond 99. */
export function CountBadge({ count }: { count: number }) {
  const { colors } = useTheme()
  return (
    <View style={{ height: 16, minWidth: 16, paddingHorizontal: 4, borderRadius: 8, backgroundColor: colors.primary, alignItems: 'center', justifyContent: 'center' }}>
      <Text style={{ fontSize: 10, lineHeight: 12, fontFamily: fonts.semibold, color: colors.primaryForeground }}>{count > 99 ? '99+' : count}</Text>
    </View>
  )
}
