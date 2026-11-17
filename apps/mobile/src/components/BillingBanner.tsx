import { useQuery } from '@tanstack/react-query'
import { router } from 'expo-router'
import { useState } from 'react'
import { Pressable, View } from 'react-native'

import { billingBanner, billingKey, workspaceBilling } from '../data/billing'
import { useAuthOptions } from '../hooks/account'
import { useOrbit } from '../orbit-context'
import { useTheme } from '../theme'
import { Icon, Text } from '../ui'

/**
 * The web's `ReadOnlyBanner` (Orbit Cloud): the trial's days left, or the read-only line, above the dock on every tab.
 * The action opens the app's Billing page; plans are chosen there in the web app.
 */
export function BillingBanner() {
  const { client, workspaceId } = useOrbit()
  const { colors } = useTheme()
  const enabled = useAuthOptions().data?.billing_enabled ?? false
  const billing = useQuery({ queryKey: billingKey(workspaceId), queryFn: () => workspaceBilling(client, workspaceId), enabled })
  const [openedAt] = useState(Date.now)
  const banner = billingBanner(billing.data, openedAt)
  if (!banner) return null
  return (
    <View
      accessibilityRole="summary"
      className={`min-h-8 flex-row items-center justify-center gap-2 border-t border-border px-3 py-1 ${banner.kind === 'read_only' ? 'bg-muted' : 'bg-background'}`}
    >
      {banner.kind === 'read_only' ? <Icon name="lock" size={14} color={colors.mutedForeground} /> : null}
      <Text numberOfLines={1} className="shrink text-[12px] text-muted-foreground">
        {banner.text}
      </Text>
      <Pressable accessibilityRole="link" hitSlop={8} onPress={() => router.navigate('/settings/billing')}>
        <Text className="font-noto-medium text-[12px] text-foreground">{banner.action}</Text>
      </Pressable>
    </View>
  )
}
