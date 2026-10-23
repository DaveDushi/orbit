import { router } from 'expo-router'
import type { ReactNode } from 'react'
import { Pressable, View } from 'react-native'
import { SafeAreaView } from 'react-native-safe-area-context'

import type { TextSegment } from '../../data/chat'
import { shortDate, timeOfDay } from '../../data/format'
import type { ConversationRecord, MessageRecord } from '../../data/orbit'
import { alpha, type, useTheme } from '../../theme'
import { Avatar, Icon, IconButton, Text } from '../../ui'

/** A chat screen with the web's 48px pane header: Back, the title, and actions at the end. */
export function ChatPane({ title, subtitle, actions, children }: { title: string; subtitle?: string; actions?: ReactNode; children: ReactNode }) {
  const { colors } = useTheme()
  return (
    <SafeAreaView edges={['top', 'left', 'right']} style={{ flex: 1, backgroundColor: colors.card }}>
      <View style={{ height: 48, flexDirection: 'row', alignItems: 'center', gap: 4, paddingHorizontal: 8 }}>
        <IconButton icon="back" label="Back" size={32} iconSize={20} color={colors.mutedForeground} onPress={() => (router.canGoBack() ? router.back() : router.navigate('/chat'))} />
        <View style={{ flex: 1, minWidth: 0 }}>
          <Text accessibilityRole="header" numberOfLines={1} style={[type(14, 'semibold'), { lineHeight: 18 }]}>
            {title}
          </Text>
          {subtitle ? (
            <Text numberOfLines={1} style={[type(12), { lineHeight: 16, color: colors.mutedForeground }]}>
              {subtitle}
            </Text>
          ) : null}
        </View>
        {actions}
      </View>
      {children}
    </SafeAreaView>
  )
}

export function ConversationGlyph({ conversation, title, userId, size = 16 }: { conversation: ConversationRecord; title: string; userId: string; size?: number }) {
  const { colors } = useTheme()
  if (conversation.kind === 'dm') return <Avatar id={conversation.member_ids.find((id) => id !== userId) ?? userId} name={title} size={size + 2} />
  return <Icon name={conversation.kind === 'private' ? 'lock' : 'hashtag'} size={size} color={colors.mutedForeground} />
}

/**
 * The web `MessagePreview`: avatar, author, time and the text (clamped), for Unreads, Threads, search and the pinned
 * and file lists. `segments` marks search matches.
 */
export function MessagePreview({
  message,
  author,
  text,
  segments,
  context,
  clamp = 4,
  onPress,
}: {
  message: MessageRecord
  author: string
  text: string
  segments?: readonly TextSegment[]
  /** "in #general" for a search over every conversation. */
  context?: string
  clamp?: number
  onPress?: () => void
}) {
  const { colors } = useTheme()
  const files = message.attachments.length
  return (
    <Pressable
      accessibilityRole={onPress ? 'button' : undefined}
      disabled={!onPress}
      onPress={onPress}
      style={({ pressed }) => ({ flexDirection: 'row', gap: 8, padding: 8, borderRadius: 8, backgroundColor: pressed ? alpha(colors.muted, 0.6) : 'transparent' })}
    >
      <Avatar id={message.author_id} name={author} size={24} />
      <View style={{ flex: 1, minWidth: 0, gap: 2 }}>
        <View style={{ flexDirection: 'row', alignItems: 'baseline', gap: 6 }}>
          <Text numberOfLines={1} style={[type(13, 'medium'), { flexShrink: 1 }]}>
            {author}
          </Text>
          <Text style={[type(11), { color: colors.mutedForeground }]}>
            {shortDate(message.created_at)} {timeOfDay(message.created_at)}
          </Text>
          {context ? (
            <Text numberOfLines={1} style={[type(11), { flexShrink: 1, color: colors.mutedForeground }]}>
              {context}
            </Text>
          ) : null}
        </View>
        {text || segments ? (
          <Text numberOfLines={clamp} style={[type(14), { lineHeight: 20, color: alpha(colors.foreground, 0.85) }]}>
            {segments
              ? segments.map((segment, index) =>
                  segment.match ? (
                    <Text key={index} style={{ backgroundColor: alpha(colors.primary, 0.2), color: colors.foreground }}>
                      {segment.text}
                    </Text>
                  ) : (
                    segment.text
                  ),
                )
              : text}
          </Text>
        ) : null}
        {files > 0 ? (
          <View style={{ flexDirection: 'row', alignItems: 'center', gap: 4 }}>
            <Icon name="attach" size={13} color={colors.mutedForeground} />
            <Text numberOfLines={1} style={[type(12), { flexShrink: 1, color: colors.mutedForeground }]}>
              {files === 1 ? message.attachments[0]!.file_name : `${files} files`}
            </Text>
          </View>
        ) : null}
      </View>
    </Pressable>
  )
}
