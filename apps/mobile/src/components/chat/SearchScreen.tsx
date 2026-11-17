import { router } from 'expo-router'
import { useEffect, useState } from 'react'
import { FlatList, Pressable, View } from 'react-native'
import { TextInput } from '../../ui/scaled'

import { conversationTitle, decodeMentions, leadToFirstMatch, type Person, splitByRanges } from '../../data/chat'
import { isSearchable } from '../../data/chat-api'
import type { MessageRecord } from '../../data/orbit'
import { parseRecentSearches, recentSearchesKey, withRecentSearch } from '../../data/recent-searches'
import { useChatSearch } from '../../hooks/chat'
import { useOrbit } from '../../orbit-context'
import { useConversations, usePeople } from '../../queries'
import { storage } from '../../storage'
import { useTheme } from '../../theme'
import { Button, EmptyState, ErrorView, Icon, LoadingView, PickerSheet, Spinner, Text } from '../../ui'
import { ChatColorScope } from './ChatColorScope'
import { useMessageText } from './MessageList'
import { ChatPane, MessagePreview } from './Pane'

/**
 * Chat search with the web's filters: in one conversation (`conversationId`, from its header) or in all of them (the
 * chat list). A result opens the message where it is; the last searches wait under the field.
 */
export function SearchScreen({ conversationId }: { conversationId?: string }) {
  return (
    <ChatColorScope surface="page">
      <SearchScreenContent id={conversationId} />
    </ChatColorScope>
  )
}

/** The last searches of this workspace and person, kept on the phone. */
function useRecentSearches() {
  const { workspaceId, user } = useOrbit()
  const key = recentSearchesKey(workspaceId, user.id)
  const [list, setList] = useState<string[]>([])
  useEffect(() => {
    let live = true
    storage.get(key).then((stored) => live && setList(parseRecentSearches(stored)), () => undefined)
    return () => {
      live = false
    }
  }, [key])
  const save = (next: string[]) => {
    setList(next)
    void storage.set(key, JSON.stringify(next)).catch(() => undefined)
  }
  return { list, add: (query: string) => save(withRecentSearch(list, query)), clear: () => save([]) }
}

