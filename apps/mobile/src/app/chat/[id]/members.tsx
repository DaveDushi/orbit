import { router, useLocalSearchParams } from 'expo-router'
import { useState } from 'react'
import { Pressable, ScrollView, View } from 'react-native'

import { AddPeopleSheet } from '../../../components/chat/ChannelSheets'
import { ChatPane } from '../../../components/chat/Pane'
import { conversationPermissions, conversationTitle } from '../../../data/chat'
import * as chat from '../../../data/chat-api'
import { confirmAction, useChatWrite, useIsChatAdmin } from '../../../hooks/chat'
import { useOrbit } from '../../../orbit-context'
import { useConversations, useMembers, usePeople } from '../../../queries'
import { alpha, presenceColors, type, useTheme } from '../../../theme'
import { Avatar, Button, ErrorView, Icon, IconButton, LoadingView, Text } from '../../../ui'

/**
 * The people in a conversation, by name. A tap opens the direct message with them; those who manage a private channel
 * add and remove people.
 */
export default function MembersScreen() {
  const { id } = useLocalSearchParams<{ id: string }>()
  const { user } = useOrbit()
  const { colors } = useTheme()
  const conversations = useConversations()
  const members = useMembers()
  const people = usePeople()
  const admin = useIsChatAdmin()
  const run = useChatWrite()
  const [adding, setAdding] = useState(false)
  const conversation = conversations.data?.find((item) => item.id === id)
  const permissions = conversation ? conversationPermissions(conversation, user.id, admin) : null
  const inIt = (conversation?.member_ids ?? [])
    .flatMap((userId) => members.data?.find((member) => member.user_id === userId) ?? [])
    .sort((a, b) => a.display_name.localeCompare(b.display_name))

  const message = async (userId: string) => {
    const dm = await run('Could not open the conversation', (client, workspaceId) => chat.openDm(client, workspaceId, userId === user.id ? [] : [userId]))
    if (dm) router.push(`/chat/${dm.id}`)
  }
  const remove = async (userId: string, name: string) => {
    if (!conversation || !(await confirmAction(`Remove ${name}?`, `${name} leaves #${conversation.name} and needs an invitation to come back.`, 'Remove'))) return
    await run(`Could not remove ${name}`, (client, workspaceId) => chat.removeMember(client, workspaceId, conversation.id, userId))
  }

  return (
    <ChatPane
      title="Members"
      subtitle={conversation ? (conversation.kind === 'dm' ? conversationTitle(conversation, people, user.id) : `#${conversation.name}`) : undefined}
      actions={permissions?.canAddPeople ? <IconButton icon="userAdd" label="Add people" size={32} iconSize={20} color={colors.mutedForeground} onPress={() => setAdding(true)} /> : undefined}
    >
      {conversations.isError || members.isError ? (
        <ErrorView error={conversations.error ?? members.error} onRetry={() => void Promise.all([conversations.refetch(), members.refetch()])} />
      ) : !conversation || !members.data ? (
        <LoadingView />
      ) : (
        <ScrollView contentContainerStyle={{ paddingHorizontal: 8, paddingBottom: 16 }}>
          <Text style={[type(12, 'semibold'), { paddingHorizontal: 8, paddingTop: 8, paddingBottom: 4, color: colors.mutedForeground }]}>Members, {inIt.length}</Text>
          {inIt.map((member) => (
            <View key={member.user_id} style={{ flexDirection: 'row', alignItems: 'center', gap: 4 }}>
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={member.user_id === user.id ? 'Message yourself' : `Message ${member.display_name}`}
                onPress={() => void message(member.user_id)}
                style={({ pressed }) => ({ flex: 1, height: 48, flexDirection: 'row', alignItems: 'center', gap: 12, paddingHorizontal: 8, borderRadius: 8, backgroundColor: pressed ? alpha(colors.muted, 0.8) : 'transparent' })}
              >
                <Avatar id={member.user_id} name={member.display_name} size={32} />
                <Text numberOfLines={1} style={[type(15, 'medium'), { flexShrink: 1 }]}>
                  {member.display_name}
                  {member.user_id === user.id ? ' (you)' : ''}
                </Text>
                {/* the web's amber-500 crown, the same colour as the idle mark */}
                {member.role === 'owner' ? <Icon name="crown" size={14} color={presenceColors.idle} /> : null}
              </Pressable>
              {permissions?.canRemovePeople && member.user_id !== user.id ? <Button label="Remove" variant="ghost" size="sm" onPress={() => void remove(member.user_id, member.display_name)} /> : null}
            </View>
          ))}
        </ScrollView>
      )}
      {adding && conversation ? <AddPeopleSheet conversation={conversation} onClose={() => setAdding(false)} /> : null}
    </ChatPane>
  )
}
