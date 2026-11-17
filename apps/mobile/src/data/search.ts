/**
 * The app's Search (the web's command palette search): tasks, docs and chats that match what the person typed, from
 * the lists the app has loaded already. No React.
 */
import type { ConversationRecord, PageSummary, ProjectRecord, TaskRecord } from './orbit'
import { identifier } from './tasks'

/** Rows per group, as on the web palette. */
export const SEARCH_LIMIT = 8

/** The words of a query, lower case. Empty for a blank query. */
export const searchWords = (query: string) => query.trim().toLowerCase().split(/\s+/).filter(Boolean)

/** Every word is somewhere in one of the texts. */
const matches = (words: readonly string[], texts: readonly string[]) => {
  const haystack = texts.join(' ').toLowerCase()
  return words.every((word) => haystack.includes(word))
}

export interface SearchInput {
  tasks: readonly TaskRecord[]
  projects: readonly ProjectRecord[]
  pages: readonly PageSummary[]
  /** Joined conversations with their shown titles (a DM's title is its people). */
  conversations: readonly { conversation: ConversationRecord; title: string }[]
}

export interface SearchResults {
  tasks: TaskRecord[]
  pages: PageSummary[]
  conversations: { conversation: ConversationRecord; title: string }[]
}

/**
 * What matches `query`: tasks by title or identifier and pages by title, the last changed first, and chats by name. A
 * blank query finds nothing. Trashed tasks and archived chats are left out.
 */
export function searchAll(query: string, input: SearchInput, limit = SEARCH_LIMIT): SearchResults {
  const words = searchWords(query)
  if (words.length === 0) return { tasks: [], pages: [], conversations: [] }
  const tasks = input.tasks
    .filter((task) => !task.deleted_at && matches(words, [task.title, identifier(task, input.projects)]))
    .sort((a, b) => b.updated_at.localeCompare(a.updated_at))
  const pages = input.pages.filter((page) => matches(words, [page.title])).sort((a, b) => b.updated_at.localeCompare(a.updated_at))
  const conversations = input.conversations.filter(({ conversation, title }) => !conversation.archived && matches(words, [title]))
  return { tasks: tasks.slice(0, limit), pages: pages.slice(0, limit), conversations: conversations.slice(0, limit) }
}
