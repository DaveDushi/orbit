import { onlineManager } from '@tanstack/react-query'
import { useEffect, useState, useSyncExternalStore } from 'react'
import { View } from 'react-native'

import type { LiveStatus } from '../live'
import { useTheme } from '../theme'
import { Icon, Text } from '../ui'

/**
 * The web's `ConnectionBanner`: a small pill above the dock while the live connection is down for more than 2 seconds
 * (a blip does not show). The screens keep their last data meanwhile, and refresh when it is back.
 */
export function ConnectionBanner({ status }: { status: LiveStatus }) {
  const { colors } = useTheme()
  const online = useSyncExternalStore(onlineManager.subscribe.bind(onlineManager), () => onlineManager.isOnline())
  const [lost, setLost] = useState(false)
  useEffect(() => {
    if (status === 'live') return setLost(false)
    const timer = setTimeout(() => setLost(true), 2000)
    return () => clearTimeout(timer)
  }, [status])
  if (online && !lost) return null
  return (
    <View accessibilityRole="text" accessibilityLiveRegion="polite" className="items-center py-1">
      <View className="flex-row items-center gap-1.5 rounded-full border border-border bg-popover px-2.5 py-0.5">
        <Icon name="wifiOff" size={12} color={colors.mutedForeground} />
        <Text className="text-[11px] text-muted-foreground">{online ? 'Reconnecting…' : 'Offline · showing saved data'}</Text>
      </View>
    </View>
  )
}
