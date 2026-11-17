import { describe, expect, test } from 'bun:test'

import { decodeMentions } from './chat'
import { encodeMentions, insertMention, mentionQueryAt, mentionSuggestions } from './mentions'

const people = [
  { id: 'u1', name: 'Ada' },
  { id: 'u2', name: 'Ada Lovelace' },
  { id: 'u3', name: 'Bob' },
]
const channels = [{ id: 'c1', name: 'general' }]

describe('encodeMentions', () => {
  test('names become tokens, the longest name wins, case does not count', () => {
    expect(encodeMentions('hi @ada lovelace and @Bob, see #general', people, channels)).toBe('hi <@u2> and <@u3>, see <#c1>')
    expect(encodeMentions('@Ada!', people, channels)).toBe('<@u1>!')
  })

  test('@channel and @here are global mentions', () => {
    expect(encodeMentions('@channel and @here', people, channels)).toBe('<!channel> and <!here>')
  })

  test('a name inside a word or code stays text', () => {
    expect(encodeMentions('mail@Bob `@Bob` ```\n@Bob\n``` @Bobby', people, channels)).toBe('mail@Bob `@Bob` ```\n@Bob\n``` @Bobby')
  })

  test('decode then encode gives the stored body back (an edit keeps its mentions)', () => {
    const body = 'ping <@u2> in <#c1> <!here>'
    expect(encodeMentions(decodeMentions(body, people, channels), people, channels)).toBe(body)
  })
})

describe('mentionQueryAt', () => {
  test('the later trigger before the cursor, after white space only', () => {
    expect(mentionQueryAt('hi @ad', 6)).toEqual({ trigger: '@', start: 3, end: 6, query: 'ad' })
    expect(mentionQueryAt('#gen', 4)).toEqual({ trigger: '#', start: 0, end: 4, query: 'gen' })
    expect(mentionQueryAt('mail@x', 6)).toBeNull()
    expect(mentionQueryAt('@a\nb', 4)).toBeNull()
    expect(mentionQueryAt('no trigger', 10)).toBeNull()
  })
})

describe('mentionSuggestions and insertMention', () => {
  test('filters by name; @channel only where everyone may be named', () => {
    const query = mentionQueryAt('@ad', 3)!
    expect(mentionSuggestions(query, people, channels, { everyone: true }).map((item) => item.id)).toEqual(['u1', 'u2'])
    const all = mentionQueryAt('@', 1)!
    expect(mentionSuggestions(all, people, channels, { everyone: false, limit: 10 }).some((item) => item.id === '!channel')).toBe(false)
    expect(mentionSuggestions(mentionQueryAt('#g', 2)!, people, channels, { everyone: true }).map((item) => item.name)).toEqual(['general'])
  })

  test('replaces the query and puts the cursor after the space', () => {
    expect(insertMention('hi @ad there', { trigger: '@', start: 3, end: 6, query: 'ad' }, 'Ada Lovelace')).toEqual({ text: 'hi @Ada Lovelace  there', cursor: 17 })
  })
})
