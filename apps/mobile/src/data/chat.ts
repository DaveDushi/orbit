import type { CategoryRecord, ConversationRecord, ConversationStateRecord, MessagePage, MessageRecord } from './orbit'

export interface Person {
  id: string
  name: string
}

const isMember = (conversation: ConversationRecord, userId: string) => conversation.member_ids.includes(userId)
const isVisible = (conversation: ConversationRecord, userId: string) => isMember(conversation, userId) && !conversation.archived

/**
 * The count badge of a sidebar row, like the web: every unread message when notify is `all`, else mentions only. `unread`
 * makes the name bold.
 */
export function conversationBadge(
  conversation: ConversationRecord,
  state: ConversationStateRecord | undefined,
  userId: string,
): { count: number; unread: boolean } {
  if (!state || !isVisible(conversation, userId)) return { count: 0, unread: false }
  if (state.notify === 'muted') return { count: state.mention_count, unread: false }
  return { count: state.notify === 'all' ? state.unread_count : state.mention_count, unread: state.unread_count > 0 }
}

/** The Chat tab badge: the sum of the row badges. */
export function chatUnreadCount(conversations: readonly ConversationRecord[], states: readonly ConversationStateRecord[], userId: string): number {
  const stateById = new Map(states.map((state) => [state.conversation_id, state]))
  return conversations.reduce((total, conversation) => total + conversationBadge(conversation, stateById.get(conversation.id), userId).count, 0)
}

export function conversationTitle(conversation: ConversationRecord, people: readonly Person[], userId: string): string {
  if (conversation.kind !== 'dm') return conversation.name
  const nameOf = (id: string) => people.find((person) => person.id === id)?.name ?? 'Unknown'
  if (conversation.self_dm) return `${nameOf(userId)} (you)`
  const peers = conversation.member_ids.filter((id) => id !== userId)
  return peers.length === 0 ? 'Former member' : peers.map(nameOf).join(', ')
}

export interface ChatSection {
  key: string
  title: string
  conversations: ConversationRecord[]
}

const lastActivity = (conversation: ConversationRecord) => conversation.last_message_at ?? conversation.created_at
const byPosition = (a: ConversationRecord, b: ConversationRecord) => a.position - b.position || a.name.localeCompare(b.name)
const byActivity = (a: ConversationRecord, b: ConversationRecord) => lastActivity(b).localeCompare(lastActivity(a)) || (a.id < b.id ? -1 : 1)

/**
 * The web sidebar: Favorites (when any), each category by position, "Channels" for channels without one, then Direct
 * messages by last activity with the DM to yourself first. Only joined, unarchived conversations; empty sections are left
 * out, as a phone cannot fill them.
 */
export function chatSections(
  conversations: readonly ConversationRecord[],
  categories: readonly CategoryRecord[],
  states: readonly ConversationStateRecord[],
  userId: string,
): ChatSection[] {
  const favoriteIds = new Set(states.filter((state) => state.favorite).map((state) => state.conversation_id))
  const joined = conversations.filter((conversation) => isVisible(conversation, userId))
  const favorites = joined.filter((conversation) => favoriteIds.has(conversation.id))
  const rest = joined.filter((conversation) => !favoriteIds.has(conversation.id))
  const channels = rest.filter((conversation) => conversation.kind !== 'dm')
  const categoryIds = new Set(categories.map((category) => category.id))

  const sections: ChatSection[] = [
    {
      key: 'favorites',
      title: 'Favorites',
      conversations: [
        ...favorites.filter((conversation) => conversation.kind !== 'dm').sort((a, b) => a.name.localeCompare(b.name)),
        ...favorites.filter((conversation) => conversation.kind === 'dm').sort(byActivity),
      ],
    },
    ...[...categories]
      .sort((a, b) => a.position - b.position)
      .map((category) => ({
        key: `category-${category.id}`,
        title: category.name,
        conversations: channels.filter((channel) => channel.category_id === category.id).sort(byPosition),
      })),
    {
      key: 'channels',
      title: 'Channels',
      conversations: channels.filter((channel) => !channel.category_id || !categoryIds.has(channel.category_id)).sort(byPosition),
    },
    {
      key: 'dms',
      title: 'Direct messages',
      conversations: rest
        .filter((conversation) => conversation.kind === 'dm')
        .sort((a, b) => Number(b.self_dm) - Number(a.self_dm) || byActivity(a, b)),
    },
  ]
  return sections.filter((section) => section.conversations.length > 0)
}

