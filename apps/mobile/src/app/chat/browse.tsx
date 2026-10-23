import { router } from 'expo-router'
import { useState } from 'react'
import { Pressable, ScrollView, View } from 'react-native'

import { ChatPane } from '../../components/chat/Pane'
import * as chat from '../../data/chat-api'
import { useChatWrite } from '../../hooks/chat'
import { useOrbit } from '../../orbit-context'
import { useConversations } from '../../queries'
import { alpha, type, useTheme } from '../../theme'
import { Button, EmptyState, Icon, Input, LoadingView, Text } from '../../ui'

/** Browse channels: the public channels you are not in, to open or join. Joined ones are in the chat list. */
export default function BrowseChannelsScreen() {
  const { user } = useOrbit()
  const { colors } = useTheme()
  const conversations = useConversations()
  const run = useChatWrite()
  const [query, setQuery] = useState('')
  const [joining, setJoining] = useState<string | null>(null)
  const text = query.trim().toLowerCase().replace(/^#/, '')
  const channels = (conversations.data ?? [])
    .filter((item) => item.kind === 'public' && !item.archived && !item.member_ids.includes(user.id))
    .filter((item) => !text || item.name.includes(text) || item.topic.toLowerCase().includes(text))
    .sort((a, b) => a.name.localeCompare(b.name))

  const join = async (id: string) => {
    setJoining(id)
    const joined = await run('Could not join the channel', (client, workspaceId) => chat.joinChannel(client, workspaceId, id))
    setJoining(null)
    if (joined) router.replace(`/chat/${id}`)
  }

  return (
    <ChatPane title="Browse channels">
      <View style={{ paddingHorizontal: 12, paddingBottom: 8 }}>
        <Input placeholder="Search channels" value={query} onChangeText={setQuery} autoCapitalize="none" autoCorrect={false} accessibilityLabel="Search channels" />
      </View>
      {!conversations.data ? (
        <LoadingView />
      ) : channels.length === 0 ? (
        <EmptyState icon="hashtag" title={text ? 'No channels found' : 'You are in every channel'} description={text ? 'Try another name.' : 'New public channels show here.'} />
      ) : (
        <ScrollView keyboardShouldPersistTaps="handled" contentContainerStyle={{ paddingHorizontal: 8 }}>
          {channels.map((channel) => (
            <View key={channel.id} style={{ flexDirection: 'row', alignItems: 'center', gap: 8, borderRadius: 8 }}>
              <Pressable
                accessibilityRole="link"
                accessibilityLabel={`Open #${channel.name}`}
                onPress={() => router.push(`/chat/${channel.id}`)}
                style={({ pressed }) => ({ flex: 1, flexDirection: 'row', alignItems: 'center', gap: 8, padding: 8, borderRadius: 8, backgroundColor: pressed ? alpha(colors.muted, 0.6) : 'transparent' })}
              >
                <Icon name="hashtag" size={16} color={colors.mutedForeground} />
                <View style={{ flex: 1, minWidth: 0 }}>
                  <Text numberOfLines={1} style={[type(14, 'medium')]}>
                    {channel.name}
                  </Text>
                  <Text numberOfLines={1} style={[type(12), { color: colors.mutedForeground }]}>
                    {channel.topic || `${channel.member_ids.length} ${channel.member_ids.length === 1 ? 'member' : 'members'}`}
                  </Text>
                </View>
              </Pressable>
              <Button label="Join" size="sm" variant="outline" loading={joining === channel.id} onPress={() => void join(channel.id)} />
            </View>
          ))}
        </ScrollView>
      )}
    </ChatPane>
  )
}