function SearchScreenContent({ id }: { id: string | undefined }) {
  const { user } = useOrbit()
  const { colors, scheme } = useTheme()
  const people = usePeople()
  const conversations = useConversations().data ?? []
  const words = useMessageText()
  const [input, setInput] = useState('')
  const [query, setQuery] = useState('')
  const [thisConversation, setThisConversation] = useState(id !== undefined)
  const [authorId, setAuthorId] = useState<string | null>(null)
  const [hasFile, setHasFile] = useState(false)
  const [mentionsMe, setMentionsMe] = useState(false)
  const [hasLink, setHasLink] = useState(false)
  const [threadsOnly, setThreadsOnly] = useState(false)
  const [oldest, setOldest] = useState(false)
  const [pickAuthor, setPickAuthor] = useState(false)
  const recent = useRecentSearches()
  const conversation = conversations.find((item) => item.id === id)
  const title = conversation ? conversationTitle(conversation, people, user.id) : ''

  // search as the user types, a moment after the last key
  useEffect(() => {
    const timer = setTimeout(() => setQuery(input.trim()), 300)
    return () => clearTimeout(timer)
  }, [input])

  const filters = {
    query,
    conversationId: thisConversation ? id : undefined,
    authorId: authorId ?? undefined,
    hasFile: hasFile || undefined,
    mentionsMe: mentionsMe || undefined,
    hasLink: hasLink || undefined,
    threadsOnly: threadsOnly || undefined,
    sort: oldest ? ('oldest' as const) : undefined,
  }
  const search = useChatSearch(filters)
  const searching = isSearchable(filters)
  const hits = search.data?.pages.flatMap((page) => page.items) ?? []
  const channels: Person[] = conversations.filter((item) => item.kind !== 'dm').map((item) => ({ id: item.id, name: item.name }))

  const open = (message: MessageRecord) => {
    if (query) recent.add(query)
    if (message.thread_root_id && !message.also_in_channel) return router.push(`/chat/thread/${message.thread_root_id}`)
    if (id && message.conversation_id === id) return router.dismissTo(`/chat/${id}?message=${message.id}`)
    router.push(`/chat/${message.conversation_id}?message=${message.id}`)
  }

  const chip = (label: string, on: boolean, onPress: () => void) => (
    <Pressable
      accessibilityRole="button"
      accessibilityState={{ selected: on }}
      onPress={onPress}
      className={`h-7 flex-row items-center gap-1 px-2.5 rounded-md border ${on ? 'border-primary bg-primary/12' : 'border-input'}`}
    >
      <Text className={`font-noto-medium text-[12.8px] ${on ? 'text-foreground' : 'text-muted-foreground'}`}>{label}</Text>
    </Pressable>
  )

  return (
    <ChatPane title="Search" subtitle={thisConversation && conversation ? `in ${conversation.kind === 'dm' ? title : `#${title}`}` : 'in all conversations'}>
      <View className="px-3 gap-2 pb-2 border-b border-border">
        <View className="h-9 flex-row items-center gap-2 px-2.5 rounded-lg bg-secondary/60">
          <Icon name="search" size={16} color={colors.mutedForeground} />
          <TextInput
            autoFocus
            value={input}
            onChangeText={setInput}
            onSubmitEditing={() => {
              setQuery(input.trim())
              recent.add(input)
            }}
            returnKeyType="search"
            placeholder="Search"
            accessibilityLabel={id ? `Search in ${title}` : 'Search messages'}
            placeholderTextColor={colors.mutedForeground}
            selectionColor={colors.primary}
            keyboardAppearance={scheme}
            autoCapitalize="none"
            autoCorrect={false}
            className="font-noto text-[16px] leading-[1.45] flex-1 py-0 text-foreground"
          />
        </View>
        <View className="flex-row flex-wrap gap-1.5">
          {id ? chip('This conversation', thisConversation, () => setThisConversation(!thisConversation)) : null}
          {chip(authorId ? `From ${words.nameOf(authorId)}` : 'From…', authorId !== null, () => setPickAuthor(true))}
          {chip('Mentions me', mentionsMe, () => setMentionsMe(!mentionsMe))}
          {chip('Has link', hasLink, () => setHasLink(!hasLink))}
          {chip('Has file', hasFile, () => setHasFile(!hasFile))}
          {chip('Threads only', threadsOnly, () => setThreadsOnly(!threadsOnly))}
          {chip(oldest ? 'Oldest first' : 'Newest first', oldest, () => setOldest(!oldest))}
        </View>
      </View>
      {!searching ? (
        recent.list.length > 0 && input.trim() === '' ? (
          <View className="p-1">
            <View className="h-9 flex-row items-center justify-between px-2">
              <Text className="font-noto-medium text-[12px] text-muted-foreground">Recent searches</Text>
              <Button label="Clear" size="sm" variant="ghost" onPress={recent.clear} />
            </View>
            {recent.list.map((entry) => (
              <Pressable
                key={entry}
                accessibilityRole="button"
                accessibilityLabel={`Search for ${entry}`}
                onPress={() => {
                  setInput(entry)
                  setQuery(entry)
                }}
                className="h-10 flex-row items-center gap-2.5 px-2 rounded-md active:bg-muted"
              >
                <Icon name="history" size={16} color={colors.mutedForeground} />
                <Text numberOfLines={1} className="text-[14px] flex-1">
                  {entry}
                </Text>
              </Pressable>
            ))}
          </View>
        ) : (
          <EmptyState icon="search" title="Search messages" description="Type in the search field, or choose a filter." />
        )
      ) : search.isPending ? (
        <LoadingView />
      ) : search.isError ? (
        <ErrorView error={search.error} onRetry={() => void search.refetch()} />
      ) : hits.length === 0 ? (
        <EmptyState
          icon="search"
          title={query ? `No results for "${query}"` : 'No results'}
          description={thisConversation && id ? 'Nothing matches in this conversation.' : 'Try other words or remove a filter.'}
          action={thisConversation && id ? <Button label="Search all conversations" variant="outline" size="sm" onPress={() => setThisConversation(false)} /> : undefined}
        />
      ) : (
        <FlatList
          data={hits}
          keyExtractor={(hit) => hit.message.id}
          keyboardShouldPersistTaps="handled"
          contentContainerClassName="p-1"
          onEndReached={() => {
            if (search.hasNextPage && !search.isFetchingNextPage) void search.fetchNextPage()
          }}
          ListFooterComponent={search.isFetchingNextPage ? <View className="p-4"><Spinner /></View> : undefined}
          renderItem={({ item: hit }) => {
            const where = conversations.find((item) => item.id === hit.conversation_id)
            const segments = leadToFirstMatch(splitByRanges(hit.message.body, hit.ranges)).map((segment) => ({ ...segment, text: decodeMentions(segment.text, people, channels) }))
            return (
              <MessagePreview
                message={hit.message}
                author={words.authorOf(hit.message)}
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