const MENTION = /<@([^<>\s]+)>|<#([^<>\s]+)>|<!(channel|here)>/g

/** Stored body to the text shown: `<@u1>` becomes `@Ada Lovelace`, `<#c1>` becomes `#general`. */
export function decodeMentions(body: string, people: readonly Person[], conversations: readonly Person[]): string {
  return body.replace(MENTION, (_token, userId?: string, conversationId?: string, global?: string) => {
    if (global) return `@${global}`
    if (userId) return `@${people.find((person) => person.id === userId)?.name ?? 'unknown'}`
    return `#${conversations.find((conversation) => conversation.id === conversationId)?.name ?? 'unknown'}`
  })
}

/** The main list of a conversation holds root messages and system rows; a thread reply only when also sent to the channel. */
export const inMainList = (message: MessageRecord) => message.thread_root_id === null || message.also_in_channel

export type MessageRow =
  | { type: 'day'; key: string; at: string }
  | { type: 'message'; key: string; message: MessageRecord; groupStart: boolean }

const CONTINUE_WITHIN = 5 * 60_000
const GROUP_SPAN = 10 * 60_000

const groupable = (message: MessageRecord) => message.kind === 'message' && !message.deleted && !message.also_in_channel && !message.forwarded
const localDay = (iso: string) => new Date(iso).toDateString()

/**
 * Messages (oldest first) to rows, like the web: a day separator before the first message of each local day, and for
 * each message whether it starts a group (avatar, name, time) or continues the one above. A reply always starts one.
 */
export function buildMessageRows(messages: readonly MessageRecord[]): MessageRow[] {
  const rows: MessageRow[] = []
  let previous: MessageRecord | null = null
  let groupStartedAt = 0
  for (const message of messages) {
    const at = Date.parse(message.created_at)
    const dayChanged = !previous || localDay(previous.created_at) !== localDay(message.created_at)
    if (dayChanged) rows.push({ type: 'day', key: `day-${message.created_at}`, at: message.created_at })
    const continues =
      previous !== null &&
      !dayChanged &&
      groupable(previous) &&
      groupable(message) &&
      !message.reply_to_id &&
      previous.author_id === message.author_id &&
      at - Date.parse(previous.created_at) < CONTINUE_WITHIN &&
      at - groupStartedAt < GROUP_SPAN
    if (!continues) groupStartedAt = at
    rows.push({ type: 'message', key: message.nonce ?? message.id, message, groupStart: !continues })
    previous = message
  }
  return rows
}

/** The cached pages of one conversation, oldest page first, as TanStack Query keeps an infinite query. */
export interface MessagePages {
  pages: MessagePage[]
  pageParams: unknown[]
}

/**
 * Puts a new or changed message into the cached pages. A changed one replaces its copy wherever it is. A new one is
 * appended to the newest page, but only when that page reaches the present (`after` is null): otherwise it lies beyond
 * what is loaded. Thread replies stay out of the main list.
 */
export function upsertMessage(data: MessagePages, message: MessageRecord): MessagePages {
  if (!inMainList(message)) return data
  const known = data.pages.some((page) => page.items.some((item) => item.id === message.id))
  if (known) {
    return { ...data, pages: data.pages.map((page) => ({ ...page, items: page.items.map((item) => (item.id === message.id ? message : item)) })) }
  }
  const last = data.pages.at(-1)
  if (!last || last.after !== null) return data
  return { ...data, pages: [...data.pages.slice(0, -1), { ...last, items: [...last.items, message] }] }
}

export function removeMessage(data: MessagePages, messageId: string): MessagePages {
  return { ...data, pages: data.pages.map((page) => ({ ...page, items: page.items.filter((item) => item.id !== messageId) })) }
}

/** The list with `state` in place of the old state of its conversation. */
export function upsertState(states: readonly ConversationStateRecord[], state: ConversationStateRecord): ConversationStateRecord[] {
  return states.some((item) => item.conversation_id === state.conversation_id)
    ? states.map((item) => (item.conversation_id === state.conversation_id ? state : item))
    : [...states, state]
}

/** Ties a sent message to its echo, so sending twice (a retry) stores it once. Hermes has no `crypto.randomUUID`. */
export const newNonce = () => `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 12)}`
