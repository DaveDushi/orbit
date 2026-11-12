/**
 * Ask Donna on the phone (the web's Ask bar and panel): a bar above the dock on every tab but Chat opens a sheet with
 * the person's DM with the agent, with Chat's own list, cards, approvals and composer. Each message tells the agent
 * which page the person is on. The DM is the same one as in Chat.
 */
import { useQueryClient } from '@tanstack/react-query'
import { router, usePathname } from 'expo-router'
import { useEffect, useMemo, useState, useSyncExternalStore } from 'react'
import { KeyboardAvoidingView, Modal, Platform, Pressable, View } from 'react-native'

import { applyChatEvents } from '../chat-cache'
import { askDonnaAgent, type AskDonnaAgent, clearPageItem, currentPageItem, type PageItemKind, pageContextFor, setPageItem, subscribePageItem } from '../data/ask-donna'
import * as chat from '../data/chat-api'
import { buildMessageRows } from '../data/chat'
import { useChatWrite, useMessages } from '../hooks/chat'
import { useAgentStatus } from '../hooks/agent'
import { useIsGuest, useOrbit } from '../orbit-context'
import { useTheme } from '../theme'
import { ErrorView, Icon, IconButton, LoadingView, SafeAreaView, Text } from '../ui'
import { ChatColorScope } from './chat/ChatColorScope'
import { Composer, useComposer } from './chat/Composer'
import { type ListRow, MessageList, useMessageSheet } from './chat/MessageList'

/** A screen names its open task or Docs page while it shows it; `null` (or no title yet) names none. */
export function usePublishPageItem(kind: PageItemKind, id: string | null | undefined, title: string | null | undefined) {
  useEffect(() => {
    if (!id || title == null) return
    const item = { kind, id, title }
    setPageItem(item)
    return () => clearPageItem(item)
  }, [kind, id, title])
}

/** The agent the signed-in person can ask here, or `null` (guest, no agent, ChatGPT not connected). */
function useAskDonnaAgent(): AskDonnaAgent | null {
  const guest = useIsGuest()
  const status = useAgentStatus(!guest).data
  return askDonnaAgent(status, guest)
}

/** The "Ask Donna…" bar above the dock; nothing in Chat or when the person cannot ask the agent. */
export function AskDonnaBar() {
  const agent = useAskDonnaAgent()
  const pathname = usePathname()
  const { colors } = useTheme()
  const run = useChatWrite()
  const [conversationId, setConversationId] = useState<string | null>(null)
  const [open, setOpen] = useState(false)
  const [busy, setBusy] = useState(false)
  const { workspaceId } = useOrbit()

  // another workspace has another DM; losing the connection closes the sheet
  useEffect(() => setConversationId(null), [workspaceId])
  useEffect(() => {
    if (!agent) setOpen(false)
  }, [agent])

  if (!agent || !pageContextFor(pathname, null)) return null

  const ask = async () => {
    if (busy) return
    if (conversationId) return setOpen(true)
    setBusy(true)
    const dm = await run(`Could not open your conversation with ${agent.name}`, (client, workspaceId) => chat.openDm(client, workspaceId, [agent.botId]))
    setBusy(false)
    if (!dm) return
    setConversationId(dm.id)
    setOpen(true)
  }

  return (
    <>
      <View className="px-4 pt-2 bg-background">
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={`Ask ${agent.name}`}
          onPress={() => void ask()}
          className="h-10 flex-row items-center gap-2 px-3 rounded-full border border-border bg-popover active:bg-muted"
        >
          <Icon name="magicStar" size={18} color={colors.primary} />
          <Text className="flex-1 text-[14px] text-muted-foreground">{busy ? 'Opening…' : `Ask ${agent.name}…`}</Text>
        </Pressable>
      </View>
      {conversationId ? (
        <Modal visible={open} animationType="slide" presentationStyle="pageSheet" onRequestClose={() => setOpen(false)}>
          <ChatColorScope surface="page">
            <AskDonnaSheet agent={agent} conversationId={conversationId} pathname={pathname} onClose={() => setOpen(false)} />
          </ChatColorScope>
        </Modal>
      ) : null}
    </>
  )
}

function AskDonnaSheet({ agent, conversationId, pathname, onClose }: { agent: AskDonnaAgent; conversationId: string; pathname: string; onClose: () => void }) {
  const { colors } = useTheme()
  const item = useSyncExternalStore(subscribePageItem, currentPageItem)
  const messages = useMessages(conversationId, undefined)
  // read at send time: the page the person opened the sheet from
  const composer = useComposer(conversationId, null, () => pageContextFor(pathname, item))
  const loaded = useMemo(() => messages.data?.pages.flatMap((page) => page.items) ?? [], [messages.data])
  const { sheet, handlers } = useMessageSheet({ inThread: false, composer, loaded })
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  // the sheet reads the agent's answers as they come, like the open conversation in Chat
  const newestId = messages.data && !messages.hasNextPage ? loaded.at(-1)?.id : undefined
  useEffect(() => {
    if (!newestId) return
    chat.markRead(client, workspaceId, conversationId).then(
      ({ events }) => applyChatEvents(queryClient, workspaceId, events),
      () => undefined,
    )
  }, [client, conversationId, newestId, queryClient, workspaceId])
  const rows: ListRow[] = useMemo(
    () => [
      ...composer.pending
        .filter((pending) => !loaded.some((message) => message.nonce === pending.nonce))
        .map((pending): ListRow => ({ type: 'pending', key: pending.nonce, pending }))
        .reverse(),
      ...buildMessageRows(loaded).reverse(),
    ],
    [loaded, composer.pending],
  )

  return (
    <SafeAreaView edges={['top', 'left', 'right', 'bottom']} className="flex-1 bg-card">
      <View className="h-12 flex-row items-center gap-2 px-3">
        <Icon name="magicStar" size={20} color={colors.primary} />
        <Text numberOfLines={1} className="flex-1 font-noto-semibold text-[14px]">
          {agent.name}
        </Text>
        <Pressable
          accessibilityRole="button"
          onPress={() => {
            onClose()
            router.push(`/chat/${conversationId}`)
          }}
          className="h-8 px-2.5 rounded-md justify-center active:bg-muted"
        >
          <Text className="font-noto-medium text-[13px]">Open in Chat</Text>
        </Pressable>
        <IconButton icon="x" label="Close" size={32} iconSize={20} color={colors.mutedForeground} onPress={onClose} />
      </View>
      {item && pageContextFor(pathname, item)?.item ? (
        <View className="flex-row items-center gap-1.5 px-4 pb-1.5">
          <Icon name={item.kind === 'task' ? 'tasks' : 'docs'} size={14} color={colors.mutedForeground} />
          <Text numberOfLines={1} className="flex-1 text-[12px] text-muted-foreground">
            About: {item.title || 'Untitled'}
          </Text>
        </View>
      ) : null}
      <KeyboardAvoidingView className="flex-1" behavior={Platform.OS === 'ios' ? 'padding' : undefined}>
        {!messages.data ? (
          messages.isError ? <ErrorView error={messages.error} onRetry={() => void messages.refetch()} /> : <LoadingView />
        ) : (
          <MessageList
            rows={rows}
            handlers={handlers}
            loadingOlder={messages.isFetchingPreviousPage}
            onOlder={() => {
              if (messages.hasPreviousPage && !messages.isFetchingPreviousPage) void messages.fetchPreviousPage()
            }}
          />
        )}
        <Composer state={composer} placeholder={`Ask ${agent.name}`} nameOf={handlers.nameOf} />
      </KeyboardAvoidingView>
      {sheet}
    </SafeAreaView>
  )
}
