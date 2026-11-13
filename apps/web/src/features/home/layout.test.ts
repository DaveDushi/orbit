import { expect, test } from 'bun:test'
import { addableCards, addCard, dropCard, moveCard, removeCard, setCardLimit, type HomeCardEntry } from './layout'

const cards: HomeCardEntry[] = [{ id: 'a' }, { id: 'b', limit: 5 }, { id: 'c' }]
const ids = (list: HomeCardEntry[]) => list.map((card) => card.id)

test('adding a card puts it last, and a card already on Home is not repeated', () => {
  expect(ids(addCard(cards, 'd'))).toEqual(['a', 'b', 'c', 'd'])
  expect(addCard(cards, 'b')).toEqual(cards)
})

test('removing a card keeps the others and their options', () => {
  expect(removeCard(cards, 'a')).toEqual([{ id: 'b', limit: 5 }, { id: 'c' }])
  expect(removeCard(cards, 'x')).toEqual(cards)
})

test('moving a card keeps it once and clamps the place to the list', () => {
  expect(ids(moveCard(cards, 'a', 2))).toEqual(['b', 'c', 'a'])
  expect(ids(moveCard(cards, 'c', 0))).toEqual(['c', 'a', 'b'])
  expect(ids(moveCard(cards, 'b', -1))).toEqual(['b', 'a', 'c'])
  expect(ids(moveCard(cards, 'b', 99))).toEqual(['a', 'c', 'b'])
  expect(moveCard(cards, 'x', 0)).toEqual(cards)
  // the option travels with the card
  expect(moveCard(cards, 'b', 0)[0]).toEqual({ id: 'b', limit: 5 })
})

test('a dropped card takes the place of the card under it, up or down', () => {
  expect(ids(dropCard(cards, 'a', 'c'))).toEqual(['b', 'c', 'a'])
  expect(ids(dropCard(cards, 'c', 'a'))).toEqual(['c', 'a', 'b'])
  expect(dropCard(cards, 'a', 'a')).toEqual(cards)
  expect(dropCard(cards, 'a', 'x')).toEqual(cards)
})

test('the default row count is not stored', () => {
  expect(setCardLimit(cards, 'a', 10)[0]).toEqual({ id: 'a', limit: 10 })
  expect(setCardLimit(cards, 'b', 7)[1]).toEqual({ id: 'b' })
})

test('the picker offers the allowed cards that are not on Home, in catalog order', () => {
  expect(addableCards(cards, ['d', 'a', 'e', 'c'])).toEqual(['d', 'e'])
})
