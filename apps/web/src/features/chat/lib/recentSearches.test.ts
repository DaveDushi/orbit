import { beforeEach, expect, test } from 'bun:test'
import { addRecentSearch, clearRecentSearches, getRecentSearches, MAX_RECENT_SEARCHES, withRecentSearch } from './recentSearches'

let workspace = ''
beforeEach(() => {
  // A fresh workspace id for each test: the module keeps a parsed copy for each list.
  workspace = `w-${crypto.randomUUID()}`
})

test('a new search goes first; a repeat moves to the top without a duplicate, also in another case or spacing', () => {
  expect(withRecentSearch(['b', 'a'], 'c')).toEqual(['c', 'b', 'a'])
  expect(withRecentSearch(['b', 'Release plan', 'a'], '  release   PLAN ')).toEqual(['release PLAN', 'b', 'a'])
  expect(withRecentSearch(['a'], '   ')).toEqual(['a'])
})

test('the list keeps the newest searches only', () => {
  let list: string[] = []
  for (let index = 0; index < MAX_RECENT_SEARCHES + 3; index += 1) list = withRecentSearch(list, `q${index}`)
  expect(list).toHaveLength(MAX_RECENT_SEARCHES)
  expect(list[0]).toBe(`q${MAX_RECENT_SEARCHES + 2}`)
  expect(list.at(-1)).toBe('q3')
})

test('each person has their own list in localStorage, and clear removes it', () => {
  addRecentSearch(workspace, 'u1', 'deploy')
  addRecentSearch(workspace, 'u1', 'links')
  addRecentSearch(workspace, 'u2', 'other')
  expect(getRecentSearches(workspace, 'u1')).toEqual(['links', 'deploy'])
  expect(getRecentSearches(workspace, 'u2')).toEqual(['other'])
  expect(JSON.parse(localStorage.getItem(`orbit:chat:recent-searches:${workspace}:u1`) ?? 'null')).toEqual(['links', 'deploy'])

  clearRecentSearches(workspace, 'u1')
  expect(getRecentSearches(workspace, 'u1')).toEqual([])
  expect(localStorage.getItem(`orbit:chat:recent-searches:${workspace}:u1`)).toBeNull()
  expect(getRecentSearches(workspace, 'u2')).toEqual(['other'])
})

test('stored junk is ignored', () => {
  localStorage.setItem(`orbit:chat:recent-searches:${workspace}:u1`, JSON.stringify(['ok', 3, null]))
  expect(getRecentSearches(workspace, 'u1')).toEqual(['ok'])
  localStorage.setItem(`orbit:chat:recent-searches:${workspace}:u2`, '{not json')
  expect(getRecentSearches(workspace, 'u2')).toEqual([])
})
