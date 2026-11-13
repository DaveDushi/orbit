import { describe, expect, test } from 'bun:test'
import { ApiProblem } from '@orbit/api/problem'

import { emojiNameFromFile, emojiProblem, emojiServerProblem } from './emoji'

describe('emojiProblem', () => {
  const free = () => false
  test('accepts a small image with a valid name', () => {
    expect(emojiProblem({ type: 'image/png', size: 1000 }, 'party_parrot', free)).toBeNull()
    // the picker may not know the type or size: the server checks them
    expect(emojiProblem({ type: '', size: null }, 'ok', free)).toBeNull()
  })

  test('refuses other types, large files, bad and taken names', () => {
    expect(emojiProblem({ type: 'image/svg+xml', size: 10 }, 'ok', free)).toContain('PNG')
    expect(emojiProblem({ type: 'image/gif', size: 300 * 1024 }, 'ok', free)).toContain('256 KB')
    expect(emojiProblem({ type: 'image/gif', size: 10 }, 'A', free)).toContain('2 to 32')
    expect(emojiProblem({ type: 'image/gif', size: 10 }, 'joy', (name) => name === 'joy')).toBe(':joy: exists already. Choose another name.')
  })
})

test('emojiNameFromFile', () => {
  expect(emojiNameFromFile('Party Parrot.gif')).toBe('party_parrot')
})

test('emojiServerProblem words a known code and keeps an unknown one', () => {
  expect(emojiServerProblem(new ApiProblem({ status: 409, code: 'emoji_name_taken', title: 'x', detail: 'x' } as never), 'Failed.')).toContain('exists already')
  expect(emojiServerProblem(new Error('offline'), 'Failed.')).toContain('did not answer')
})
