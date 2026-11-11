import { describe, expect, test } from 'bun:test'
import type { DriveFileReference, DriveNode } from '@orbit/api/generated/types.gen'
import { toEditorContent, toStoredContent } from './content'
import { driveFileView } from './driveFileView'
import { driveFileBlock } from './pageBlockCommands'
import { EDITOR_BLOCK_TYPES, TASK_EDITOR_BLOCK_TYPES } from './schema'
import { pageSlashItems } from './slashMenu'

const WS = '0199c3a0-0000-7000-8000-0000000000f0'
const NODE = '0199c3a0-0000-7000-8000-0000000000f1'
const ref = { workspaceId: WS, nodeId: NODE }

function file(name: string, mime: string): DriveNode {
  return {
    id: NODE,
    parent_id: null,
    kind: 'file',
    name,
    size: 10,
    mime,
    created_by: null,
    created_at: '2026-11-11T00:00:00Z',
    updated_by: null,
    updated_at: '2026-11-11T00:00:00Z',
    trashed_at: null,
    media: null,
  }
}

function answer(data: Partial<DriveFileReference> & Pick<DriveFileReference, 'status'>) {
  return { data: { workspace_id: WS, node_id: NODE, file: null, url: null, ...data }, isError: false, hosted: true }
}

describe('driveFile blocks', () => {
  test('a page stores the reference (workspace and node), never the file; task text drops the block', () => {
    const content = [{ id: 'b1', ...driveFileBlock(WS, NODE), children: [] }]
    expect(toStoredContent(toEditorContent(content, EDITOR_BLOCK_TYPES) ?? [])).toEqual(content)
    expect(toEditorContent([...content, { type: 'paragraph', content: [] }], TASK_EDITOR_BLOCK_TYPES)).toEqual([{ type: 'paragraph', content: [] }])
  })

  test('the view follows the server: inline media, a card for other files, locked and unavailable without a name', () => {
    expect(driveFileView(ref, answer({ status: 'available', file: file('a.png', 'image/png'), url: 'https://s3/a' })).kind).toBe('image')
    expect(driveFileView(ref, answer({ status: 'available', file: file('a.mp4', 'video/mp4'), url: 'https://s3/a' })).kind).toBe('video')
    expect(driveFileView(ref, answer({ status: 'available', file: file('a.pdf', 'application/pdf') })).kind).toBe('card')
    // Media without a URL (no bucket) falls back to the card.
    expect(driveFileView(ref, answer({ status: 'available', file: file('a.png', 'image/png') })).kind).toBe('card')
    expect(driveFileView(ref, answer({ status: 'locked' }))).toEqual({ kind: 'locked' })
    expect(driveFileView(ref, answer({ status: 'unavailable' }))).toEqual({ kind: 'unavailable' })
    expect(driveFileView(ref, { isError: true, hosted: true })).toEqual({ kind: 'unavailable' })
    expect(driveFileView(ref, { isError: false, hosted: true })).toEqual({ kind: 'loading' })
    expect(driveFileView(ref, { isError: false, hosted: false })).toEqual({ kind: 'placeholder' })
  })

  test('"Insert from Drive" is offered only where the editor has a Drive', () => {
    const actions = { onSubpage: () => {}, onLinkPage: () => {} }
    expect(pageSlashItems(actions).map((item) => item.title)).not.toContain('Insert from Drive')
    expect(pageSlashItems({ ...actions, onInsertDriveFile: () => {} }).map((item) => item.title)).toContain('Insert from Drive')
  })
})
