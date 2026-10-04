import { expect, test } from 'bun:test'
import { overLimit, parseMediaLimit } from './localCache'

test('the oldest cached files go first, and only as many as the limit needs', () => {
  const files = [
    { name: 'oldest', size: 40 },
    { name: 'older', size: 30 },
    { name: 'newer', size: 20 },
    { name: 'newest', size: 10 },
  ]
  expect(overLimit(files, 100)).toEqual([])
  expect(overLimit(files, 60).map((file) => file.name)).toEqual(['oldest'])
  expect(overLimit(files, 25).map((file) => file.name)).toEqual(['oldest', 'older', 'newer'])
  expect(overLimit(files, 0).map((file) => file.name)).toEqual(['oldest', 'older', 'newer', 'newest'])
})

test('a stored limit that is not one of the choices becomes the default', () => {
  expect(parseMediaLimit('500')).toBe(500)
  expect(parseMediaLimit('7')).toBe(250)
  expect(parseMediaLimit(null)).toBe(250)
})
