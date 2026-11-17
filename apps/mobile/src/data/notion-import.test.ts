import { expect, test } from 'bun:test'

import { coveredByParent, importDestination, importRows, importSelection, type NotionImportNode } from './notion-import'

const node = (notion_id: string, parent_id: string | null, title = notion_id) => ({ notion_id, parent_id, title, icon: null, kind: 'page', child_count: 0 }) as NotionImportNode

// `orphan` has a parent the token cannot see: it is a top page too
const nodes = [node('a', null), node('a1', 'a', 'Roadmap'), node('a11', 'a1'), node('b', null), node('orphan', 'hidden', 'road trip')]

test('the list shows the top pages, or every match of a search', () => {
  expect(importRows(nodes, '').map((item) => item.notion_id)).toEqual(['a', 'b', 'orphan'])
  expect(importRows(nodes, 'ROAD').map((item) => item.notion_id)).toEqual(['a1', 'orphan'])
})

test('a checked page covers everything under it', () => {
  expect(coveredByParent(nodes, new Set(['a']), 'a11')).toBe(true)
  expect(coveredByParent(nodes, new Set(['a11']), 'a')).toBe(false)
})

test('the selection is everything, or the checked pages no checked page covers', () => {
  expect(importSelection(nodes, new Set(['a', 'b', 'orphan']))).toEqual({ all: true })
  expect(importSelection(nodes, new Set(['a', 'a11', 'b']))).toEqual({ notion_ids: ['a', 'b'] })
  expect(importSelection(nodes, new Set(['a11']))).toEqual({ notion_ids: ['a11'] })
  expect(importSelection(nodes, new Set())).toBeNull()
  expect(importSelection(nodes, new Set(['gone']))).toBeNull()
})

test('the destination is Private or one teamspace', () => {
  expect(importDestination('private')).toEqual({ private: true })
  expect(importDestination('teamspace:t1')).toEqual({ teamspace_id: 't1' })
})
