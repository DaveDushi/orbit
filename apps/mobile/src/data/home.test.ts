import { expect, test } from 'bun:test'
import { addableCards, addCard, cardShown, latestSaver, moveCardTo, removeCard, routinesByNextRun, setCardLimit, stepCard, taskCardQuery, visibleCards, type HomeCard } from './home'

const ids = (list: readonly HomeCard[]) => list.map((card) => card.id)
const member = ['my_tasks', 'due_soon', 'overdue', 'inbox', 'activity', 'unread_chat', 'recent_docs', 'recent_files', 'ask_donna', 'routines']

test('the server layout decodes as it is, and the edited list encodes back without the default row count', () => {
  const layout = JSON.parse('{"cards":[{"id":"overdue"},{"id":"inbox","limit":10}],"available":["overdue","inbox"],"customized":true}')
  expect(visibleCards(layout, true)).toEqual([{ id: 'overdue' }, { id: 'inbox', limit: 10 }])
  expect(JSON.stringify({ cards: setCardLimit(layout.cards, 'inbox', 7) })).toBe('{"cards":[{"id":"overdue"},{"id":"inbox"}]}')
  expect(setCardLimit(layout.cards, 'overdue', 15)[0]).toEqual({ id: 'overdue', limit: 15 })
})

test('a card the phone does not know is hidden but keeps its place when the list is edited', () => {
  const layout = { cards: [{ id: 'my_tasks' }, { id: 'weather' }, { id: 'inbox' }], available: [...member, 'weather'] }
  expect(ids(visibleCards(layout, true))).toEqual(['my_tasks', 'inbox'])
  expect(addableCards(layout, true)).not.toContain('weather')
  expect(ids(removeCard(layout.cards, 'my_tasks'))).toEqual(['weather', 'inbox'])
  // moving Inbox up passes over the hidden card to the shown neighbour
  expect(ids(stepCard(layout.cards, visibleCards(layout, true), 'inbox', -1))).toEqual(['inbox', 'my_tasks', 'weather'])
})

test('a guest sees and adds only the cards the server allows, even from an older optimistic list', () => {
  const guest = { cards: [{ id: 'my_tasks' }, { id: 'unread_chat' }], available: ['unread_chat'] }
  expect(ids(visibleCards(guest, false))).toEqual(['unread_chat'])
  expect(addableCards({ cards: [], available: ['unread_chat'] }, true)).toEqual(['unread_chat'])
  expect(cardShown('recent_docs', ['unread_chat'], true)).toBe(false)
})

test('Ask Donna shows only while the person can ask the agent, and stays saved meanwhile', () => {
  const layout = { cards: [{ id: 'ask_donna' }, { id: 'inbox' }], available: member }
  expect(ids(visibleCards(layout, false))).toEqual(['inbox'])
  expect(ids(visibleCards(layout, true))).toEqual(['ask_donna', 'inbox'])
  expect(addableCards({ cards: [], available: member }, false)).not.toContain('ask_donna')
  // removing another card while Ask Donna is hidden keeps it in the saved list
  expect(ids(removeCard(layout.cards, 'inbox'))).toEqual(['ask_donna'])
})

test('the picker offers the allowed cards that are not on Home, in catalog order', () => {
  expect(addableCards({ cards: [{ id: 'inbox' }, { id: 'my_tasks' }], available: member }, true)).toEqual(member.filter((id) => id !== 'inbox' && id !== 'my_tasks'))
  expect(ids(addCard([{ id: 'inbox' }], 'routines'))).toEqual(['inbox', 'routines'])
  expect(addCard([{ id: 'inbox', limit: 5 }], 'inbox')).toEqual([{ id: 'inbox', limit: 5 }])
})

test('reordering keeps each card once, with its options, and ignores moves past the ends', () => {
  const cards: HomeCard[] = [{ id: 'a' }, { id: 'b', limit: 5 }, { id: 'c' }]
  expect(ids(moveCardTo(cards, 'a', 'c'))).toEqual(['b', 'c', 'a'])
  expect(ids(moveCardTo(cards, 'c', 'a'))).toEqual(['c', 'a', 'b'])
  expect(moveCardTo(cards, 'x', 'a')).toEqual(cards)
  expect(stepCard(cards, cards, 'b', -1)[0]).toEqual({ id: 'b', limit: 5 })
  expect(stepCard(cards, cards, 'a', -1)).toEqual(cards)
  expect(stepCard(cards, cards, 'c', 1)).toEqual(cards)
})

test('task cards ask for the caller\'s open tasks, and other cards are not task queries', () => {
  const overdue = taskCardQuery('overdue', 5)
  expect(overdue).toMatchObject({ show_completed: 'none', limit: 5, order_by: 'due_date', order_direction: 'asc' })
  expect(JSON.stringify(overdue?.filter)).toContain('"assignee"')
  expect(taskCardQuery('inbox', 5)).toBeNull()
})

test('routines run soonest first, paused ones last', () => {
  const rows = routinesByNextRun([{ id: 'p', next_run_at: null }, { id: 'late', next_run_at: 9 }, { id: 'soon', next_run_at: 1 }], 3)
  expect(rows.map((row) => row.id)).toEqual(['soon', 'late', 'p'])
})

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

test('quick saves go one at a time, and only the newest waiting one is sent', async () => {
  const calls: { value: string; done: ReturnType<typeof deferred<string>> }[] = []
  const saver = latestSaver((value: string) => {
    const done = deferred<string>()
    calls.push({ value, done })
    return done.promise
  })
  const first = saver.save('one')
  const second = saver.save('two')
  const third = saver.save('three')
  expect(calls.map((call) => call.value)).toEqual(['one'])
  expect(await second).toEqual({ kind: 'superseded' })
  calls[0].done.resolve('saved one')
  expect(await first).toEqual({ kind: 'saved', result: 'saved one' })
  // the newest list follows only after the first answer, so the server gets them in order
  expect(calls.map((call) => call.value)).toEqual(['one', 'three'])
  expect(saver.pending()).toBe(true)
  calls[1].done.resolve('saved three')
  expect(await third).toEqual({ kind: 'saved', result: 'saved three' })
  expect(saver.pending()).toBe(false)
})

test('a failed save is reported, and the next one still goes', async () => {
  let fail = true
  const saver = latestSaver(async (value: string) => {
    if (fail) throw new Error('offline')
    return value
  })
  const failed = await saver.save('a')
  expect(failed.kind).toBe('failed')
  fail = false
  expect(await saver.save('b')).toEqual({ kind: 'saved', result: 'b' })
})

test('while the first save runs, a waiting save makes pending true when the first one settles', async () => {
  const first = deferred<string>()
  const second = deferred<string>()
  const saver = latestSaver((value: string) => (value === 'a' ? first.promise : second.promise))
  const a = saver.save('a')
  void saver.save('b')
  first.resolve('a')
  await a
  // the handler of the first answer must not show it: a newer list is on its way
  expect(saver.pending()).toBe(true)
  second.resolve('b')
})
