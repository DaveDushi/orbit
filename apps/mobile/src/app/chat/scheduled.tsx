import { router } from 'expo-router'
import { useState } from 'react'
import { FlatList, Pressable, RefreshControl, View } from 'react-native'

import { ChatColorScope } from '../../components/chat/ChatColorScope'
import { ChatPane } from '../../components/chat/Pane'
import { SendLaterSheet } from '../../components/chat/SendLater'
import { ActionSheet, FormSheet } from '../../components/chat/Sheet'
import { useMessageText } from '../../components/chat/MessageList'
import { decodeMentions, type Person } from '../../data/chat'
import { encodeMentions } from '../../data/mentions'
import { type ScheduledMessage, scheduledWhere, whenLabel } from '../../data/scheduled'
import { confirmAction } from '../../hooks/chat'
import { useDeleteScheduled, useScheduledMessages, useUpdateScheduled } from '../../hooks/scheduled'
import { useConversations, usePeople } from '../../queries'
import { useTheme } from '../../theme'
import { EmptyState, ErrorView, Icon, Input, LoadingView, Text } from '../../ui'

/** The web's "Scheduled messages": the person's messages and reminders that wait, soonest first. A tap offers edit, a new time or cancel. */
export default function ScheduledScreen() {
  return (
    <ChatColorScope surface="page">
      <ScheduledScreenContent />
    </ChatColorScope>
  )
}

function ScheduledScreenContent() {
  const { colors } = useTheme()
  const scheduled = useScheduledMessages()
  const update = useUpdateScheduled()
  const cancel = useDeleteScheduled()
  const people = usePeople()
  const conversations = useConversations().data ?? []
  const words = useMessageText()
  const channels: Person[] = conversations.filter((item) => item.kind !== 'dm').map((item) => ({ id: item.id, name: item.name }))
  const [selected, setSelected] = useState<ScheduledMessage | null>(null)
  const [editing, setEditing] = useState<{ message: ScheduledMessage; text: string } | null>(null)
  const [retiming, setRetiming] = useState<ScheduledMessage | null>(null)
  const items = [...(scheduled.data ?? [])].sort((a, b) => a.send_at - b.send_at)

  const remove = async (message: ScheduledMessage) => {
    const reminder = message.kind === 'reminder'
    if (await confirmAction(reminder ? 'Cancel reminder?' : 'Cancel message?', reminder ? 'The reminder will not come.' : 'The message will not be sent.', reminder ? 'Cancel reminder' : 'Cancel message')) cancel.mutate(message.id)
  }

  return (
    <ChatPane title="Scheduled messages" subtitle="Your messages and reminders that wait to be sent">
      {!scheduled.data ? (
        scheduled.isError ? <ErrorView error={scheduled.error} onRetry={() => void scheduled.refetch()} /> : <LoadingView />
      ) : (
        <FlatList
          data={items}
          keyExtractor={(item) => item.id}
          contentContainerClassName={items.length === 0 ? 'grow' : 'p-1'}
          refreshControl={<RefreshControl refreshing={scheduled.isRefetching} onRefresh={() => void scheduled.refetch()} tintColor={colors.mutedForeground} />}
          ListEmptyComponent={<EmptyState icon="clock" title="Nothing is scheduled" description='Hold the Send button and choose "Send later", or ask Donna to remind you.' />}
          renderItem={({ item }) => (
            <Pressable
              accessibilityRole="button"
              accessibilityHint="Edit, change the time or cancel"
              onPress={() => setSelected(item)}
              onLongPress={() => setSelected(item)}
              className="gap-1 px-3 py-2.5 rounded-md active:bg-muted/60"
            >
              <View className="flex-row items-center gap-1.5">
                <Icon name={item.kind === 'reminder' ? 'bell' : 'clock'} size={14} color={colors.mutedForeground} />
                <Text numberOfLines={1} className="font-noto-medium text-[12px] text-foreground shrink">
                  {scheduledWhere(item)}
                </Text>
                <Text className="text-[12px] text-muted-foreground">· {whenLabel(item.send_at)}</Text>
              </View>
              {item.status === 'failed' ? <Text className="text-[12px] text-destructive">{item.error ?? 'Not sent.'}</Text> : null}
              <Text numberOfLines={3} className="text-[14px] leading-[20px] text-foreground/85">
                {words.text(item.body)}
              </Text>
            </Pressable>
          )}
        />
      )}
      <ActionSheet
        visible={selected !== null}
        title={selected ? `${scheduledWhere(selected)} · ${whenLabel(selected.send_at)}` : undefined}
        items={
          selected
            ? [
                ...(selected.kind === 'message' && selected.conversation_id ? [{ key: 'open', label: 'Open conversation', icon: 'messages2' as const, onPress: () => router.push(`/chat/${selected.conversation_id}`) }] : []),
                { key: 'edit', label: 'Edit text', icon: 'edit' as const, onPress: () => setEditing({ message: selected, text: decodeMentions(selected.body, people, conversations.map((item) => ({ id: item.id, name: item.name }))) }) },
                { key: 'time', label: selected.status === 'failed' ? 'Try again at…' : 'Change time', icon: 'clock' as const, onPress: () => setRetiming(selected) },
                { key: 'cancel', label: selected.kind === 'reminder' ? 'Cancel reminder' : 'Cancel message', icon: 'trash' as const, destructive: true, onPress: () => void remove(selected) },
              ]
            : []
        }
        onClose={() => setSelected(null)}
      />
      {editing ? (
        <FormSheet
          visible
          title="Edit message"
          submitLabel="Save"
          canSubmit={editing.text.trim() !== ''}
          onSubmit={() => {
            update.mutate({ id: editing.message.id, body: encodeMentions(editing.text.trim(), people, channels) })
            setEditing(null)
          }}
          onClose={() => setEditing(null)}
        >
          <Input autoFocus multiline value={editing.text} onChangeText={(text) => setEditing({ ...editing, text })} accessibilityLabel="Message" />
        </FormSheet>
      ) : null}
      {retiming ? (
        <SendLaterSheet
          title="Change time"
          initial={new Date(retiming.send_at)}
          onClose={() => setRetiming(null)}
          onPick={(at) => {
            update.mutate({ id: retiming.id, sendAt: at })
            setRetiming(null)
          }}
        />
      ) : null}
    </ChatPane>
  )
}
