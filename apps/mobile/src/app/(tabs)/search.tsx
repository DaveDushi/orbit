import { useQuery } from '@tanstack/react-query'
import { type Href, router } from 'expo-router'
import { type ReactNode, useDeferredValue, useMemo, useState } from 'react'
import { Pressable, ScrollView, View } from 'react-native'
import { TextInput } from '../../ui/scaled'

import { conversationTitle, isVisible } from '../../data/chat'
import { pageTitle } from '../../data/docs'
import { searchNodes } from '../../data/drive'
import { keys } from '../../data/keys'
import { searchAll } from '../../data/search'
import { identifier } from '../../data/tasks'
import { useOrbit } from '../../orbit-context'
import { useConversations, usePages, usePeople, useProjects, useStatuses, useTasks } from '../../queries'
import { useTheme } from '../../theme'
import { EmptyState, Icon, type IconName, ListRow, PaneHeader, SafeAreaView, Spinner, StatusIcon, Text } from '../../ui'

/**
 * The app's one Search (the web's command palette search): tasks, docs and chats by name from the loaded lists, Drive
 * from the server, and a link to search the messages. Home, the drawer and Settings open it; a guest has none.
 */
export default function SearchScreen() {
  const { client, workspaceId, user } = useOrbit()
  const { colors } = useTheme()
  const [input, setInput] = useState('')
  const query = useDeferredValue(input.trim())
  const tasks = useTasks()
  const projects = useProjects()
  const statuses = useStatuses()
  const pages = usePages()
  const conversations = useConversations()
  const people = usePeople()
  const drive = useQuery({ queryKey: [...keys.drive(workspaceId), 'search', query], queryFn: () => searchNodes(client, workspaceId, query), enabled: query.length > 0 })

  const found = useMemo(
    () =>
      searchAll(query, {
        tasks: tasks.data ?? [],
        projects: projects.data ?? [],
        pages: pages.data ?? [],
        conversations: (conversations.data ?? []).filter((conversation) => isVisible(conversation, user.id)).map((conversation) => ({ conversation, title: conversationTitle(conversation, people, user.id) })),
      }),
    [conversations.data, pages.data, people, projects.data, query, tasks.data, user.id],
  )
  const files = (drive.data ?? []).slice(0, 8)
  const nothing = found.tasks.length + found.pages.length + found.conversations.length + files.length === 0
  const muted = colors.mutedForeground
  const close = () => (router.canGoBack() ? router.back() : router.navigate('/'))

  return (
    <SafeAreaView edges={['top', 'left', 'right']} className="flex-1 bg-background">
      <PaneHeader className="gap-2 px-3">
        <View className="h-8.5 flex-1 flex-row items-center border border-input rounded-lg bg-input/30">
          <View className="w-5.5 items-end">
            <Icon name="search" size={14} color={muted} />
          </View>
          <TextInput
            accessibilityLabel="Search"
            placeholder="Search tasks, docs, chat and Drive"
            placeholderTextColor={muted}
            selectionColor={colors.primary}
            autoFocus
            value={input}
            onChangeText={setInput}
            returnKeyType="search"
            autoCorrect={false}
            className="flex-1 h-8 pl-2 pr-2.5 font-noto text-[16px] text-foreground"
          />
        </View>
        <Pressable accessibilityRole="button" onPress={close} hitSlop={6} className="h-8 justify-center px-1">
          <Text className="font-noto-medium text-[14px] text-primary">Cancel</Text>
        </Pressable>
      </PaneHeader>
      {!query ? (
        <EmptyState icon="search" title="Search Orbit" description="Find tasks, docs, chats and Drive files by name." />
      ) : (
        <ScrollView keyboardShouldPersistTaps="handled" keyboardDismissMode="on-drag" contentContainerClassName="pb-6">
          <Group label="Tasks" show={found.tasks.length > 0}>
            {found.tasks.map((task) => (
              <ListRow
                key={task.id}
                title={task.title || 'Untitled'}
                subtitle={identifier(task, projects.data ?? [])}
                leading={<StatusIcon status={statuses.data?.find((status) => status.id === task.status_id)} size={16} />}
                onPress={() => router.push(`/tasks/${task.id}`)}
              />
            ))}
          </Group>
          <Group label="Docs" show={found.pages.length > 0}>
            {found.pages.map((page) => (
              <ListRow key={page.id} title={pageTitle(page)} leading={page.icon ? <Text className="w-4 text-center text-[14px]">{page.icon}</Text> : <Icon name="docs" size={16} color={muted} />} onPress={() => router.push(`/docs/${page.id}`)} />
            ))}
          </Group>
          <Group label="Chat" show={found.conversations.length > 0}>
            {found.conversations.map(({ conversation, title }) => (
              <ListRow
                key={conversation.id}
                title={title}
                leading={<Icon name={conversation.kind === 'dm' ? 'messages2' : conversation.kind === 'private' ? 'lock' : 'hashtag'} size={16} color={muted} />}
                onPress={() => router.push(`/chat/${conversation.id}`)}
              />
            ))}
          </Group>
          <Group label="Drive" show={files.length > 0}>
            {files.map(({ node, folder_path }) => (
              <ListRow
                key={node.id}
                title={node.name}
                subtitle={folder_path || 'Drive'}
                leading={<Icon name={node.kind === 'folder' ? 'folder' : 'note2'} size={16} color={node.kind === 'folder' ? colors.primary : muted} />}
                // a file opens in its folder, where its menu is
                onPress={() => router.push((node.kind === 'folder' ? `/drive/${node.id}` : node.parent_id ? `/drive/${node.parent_id}` : '/drive') as Href)}
              />
            ))}
          </Group>
          {nothing ? (
            <View className="items-center gap-1 px-6 pt-10 pb-4">
              {drive.isFetching ? <Spinner /> : <Text className="text-[14px] text-muted-foreground">{`No task, doc, chat or file is called “${query}”.`}</Text>}
            </View>
          ) : null}
          <View className="mt-4 border-t border-border">
            <MoreRow icon="chat" label={`Search messages for “${query}”`} onPress={() => router.push({ pathname: '/chat/search', params: { q: query } })} />
          </View>
        </ScrollView>
      )}
    </SafeAreaView>
  )
}

function Group({ label, show, children }: { label: string; show: boolean; children: ReactNode }) {
  if (!show) return null
  return (
    <View className="pt-3">
      <Text accessibilityRole="header" className="px-4 pb-1 font-noto-medium text-[12px] text-muted-foreground">
        {label}
      </Text>
      {children}
    </View>
  )
}

function MoreRow({ icon, label, onPress }: { icon: IconName; label: string; onPress: () => void }) {
  const { colors } = useTheme()
  return (
    <Pressable accessibilityRole="button" onPress={onPress} className="h-12 flex-row items-center gap-3 px-4 active:bg-accent">
      <Icon name={icon} size={16} color={colors.mutedForeground} />
      <Text numberOfLines={1} className="flex-1 text-[14px]">
        {label}
      </Text>
      <Icon name="chevronRight" size={14} color={colors.mutedForeground} />
    </Pressable>
  )
}
