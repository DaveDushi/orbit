import { expect, test } from 'bun:test'

import { markdownToText, splitTables } from './markdown'

test('keeps paragraphs and list structure, drops the markup', () => {
  const markdown = [
    '## Plan',
    '',
    'Ship **the importer** with _care_ and a [link](https://example.com).',
    '',
    '',
    '',
    '- first `snake_case` item',
    '  * nested ~~old~~',
    '- [ ] open',
    '- [x] done',
    '1. one',
    '> quoted *text*',
    '---',
    '```ts',
    'const **raw** = 1',
    '```',
    '![diagram](x.png) and file_name_here',
  ].join('\n')
  expect(markdownToText(markdown)).toBe(
    [
      'Plan',
      '',
      'Ship the importer with care and a link.',
      '',
      '• first snake_case item',
      '  • nested old',
      '☐ open',
      '☑ done',
      '1. one',
      'quoted text',
      '',
      'const **raw** = 1',
      'diagram and file_name_here',
    ].join('\n'),
  )
})

test('plain text passes through', () => {
  expect(markdownToText('A safe, idempotent development seed record.')).toBe('A safe, idempotent development seed record.')
})

test('tables come apart from the text around them, with their cells and right-aligned columns', () => {
  const text = ['Open orders:', '', '| Order | Total |', '| --- | ---: |', '| #SO-1 | 89.00 € |', '| #SO-2 |', 'Done.'].join('\n')
  expect(splitTables(text)).toEqual([
    { kind: 'text', text: 'Open orders:' },
    { kind: 'table', header: ['Order', 'Total'], rows: [['#SO-1', '89.00 €'], ['#SO-2', '']], right: [false, true] },
    { kind: 'text', text: 'Done.' },
  ])
  expect(splitTables('a | b\nno table here')).toEqual([{ kind: 'text', text: 'a | b\nno table here' }])
  expect(splitTables('')).toEqual([])
})
