import { useState } from 'react'
import { Image, View } from 'react-native'

import { absoluteUrl } from '../data/docs'
import { useOrbit } from '../orbit-context'
import { Text } from '../ui'

/**
 * The web's `WorkspaceLogo`: the workspace's logo (it needs the session's bearer token), or its first letter on a
 * muted tile while it has none or the image does not load. Decorative: the workspace name always stands beside it.
 */
export function WorkspaceLogo({ workspace, size = 20 }: { workspace: { name: string; logo_url?: string | null }; size?: number }) {
  const { session } = useOrbit()
  const [failed, setFailed] = useState<string | null>(null)
  const radius = size >= 40 ? 12 : 6
  const url = workspace.logo_url
  if (url && failed !== url) {
    return (
      <Image
        accessibilityIgnoresInvertColors
        accessibilityElementsHidden
        importantForAccessibility="no-hide-descendants"
        source={{ uri: absoluteUrl(session.server, url), headers: { Authorization: `Bearer ${session.token}` } }}
        onError={() => setFailed(url)}
        style={{ width: size, height: size, borderRadius: radius }}
      />
    )
  }
  return (
    <View accessibilityElementsHidden importantForAccessibility="no-hide-descendants" className="items-center justify-center bg-muted" style={{ width: size, height: size, borderRadius: radius }}>
      <Text className="font-noto-semibold text-muted-foreground" style={{ fontSize: Math.max(10, Math.round(size * 0.55)), lineHeight: size }}>
        {workspace.name.charAt(0).toUpperCase()}
      </Text>
    </View>
  )
}
