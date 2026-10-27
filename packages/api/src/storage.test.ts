import { expect, test } from 'bun:test'
import { storageFileLabel } from './storage'

test('a stored file names each place once and hides private names', () => {
  expect(storageFileLabel({
    byte_size: 1,
    name: 'demo.mp4',
    uses: [{ kind: 'task', title: 'Launch' }, { kind: 'page', title: '' }, { kind: 'channel', title: 'general' }, { kind: 'private' }, { kind: 'private' }],
  })).toEqual({ name: 'demo.mp4', where: 'Task “Launch”, Page “Untitled”, #general, Private page or chat' })
  expect(storageFileLabel({ byte_size: 1, name: null, uses: [{ kind: 'private' }] })).toEqual({ name: 'Private file', where: 'Private page or chat' })
  expect(storageFileLabel({ byte_size: 1, name: null, uses: [] })).toEqual({ name: 'Deleted file', where: 'Deleted · stops counting within a day' })
})
