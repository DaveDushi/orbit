import { type QueryClient } from '@tanstack/react-query'

import { type MessagePages, removeMessage, type ThreadPages, upsertMessage, upsertState, upsertThreadMessage } from './data/chat'
import type * as chat from './data/chat-api'
import { keys } from './data/keys'
import { type ChatAction, chatActions } from './data/live'
import type { ConversationStateRecord } from './data/orbit'

/**
 * Keys of the chat queries outside `data/keys.ts`. Threads sit under the `messages` prefix so a chat resync (`chatKeys`) refetches
 * them with the conversations.
 */
export const chatQueryKeys = {
  followedThreads: (workspaceId: string) => ['messages', workspaceId, 'followed-threads'] as const,
  unreadThreads: (workspaceId: string) => ['messages', workspaceId, 'unread-threads'] as const,
  thread: (workspaceId: string, rootId: string) => ['messages', workspaceId, 'thread', rootId] as const,
  around: (workspaceId: string, conversationId: string, messageId: string) => ['messages', workspaceId, 'around', conversationId, messageId] as const,
  conversationList: (workspaceId: string, conversationId: string, kind: ConversationList) => ['messages', workspaceId, kind, conversationId] as const,
  search: (workspaceId: string, query: Omit<chat.SearchQuery, 'cursor'>) => ['chat-search', workspaceId, query] as const,
  customEmoji: keys.chatEmoji,
  emojibase: (server: string) => ['emojibase', server] as const,
}

export type ConversationList = 'pins' | 'files' | 'threads'

export function applyChatActions(queryClient: QueryClient, workspaceId: string, actions: readonly ChatAction[]) {
  for (const action of actions) {
    switch (action.kind) {
      case 'upsert-message':
        queryClient.setQueryData<MessagePages>(keys.messages(workspaceId, action.conversationId), (data) => data && upsertMessage(data, action.message))
        break
      case 'remove-message':
        queryClient.setQueryData<MessagePages>(keys.messages(workspaceId, action.conversationId), (data) => data && removeMessage(data, action.messageId))
        break
      case 'state':
        queryClient.setQueryData<ConversationStateRecord[]>(keys.chatStates(workspaceId), (states) => states && upsertState(states, action.state))
        break
      case 'invalidate':
        for (const queryKey of action.keys) void queryClient.invalidateQueries({ queryKey })
        break
    }
  }
}

/**
 * Applies the events of a write the way the live socket applies them, and also to the open thread and the followed
 * threads, which the socket hook does not patch.
 */
export function applyChatEvents(queryClient: QueryClient, workspaceId: string, events: readonly chat.ChatEvent[]) {
  applyAgainAfterFetch(queryClient, workspaceId, events)
  const states = queryClient.getQueryData<ConversationStateRecord[]>(keys.chatStates(workspaceId))
  const wasMuted = (conversationId: string) => states?.find((state) => state.conversation_id === conversationId)?.notify === 'muted'
  applyChatActions(
    queryClient,
    workspaceId,
    events.flatMap((event) => chatActions(event, workspaceId)),
  )
  for (const event of events) {
    if (event.type === 'message.created' || event.type === 'message.updated') {
      const rootId = event.message.thread_root_id ?? event.message.id
      queryClient.setQueryData<ThreadPages>(chatQueryKeys.thread(workspaceId, rootId), (data) => data && upsertThreadMessage(data, event.message))
    }
    // the conversation's Threads, Pins and Files lists: refetched when on show, else when opened
    const conversationId = event.type === 'message.deleted' ? event.conversation_id : event.type === 'message.created' || event.type === 'message.updated' ? event.message.conversation_id : null
    if (conversationId) {
      for (const kind of ['threads', 'pins', 'files'] as const) void queryClient.invalidateQueries({ queryKey: chatQueryKeys.conversationList(workspaceId, conversationId, kind) })
    }
    if (event.type === 'message.deleted' && event.thread_root_id) void queryClient.invalidateQueries({ queryKey: chatQueryKeys.thread(workspaceId, event.thread_root_id) })
    if (event.type === 'thread.changed' || (event.type === 'message.created' && event.message.thread_root_id)) {
      void queryClient.invalidateQueries({ queryKey: chatQueryKeys.followedThreads(workspaceId) })
    }
    // Unreads lists threads the user does not follow too: their replies send no thread state. Unreads leaves out
    // muted conversations; unmuting brings their threads back.
    const unmuted = event.type === 'state.changed' && event.state.notify !== 'muted' && wasMuted(event.state.conversation_id)
    if (
      event.type === 'thread.changed' ||
      (event.type === 'message.created' && event.message.thread_root_id && !wasMuted(event.message.conversation_id)) ||
      (event.type === 'message.deleted' && event.thread_root_id) ||
      unmuted
    ) {
      void queryClient.invalidateQueries({ queryKey: chatQueryKeys.unreadThreads(workspaceId) })
    }
  }
}

/** The queries that an event patches in place. */
function patchedKeys(workspaceId: string, event: chat.ChatEvent): (readonly string[])[] {
  switch (event.type) {
    case 'message.created':
    case 'message.updated':
      return [keys.messages(workspaceId, event.message.conversation_id), chatQueryKeys.thread(workspaceId, event.message.thread_root_id ?? event.message.id)]
    case 'message.deleted':
      return [keys.messages(workspaceId, event.conversation_id)]
    case 'state.changed':
      return [keys.chatStates(workspaceId)]
    default:
      return []
  }
}

/**
 * An event for a query that is loading can be lost: the query has no data to patch yet, or the response that comes
 * was read before the event and takes its place. The events are applied once more when the load ends: each one gives
 * the same result when it is applied a second time.
 */
function applyAgainAfterFetch(queryClient: QueryClient, workspaceId: string, events: readonly chat.ChatEvent[]) {
  const loading = events
    .flatMap((event) => patchedKeys(workspaceId, event))
    .flatMap((queryKey) => queryClient.getQueryCache().findAll({ queryKey, exact: true, fetchStatus: 'fetching' }))
  if (loading.length === 0) return
  void Promise.allSettled(loading.map((query) => query.promise)).then(() => {
    const again = events.filter((event) => patchedKeys(workspaceId, event).length > 0)
    applyChatActions(queryClient, workspaceId, again.flatMap((event) => chatActions(event, workspaceId).filter((action) => action.kind !== 'invalidate')))
    for (const event of again) {
      if (event.type === 'message.created' || event.type === 'message.updated') {
        queryClient.setQueryData<ThreadPages>(chatQueryKeys.thread(workspaceId, event.message.thread_root_id ?? event.message.id), (data) => data && upsertThreadMessage(data, event.message))
      }
    }
  })
}
