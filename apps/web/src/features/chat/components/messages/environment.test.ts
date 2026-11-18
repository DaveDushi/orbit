import { expect, test } from 'bun:test'
import { touchOnly } from './environment'

/** A device as `matchMedia` describes it: the queries that match. */
function device(...matching: string[]) {
  return (query: string) => matching.includes(query)
}

test('a touch laptop that reports touch as its main pointer is not touch-only: Enter sends', () => {
  expect(touchOnly(device('(hover: none) and (pointer: coarse)', '(any-pointer: fine)'))).toBe(false)
})

test('a desktop or a laptop is not touch-only', () => {
  expect(touchOnly(device('(any-pointer: fine)'))).toBe(false)
})

test('a phone or a tablet without a trackpad is touch-only: Enter makes a new line', () => {
  expect(touchOnly(device('(hover: none) and (pointer: coarse)'))).toBe(true)
})
