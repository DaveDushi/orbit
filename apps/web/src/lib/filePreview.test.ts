import { expect, test } from 'bun:test'
import type { FilePreview } from '@orbit/api/generated/types.gen'
import { hasViewer, isSheetFile, previewView } from './filePreview'

const url = '/api/v1/workspaces/w/chat/files/f'
const preview = (change: Partial<FilePreview>): FilePreview => ({ kind: 'video', status: 'pending', play_url: null, poster_url: null, pdf_url: null, ...change })

test('a video shows "processing", then its playable copy, and its original when no copy is made', () => {
  const file = { mimeType: 'video/quicktime', fileName: 'IMG_1.MOV', url }
  expect(previewView({ ...file, preview: preview({}) })).toEqual({ kind: 'pending', label: 'Processing the video…' })
  expect(previewView({ ...file, preview: preview({ status: 'ready', play_url: `${url}/preview/play`, poster_url: `${url}/preview/poster` }) })).toEqual({
    kind: 'video',
    src: `${url}/preview/play`,
    poster: `${url}/preview/poster`,
  })
  for (const status of ['failed', 'skipped', 'none'] as const) {
    expect(previewView({ ...file, preview: preview({ status, poster_url: `${url}/preview/poster` }) })).toEqual({
      kind: 'video',
      src: `${url}?inline=true`,
      poster: `${url}/preview/poster`,
    })
  }
  // A file the browser labelled vaguely is still a video when the server makes a playable copy of it.
  expect(previewView({ ...file, mimeType: 'application/octet-stream', preview: preview({ status: 'ready', play_url: `${url}/preview/play` }) }).kind).toBe('video')
})

test('an office file shows its PDF, waits while it is made, and says when there is none', () => {
  const file = { mimeType: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet', fileName: 'Sales.xlsx', url }
  const document = (status: FilePreview['status'], pdf: string | null = null) => preview({ kind: 'document', status, pdf_url: pdf })
  expect(previewView({ ...file, preview: document('ready', `${url}/preview/pdf`) })).toEqual({ kind: 'pdf', src: `${url}/preview/pdf` })
  expect(previewView({ ...file, preview: document('pending') }).kind).toBe('pending')
  expect(previewView({ ...file, preview: document('none') }).kind).toBe('pending')
  expect(previewView({ ...file, preview: document('skipped') }).kind).toBe('unavailable')
  expect(previewView({ ...file, preview: document('failed') }).kind).toBe('unavailable')
  expect(hasViewer({ ...file, preview: document('skipped') })).toBe(true)
  expect(hasViewer({ mimeType: 'application/zip', fileName: 'a.zip', preview: null })).toBe(false)
  expect(previewView({ mimeType: 'application/pdf', fileName: 'a.pdf', url })).toEqual({ kind: 'pdf', src: url })
})

test('workbooks and delimited text open as a sheet grid, other office files keep their PDF', () => {
  for (const name of ['Plan.XLSX', 'a.xlsm', 'old.xls', 'export.csv', 'data.tsv']) expect(isSheetFile(name)).toBe(true)
  for (const name of ['report.docx', 'deck.pptx', 'notes.txt', 'xlsx', 'a.csv.zip']) expect(isSheetFile(name)).toBe(false)
  // A csv gets no PDF from the server, but it opens in the viewer as a table.
  expect(hasViewer({ mimeType: 'text/csv', fileName: 'export.csv', preview: null })).toBe(true)
})
