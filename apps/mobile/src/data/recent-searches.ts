/** The last chat searches, as on the web (`features/chat/lib/recentSearches.ts`): one list per workspace and person, newest first. */
export const MAX_RECENT_SEARCHES = 10

/** The store key (letters, digits, `.`, `-` and `_` only, as the keychain wants). */
export const recentSearchesKey = (workspaceId: string, userId: string) => `orbit.chat.recent-searches.${workspaceId}.${userId}`.replace(/[^\w.-]/g, '_')

/** `query` first; an older entry that differs only in case or spaces goes, and the list keeps `max` entries. */
export function withRecentSearch(list: readonly string[], query: string, max = MAX_RECENT_SEARCHES): string[] {
  const text = query.trim().replace(/\s+/g, ' ')
  if (!text) return [...list]
  const key = text.toLowerCase()
  return [text, ...list.filter((entry) => entry.toLowerCase() !== key)].slice(0, max)
}

/** A stored list, or `[]` when it is missing or not readable. */
export function parseRecentSearches(stored: string | null): string[] {
  try {
    const parsed: unknown = JSON.parse(stored ?? '[]')
    return Array.isArray(parsed) ? parsed.filter((entry): entry is string => typeof entry === 'string').slice(0, MAX_RECENT_SEARCHES) : []
  } catch {
    return []
  }
}
