import { describe, expect, test } from 'bun:test'
import { closedPanel, panelIn, panelReducer, type PanelState } from './panelState'

const file = new File(['x'], 'a.txt')

describe('panelReducer', () => {
  test('closing keeps the conversation, and opening again shows it', () => {
    const open = panelReducer(closedPanel, { type: 'open', workspaceId: 'w1', conversationId: 'dm1' })
    expect(open).toEqual({ open: true, workspaceId: 'w1', conversationId: 'dm1', files: [] })
    const closed = panelReducer(open, { type: 'close' })
    expect(closed).toEqual({ open: false, workspaceId: 'w1', conversationId: 'dm1', files: [] })
    expect(panelReducer(closed, { type: 'open', workspaceId: 'w1', conversationId: 'dm1' })).toEqual(open)
  })

  test('files from the bar wait for the composer, once', () => {
    const open = panelReducer(closedPanel, { type: 'open', workspaceId: 'w1', conversationId: 'dm1', files: [file] })
    expect(open.files).toEqual([file])
    // a second open without files (Enter on the bar) keeps them until the composer takes them
    expect(panelReducer(open, { type: 'open', workspaceId: 'w1', conversationId: 'dm1' }).files).toEqual([file])
    const taken = panelReducer(open, { type: 'filesTaken' })
    expect(taken.files).toEqual([])
    expect(panelReducer(taken, { type: 'filesTaken' })).toBe(taken)
    // closing drops files that were not taken
    expect(panelReducer(open, { type: 'close' }).files).toEqual([])
  })

  test('another workspace sees a closed panel without the DM of the first', () => {
    const open: PanelState = panelReducer(closedPanel, { type: 'open', workspaceId: 'w1', conversationId: 'dm1', files: [file] })
    expect(panelIn(open, 'w1')).toBe(open)
    expect(panelIn(open, 'w2')).toEqual(closedPanel)
    const other = panelReducer(open, { type: 'open', workspaceId: 'w2', conversationId: 'dm2' })
    expect(other).toEqual({ open: true, workspaceId: 'w2', conversationId: 'dm2', files: [] })
  })
})
