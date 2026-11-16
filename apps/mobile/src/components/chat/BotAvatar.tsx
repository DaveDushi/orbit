import { useState } from 'react'
import { Image, View } from 'react-native'

import { emojiNamed } from '../../data/chat'
import type { MessageRecord } from '../../data/orbit'
import { useCustomEmoji, useEmojiGroups } from '../../hooks/chat'
import { useTheme } from '../../theme'
import { Icon, Text } from '../../ui'
import { Emoji } from './EmojiPicker'

type Bot = NonNullable<MessageRecord['bot']>

/** The emoji of a name, as the web: a custom emoji of the workspace (`:name:`), else a standard one. */
function useNamedEmoji(name: string | null | undefined): string | null {
  const custom = useCustomEmoji()
  const groups = useEmojiGroups()
  if (!name) return null
  return custom.has(name) ? `:${name}:` : emojiNamed(groups, name)
}

/**
 * The web `BotAvatar`: an integration's emoji (`icon_emoji`) or icon, else a chip on the muted surface. The icon is an
 * https image of another host, so it loads without the session's token.
 */
export function BotAvatar({ bot, size }: { bot: Pick<Bot, 'name' | 'icon_url'> & { icon_emoji?: string | null }; size: number }) {
  const { colors } = useTheme()
  const emoji = useNamedEmoji(bot.icon_emoji)
  const [failed, setFailed] = useState<string | null>(null)
  const url = bot.icon_url?.startsWith('https://') ? bot.icon_url : null
  if (url && !emoji && failed !== url) {
    return <Image accessibilityLabel={bot.name} source={{ uri: url }} onError={() => setFailed(url)} className="rounded-full bg-muted" style={{ width: size, height: size }} />
  }
  return (
    <View accessibilityLabel={bot.name} className="items-center justify-center rounded-full bg-muted" style={{ width: size, height: size }}>
      {emoji ? <Emoji value={emoji} size={Math.round(size * 0.62)} /> : <Icon name="cpu" size={Math.round(size * 0.55)} color={colors.mutedForeground} />}
    </View>
  )
}

/** "APP" after an integration's name, as on the web. */
export function BotBadge() {
  return <Text className="shrink-0 rounded-sm bg-muted px-1 font-noto-semibold text-[10px] leading-4 tracking-wide text-muted-foreground">APP</Text>
}
