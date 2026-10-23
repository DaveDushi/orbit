import { router, useLocalSearchParams } from 'expo-router'
import { useEffect, useState } from 'react'
import { FlatList, Pressable, TextInput, View } from 'react-native'

import { useMessageText } from '../../../components/chat/MessageList'
import { ChatPane, MessagePreview } from '../../../components/chat/Pane'
import { conversationTitle, decodeMentions, leadToFirstMatch, type Person, splitByRanges } from '../../../data/chat'
import type { MessageRecord } from '../../../data/orbit'
import { useChatSearch } from '../../../hooks/chat'
import { useOrbit } from '../../../orbit-context'
import { useConversations, usePeople } from '../../../queries'
import { alpha, type, useTheme } from '../../../theme'
import { Button, EmptyState, ErrorView, Icon, LoadingView, PickerSheet, Spinner, Text } from '../../../ui'

/** Search in a conversation (or all of them) with the web's filters. A result opens the message where it is. */
export default function SearchScreen() {
  const { id } = useLocalSearchParams<{ id: string }>()
  const { user } = useOrbit()
  const { colors, scheme } = useTheme()
  const people = usePeople()
  const conversations = useConversations().data ?? []
  const words = useMessageText()
  const [input, setInput] = useState('')
  const [query, setQuery] = useState('')
  const [thisConversation, setThisConversation] = useState(true)
  const [authorId, setAuthorId] = useState<string | null>(null)
  const [hasFile, setHasFile] = useState(false)
  const [pickAuthor, setPickAuthor] = useState(false)
  const conversation = conversations.find((item) => item.id === id)
  const title = conversation ? conversationTitle(conversation, people, user.id) : ''

  // search as the user types, a moment after the last key
  useEffect(() => {
    const timer = setTimeout(() => setQuery(input.trim()), 300)
    return () => clearTimeout(timer)
  }, [input])

  const search = useChatSearch({ query, conversationId: thisConversation ? id : undefined, authorId: authorId ?? undefined, hasFile: hasFile || undefined })
  const searching = query !== '' || authorId !== null || hasFile
  const hits = search.data?.pages.flatMap((page) => page.items) ?? []
  const channels: Person[] = conversations.filter((item) => item.kind !== 'dm').map((item) => ({ id: item.id, name: item.name }))

  const open = (message: MessageRecord) => {
    if (message.thread_root_id && !message.also_in_channel) return router.push(`/chat/thread/${message.thread_root_id}`)
    if (message.conversation_id === id) return router.dismissTo(`/chat/${id}?message=${message.id}`)
    router.push(`/chat/${message.conversation_id}?message=${message.id}`)
  }

  const chip = (label: string, on: boolean, onPress: () => void) => (
    <Pressable
      accessibilityRole="button"
      accessibilityState={{ selected: on }}
      onPress={onPress}
      style={{ height: 28, flexDirection: 'row', alignItems: 'center', gap: 4, paddingHorizontal: 10, borderRadius: 8, borderWidth: 1, borderColor: on ? colors.primary : colors.input, backgroundColor: on ? alpha(colors.primary, 0.12) : 'transparent' }}
    >
      <Text style={[type(12.8, 'medium'), { color: on ? colors.foreground : colors.mutedForeground }]}>{label}</Text>
    </Pressable>
  )

  return (
    <ChatPane title="Search" subtitle={thisConversation && conversation ? `in ${conversation.kind === 'dm' ? title : `#${title}`}` : 'in all conversations'}>
      <View style={{ paddingHorizontal: 12, gap: 8, paddingBottom: 8, borderBottomWidth: 1, borderBottomColor: colors.border }}>
        <View style={{ height: 36, flexDirection: 'row', alignItems: 'center', gap: 8, paddingHorizontal: 10, borderRadius: 10, backgroundColor: alpha(colors.secondary, 0.6) }}>
          <Icon name="search" size={16} color={colors.mutedForeground} />
          <TextInput
            autoFocus
            value={input}
            onChangeText={setInput}
            onSubmitEditing={() => setQuery(input.trim())}
            returnKeyType="search"
            placeholder="Search"
            accessibilityLabel={`Search in ${title}`}
            placeholderTextColor={colors.mutedForeground}
            selectionColor={colors.primary}
            keyboardAppearance={scheme}
            autoCapitalize="none"
            autoCorrect={false}
            style={[type(16), { flex: 1, paddingVertical: 0, color: colors.foreground }]}
          />
        </View>
        <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 6 }}>
          {chip('This conversation', thisConversation, () => setThisConversation(!thisConversation))}
          {chip(authorId ? `From ${words.nameOf(authorId)}` : 'From…', authorId !== null, () => setPickAuthor(true))}
          {chip('Has file', hasFile, () => setHasFile(!hasFile))}
        </View>
      </View>
      {!searching ? (
        <EmptyState icon="search" title="Search messages" description="Type in the search field, or choose a filter." />
      ) : search.isPending ? (
        <LoadingView />
      ) : search.isError ? (
        <ErrorView error={search.error} onRetry={() => void search.refetch()} />
      ) : hits.length === 0 ? (
        <EmptyState
          icon="search"
          title={query ? `No results for "${query}"` : 'No results'}
          description={thisConversation ? 'Nothing matches in this conversation.' : 'Try other words or remove a filter.'}
          action={thisConversation ? <Button label="Search all conversations" variant="outline" size="sm" onPress={() => setThisConversation(false)} /> : undefined}
        />
      ) : (
        <FlatList
          data={hits}
          keyExtractor={(hit) => hit.message.id}
          keyboardShouldPersistTaps="handled"
          contentContainerStyle={{ padding: 4 }}
          onEndReached={() => {
            if (search.hasNextPage && !search.isFetchingNextPage) void search.fetchNextPage()
          }}
          ListFooterComponent={search.isFetchingNextPage ? <View style={{ padding: 16 }}><Spinner /></View> : undefined}
          renderItem={({ item: hit }) => {
            const where = conversations.find((item) => item.id === hit.conversation_id)
            const segments = leadToFirstMatch(splitByRanges(hit.message.body, hit.ranges)).map((segment) => ({ ...segment, text: decodeMentions(segment.text, people, channels) }))
            return (
              <MessagePreview
                message={hit.message}
                author={words.nameOf(hit.message.author_id)}
                text=""
                segments={hit.message.body ? segments : undefined}
                context={!thisConversation && where ? `in ${where.kind === 'dm' ? '' : '#'}${conversationTitle(where, people, user.id)}` : undefined}
                onPress={() => open(hit.message)}
              />
            )
          }}
        />
      )}
      <PickerSheet
        visible={pickAuthor}
        title="From"
        options={[{ key: '', label: 'Anyone', selected: authorId === null }, ...[...people].sort((a, b) => a.name.localeCompare(b.name)).map((person) => ({ key: person.id, label: person.name, selected: person.id === authorId }))]}
        onSelect={(key) => {
          setAuthorId(key || null)
          setPickAuthor(false)
        }}
        onClose={() => setPickAuthor(false)}
      />
    </ChatPane>
  )
}
