import { describe, expect, test } from 'bun:test'

import { docBlocks, docSpaces, pageTitle } from './docs'
import type { PageSummary, Teamspace } from './orbit'

const page = (id: string, fields: Partial<PageSummary>): PageSummary => ({
  id,
  title: id,
  icon: null,
  parent_id: null,
  position: 0,
  private: false,
  teamspace_id: 'ts-general',
  updated_at: '2026-10-15T10:00:00Z',
  version: 1,
  ...fields,
})

const teamspace = (id: string, name: string, position: number): Teamspace => ({
  id,
  name,
  position,
  icon: null,
  is_default: position === 0,
  version: 0,
  workspace_id: 'w',
  created_at: '2026-10-15T10:00:00Z',
  updated_at: '2026-10-15T10:00:00Z',
})

describe('docSpaces', () => {
  const pages = [
    page('roadmap', { position: 2 }),
    page('notes', { position: 1 }),
    page('q4', { parent_id: 'roadmap', position: 0 }),
    page('design-doc', { teamspace_id: 'ts-design' }),
    page('diary', { private: true, teamspace_id: null }),
  ]
  const teamspaces = [teamspace('ts-design', 'Design', 1), teamspace('ts-general', 'General', 0)]

  test('teamspaces by position, pages depth first by position, private pages apart', () => {
    const spaces = docSpaces(pages, teamspaces, new Set())
    expect(spaces.teamspaces.map((space) => [space.name, space.rows.map((row) => `${'-'.repeat(row.depth)}${row.page.id}${row.hasChildren ? '+' : ''}`)])).toEqual([
      ['General', ['notes', 'roadmap+', '-q4']],
      ['Design', ['design-doc']],
    ])
    expect(spaces.private.rows.map((row) => row.page.id)).toEqual(['diary'])
  })

  test('a collapsed page hides its children', () => {
    expect(docSpaces(pages, teamspaces, new Set(['roadmap'])).teamspaces[0]!.rows.map((row) => row.page.id)).toEqual(['notes', 'roadmap'])
  })
})

test('an empty title reads Untitled', () => {
  expect(pageTitle({ title: '  ' })).toBe('Untitled')
})

describe('docBlocks', () => {
  test('blocks become lines with their kind, numbering and nesting', () => {
    const blocks = docBlocks([
      { id: 'h', type: 'heading', props: { level: 2 }, content: [{ type: 'text', text: 'Plan', styles: {} }], children: [] },
      { id: 'p', type: 'paragraph', content: [{ type: 'text', text: 'See ', styles: {} }, { type: 'link', href: 'https://x', content: [{ type: 'text', text: 'the doc', styles: {} }] }] },
      { id: 'n1', type: 'numberedListItem', content: [{ type: 'text', text: 'One' }], children: [{ id: 'b', type: 'bulletListItem', content: [{ type: 'text', text: 'Detail' }] }] },
      { id: 'n2', type: 'numberedListItem', content: [{ type: 'text', text: 'Two' }] },
      { id: 'c', type: 'checkListItem', props: { checked: true }, content: [{ type: 'text', text: 'Done' }] },
      { id: 'm', type: 'paragraph', content: [{ type: 'mention', props: { label: 'Ada' } }] },
      { id: 'i', type: 'image', props: { caption: 'Diagram' } },
      { id: 'e', type: 'paragraph', content: [] },
    ])
    expect(blocks).toEqual([
      { id: 'h', kind: 'heading', text: 'Plan', depth: 0, level: 2 },
      { id: 'p', kind: 'paragraph', text: 'See the doc', depth: 0 },
      { id: 'n1', kind: 'numbered', text: 'One', depth: 0, number: 1 },
      { id: 'b', kind: 'bullet', text: 'Detail', depth: 1 },
      { id: 'n2', kind: 'numbered', text: 'Two', depth: 0, number: 2 },
      { id: 'c', kind: 'check', text: 'Done', depth: 0, checked: true },
      { id: 'm', kind: 'paragraph', text: '@Ada', depth: 0 },
      { id: 'i', kind: 'other', text: 'Diagram', depth: 0 },
      { id: 'e', kind: 'paragraph', text: '', depth: 0 },
    ])
  })
})
