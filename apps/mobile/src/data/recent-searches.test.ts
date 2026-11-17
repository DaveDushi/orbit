import { expect, test } from 'bun:test'

import { parseRecentSearches, recentSearchesKey, withRecentSearch } from './recent-searches'

test('a search goes first, a repeat in another case moves up, the list keeps ten', () => {
  expect(withRecentSearch(['deploy', 'Budget'], '  budget   plan ')).toEqual(['budget plan', 'deploy', 'Budget'])
  expect(withRecentSearch(['deploy', 'Budget'], 'BUDGET')).toEqual(['BUDGET', 'deploy'])
  expect(withRecentSearch(['a'], '   ')).toEqual(['a'])
  expect(withRecentSearch(Array.from({ length: 10 }, (_, index) => `q${index}`), 'new')).toHaveLength(10)
})

test('a damaged store gives an empty list; the key is keychain-safe', () => {
  expect(parseRecentSearches('{oops')).toEqual([])
  expect(parseRecentSearches('[1, "a"]')).toEqual(['a'])
  expect(parseRecentSearches(null)).toEqual([])
  expect(recentSearchesKey('w-1', 'u:2')).toBe('orbit.chat.recent-searches.w-1.u_2')
})
