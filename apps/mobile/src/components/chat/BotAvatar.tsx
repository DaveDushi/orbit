import { useState } from 'react'
import { Image, View } from 'react-native'

import type { MessageRecord } from '../../data/orbit'
import { useTheme } from '../../theme'
import { Icon, Text } from '../../ui'

type Bot = NonNullable<MessageRecord['bot']>

/**
 * The web `BotAvatar`: an integration's icon, else a chip on the muted surface. The icon is an https image of another
 * host, so it loads without the session's token. Icon emoji (`icon_emoji`) are not drawn yet: the chip shows.
 */
export function BotAvatar({ bot, size }: { bot: Pick<Bot, 'name' | 'icon_url'>; size: number }) {
  const { colors } = useTheme()
  const [failed, setFailed] = useState<string | null>(null)
  const url = bot.icon_url?.startsWith('https://') ? bot.icon_url : null
  if (url && failed !== url) {
    return <Image accessibilityLabel={bot.name} source={{ uri: url }} onError={() => setFailed(url)} className="rounded-full bg-muted" style={{ width: size, height: size }} />
  }
  return (
    <View accessibilityLabel={bot.name} className="items-center justify-center rounded-full bg-muted" style={{ width: size, height: size }}>
      <Icon name="cpu" size={Math.round(size * 0.55)} color={colors.mutedForeground} />
    </View>
  )
}

/** "APP" after an integration's name, as on the web. */
export function BotBadge() {
  return <Text className="shrink-0 rounded-sm bg-muted px-1 font-noto-semibold text-[10px] leading-4 tracking-wide text-muted-foreground">APP</Text>
}
