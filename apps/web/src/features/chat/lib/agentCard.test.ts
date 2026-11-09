import { describe, expect, test } from 'bun:test'
import type { MessageBlock } from '@orbit/api/generated/types.gen'
import { agentCardOf, sentOf } from './agentCard'

const card = (spec: unknown, version = 1): MessageBlock[] => [{ type: 'ui', version, title: 'Open tasks', block_id: 'ui0', spec } as MessageBlock]
const good = {
  root: 'card',
  elements: {
    card: { type: 'Card', props: { title: 'Open tasks' }, children: ['chart'] },
    chart: { type: 'Chart', props: { kind: 'bar', series: [{ name: 'Tasks', points: [{ x: 'Todo', y: 3 }] }] } },
  },
}

describe('agentCardOf', () => {
  test('a card that fits the catalog renders', () => {
    const result = agentCardOf(card(good))
    expect(result === 'invalid' || result === null ? result : result.blockId).toBe('ui0')
  })

  test('a card this app cannot show falls back to the text', () => {
    expect(agentCardOf(card(good, 2))).toBe('invalid')
    expect(agentCardOf(card({ root: 'card', elements: { card: { type: 'Iframe', props: { src: 'https://x' } } } }))).toBe('invalid')
    // json-render checks props only for a one-component catalog: wrong props must still fail here.
    expect(agentCardOf(card({ root: 'card', elements: { card: { type: 'Chart', props: { kind: 'pie', series: [] } } } }))).toBe('invalid')
    expect(agentCardOf(card({ root: 'card', elements: { card: { type: 'Card', props: {}, children: ['card'] } } }))).toBe('invalid')
    expect(agentCardOf(card({ root: 'card', elements: { card: { type: 'Text', props: { text: { $state: '/secret' } } } } }))).toBe('invalid')
  })

  test('a card being written is live', () => {
    const live = agentCardOf([{ ...card(good)[0], live: true } as MessageBlock])
    expect(live !== 'invalid' && live !== null && live.live).toBe(true)
    const done = agentCardOf(card(good))
    expect(done !== 'invalid' && done !== null && done.live).toBe(false)
  })

  test('the sent buttons and forms of a card are those of its block, the last send of each counts', () => {
    const sent = sentOf(
      [
        { blockId: 'ui0', elementId: 'plan', values: null },
        { blockId: 'ui1', elementId: 'other', values: null },
        { blockId: 'ui0', elementId: 'form', values: { title: 'First' } },
        { blockId: 'ui0', elementId: 'form', values: { title: 'Second' } },
      ],
      'ui0',
    )
    expect([...sent.keys()]).toEqual(['plan', 'form'])
    expect(sent.get('form')).toEqual({ title: 'Second' })
    expect(sentOf(undefined, 'ui0').size).toBe(0)
  })

  test('a message without a card has none', () => {
    expect(agentCardOf([{ type: 'divider' }])).toBeNull()
    expect(agentCardOf(null)).toBeNull()
  })
})
