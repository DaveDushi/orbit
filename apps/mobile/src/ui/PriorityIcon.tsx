import Svg, { Path, Rect } from 'react-native-svg'

import { useTheme } from '../theme'

const BARS: Record<string, number> = { low: 1, medium: 2, high: 3 }

/** Signal bars for low, medium and high; a filled square with `!` for urgent; nothing for no priority. */
export function PriorityIcon({ priority, size = 14 }: { priority: string; size?: number }) {
  const { colors } = useTheme()
  if (priority === 'urgent') {
    return (
      <Svg width={size} height={size} viewBox="0 0 14 14" accessibilityLabel="Urgent">
        <Rect x={1} y={1} width={12} height={12} rx={3} fill={colors.destructive} />
        <Path d="M7 4 V7.6 M7 9.8 V10" stroke={colors.primaryForeground} strokeWidth={1.6} strokeLinecap="round" />
      </Svg>
    )
  }
  const filled = BARS[priority]
  if (!filled) return null
  return (
    <Svg width={size} height={size} viewBox="0 0 14 14" accessibilityLabel={priority}>
      {[0, 1, 2].map((index) => (
        <Rect key={index} x={1.5 + index * 4} y={9 - index * 3} width={3} height={4 + index * 3} rx={1} fill={index < filled ? colors.foreground : colors.border} />
      ))}
    </Svg>
  )
}
