/**
 * Mentions in the composer, as on the web (`features/chat/lib/mentionTokens.ts`, `lib/useMentionAutocomplete.ts`):
 * the field shows `@Ada Lovelace` and `#general`, the stored body has `<@id>` and `<#id>`, so the server notifies the
 * person. No React, so the tests drive it.
 */
import type { Person } from './chat'

/** Fenced and inline code: `@name` inside it is text, not a mention. */
const CODE = /(```[\s\S]*?```|`[^`\n]+`)/

const isBoundary = (char: string | undefined) => !char || /[\s.,!?;:()[\]{}"'`]/.test(char)

interface Candidate {
  /** The name in lower case. */
  label: string
  /** The length of the name as written: lower case can change the length of a string ("İ"). */
  length: number
  token: string
}

/** What `@…` and `#…` can match, longest name first so "Ann Lee" wins over "Ann". */
function candidates(people: readonly Person[], channels: readonly Person[]) {
  const byLength = (a: Candidate, b: Candidate) => b.label.length - a.label.length
  const users = [
    { label: 'channel', length: 7, token: '<!channel>' },
    { label: 'here', length: 4, token: '<!here>' },
    ...people.map((person) => ({ label: person.name.toLowerCase(), length: person.name.length, token: `<@${person.id}>` })),
  ]
    .filter((item) => item.label)
    .sort(byLength)
  const rooms = channels
    .map((channel) => ({ label: channel.name.toLowerCase(), length: channel.name.length, token: `<#${channel.id}>` }))
    .filter((item) => item.label)
    .sort(byLength)
  return { users, rooms }
}

/**
 * Composer text to the stored body. A name matches without regard to case, must start after a word boundary and end at
 * one, and the longest name wins ("@Ada Lovelace" before "@Ada"). Code is left alone.
 */
export function encodeMentions(text: string, people: readonly Person[], channels: readonly Person[]): string {
  const { users, rooms } = candidates(people, channels)
  let out = ''
  text.split(CODE).forEach((part, partIndex) => {
    // `split` with a capturing group puts the code segments at the odd indexes
    if (partIndex % 2 === 1) {
      out += part
      return
    }
    let index = 0
    while (index < part.length) {
      const char = part[index]
      const list = char === '@' ? users : char === '#' ? rooms : null
      const match =
        list && isBoundary(part[index - 1])
          ? list.find((item) => part.slice(index + 1, index + 1 + item.length).toLowerCase() === item.label && isBoundary(part[index + 1 + item.length]))
          : undefined
      if (match) {
        out += match.token
        index += 1 + match.length
      } else {
        out += char
        index += 1
      }
    }
  })
  return out
}

/** `@…` or `#…` being typed just before the cursor. */
export interface MentionQuery {
  trigger: '@' | '#'
  /** Where the `@` or `#` is. */
  start: number
  /** The cursor. */
  end: number
  query: string
}

/** The mention typed before `cursor`: the later `@` or `#`, at the start or after white space, without a new line. */
export function mentionQueryAt(text: string, cursor: number): MentionQuery | null {
  const before = text.slice(0, cursor)
  const start = Math.max(before.lastIndexOf('@'), before.lastIndexOf('#'))
  if (start === -1) return null
  const query = before.slice(start + 1)
  if ((start > 0 && !/\s/.test(before[start - 1]!)) || query.includes('\n') || query.length > 48) return null
  return { trigger: before[start] as '@' | '#', start, end: cursor, query }
}

export interface MentionSuggestion {
  id: string
  /** What goes into the field after the trigger. */
  name: string
  /** A second line: "everyone here" for `@channel`. */
  note?: string
}

/** What the picker lists for a query: names that contain it (people, then `@channel`/`@here`), at most `limit`. */
export function mentionSuggestions(query: MentionQuery, people: readonly Person[], channels: readonly Person[], options: { everyone: boolean; limit?: number }): MentionSuggestion[] {
  const text = query.query.trim().toLowerCase()
  const all: MentionSuggestion[] =
    query.trigger === '#'
      ? channels.map((channel) => ({ id: channel.id, name: channel.name }))
      : [
          ...[...people].sort((a, b) => a.name.localeCompare(b.name)).map((person) => ({ id: person.id, name: person.name })),
          ...(options.everyone
            ? [
                { id: '!channel', name: 'channel', note: 'everyone here' },
                { id: '!here', name: 'here', note: 'everyone online' },
              ]
            : []),
        ]
  return all.filter((item) => item.name && (!text || item.name.toLowerCase().includes(text))).slice(0, options.limit ?? 6)
}

/** The text with the query replaced by the picked name and a space; the cursor goes after the space. */
export function insertMention(text: string, query: MentionQuery, name: string): { text: string; cursor: number } {
  const insertion = `${query.trigger}${name} `
  return { text: `${text.slice(0, query.start)}${insertion}${text.slice(query.end)}`, cursor: query.start + insertion.length }
}

/** Whether a body notifies a whole channel: `@channel` asks first in a big one, as on the web. */
export const notifiesEveryone = (body: string) => /<!(channel|here)>/.test(body)
