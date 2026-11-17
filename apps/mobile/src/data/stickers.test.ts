import { expect, test } from 'bun:test'
import { ApiProblem } from '@orbit/api/problem'

import { stickerNameFromFile, stickerProblem, stickerServerProblem } from './stickers'

const png = { type: 'image/png', size: 1000 }

test('stickerProblem follows the server rules', () => {
  expect(stickerProblem(png, 'Party Parrot', [])).toBeNull()
  expect(stickerProblem({ type: 'image/svg+xml', size: 10 }, 'Logo', [])).toBe('Choose a PNG, JPEG, WebP or GIF image.')
  expect(stickerProblem({ type: '', size: 10 }, 'Logo', [])).toBeNull()
  expect(stickerProblem({ type: 'image/gif', size: 600 * 1024 }, 'Big', [])).toBe('The image is too large. The limit is 512 KB.')
  expect(stickerProblem(png, ' x ', [])).toBe('A name has 2 to 30 characters.')
  expect(stickerProblem(png, 'party PARROT', ['Party Parrot'])).toBe('A sticker named party PARROT exists already. Choose another name.')
})

test('stickerNameFromFile drops the extension and keeps 30 characters', () => {
  expect(stickerNameFromFile('Party Parrot.gif')).toBe('Party Parrot')
  expect(stickerNameFromFile(`${'a'.repeat(40)}.png`)).toBe('a'.repeat(30))
})

test('stickerServerProblem explains known codes', () => {
  expect(stickerServerProblem(new ApiProblem({ type: 'about:blank', title: 'x', status: 409, code: 'sticker_name_taken', detail: 'taken' } as never), 'Failed.')).toBe('A sticker with this name exists already. Choose another name.')
  expect(stickerServerProblem(new Error('offline'), 'Failed.')).toBe('Failed. Orbit did not answer. Check your connection and try again.')
})
