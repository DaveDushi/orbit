import { useState } from 'react'
import { Image, View } from 'react-native'

import { serverHeaders } from '../data/chat-api'
import { absoluteUrl } from '../data/docs'
import { useOrbit } from '../orbit-context'

/**
 * The web's `ProjectMark`: a project's icon image when it has one (it needs the session's bearer token), else its colour
 * dot, also when the image does not load. `className` shapes the dot, `iconSize` sizes the image (an icon needs more
 * room than a dot to be readable). Decorative: the project name always stands beside it.
 */
export function ProjectMark({
  project,
  className = 'size-2 rounded-full',
  iconSize = 16,
}: {
  project: { color: string; icon_url?: string | null }
  className?: string
  iconSize?: number
}) {
  const { session } = useOrbit()
  const [failed, setFailed] = useState<string | null>(null)
  const url = project.icon_url
  if (url && failed !== url) {
    const uri = absoluteUrl(session.server, url)
    return (
      <Image
        accessibilityIgnoresInvertColors
        accessibilityElementsHidden
        importantForAccessibility="no-hide-descendants"
        source={{ uri, headers: serverHeaders(session.server, uri, session.token) }}
        onError={() => setFailed(url)}
        style={{ width: iconSize, height: iconSize, borderRadius: 4 }}
      />
    )
  }
  return <View className={className} style={{ backgroundColor: project.color }} />
}
