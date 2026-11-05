import { describe, expect, test } from 'bun:test'
import type { DriveNode } from './generated/types.gen'
import { foldersOf, missingParts, needsThumbnail, partRange, sameFile, scaledSize, uniqueZipPaths, zipPath, zipSafeName } from './drive'

const MIB = 1024 * 1024

describe('multipart resume', () => {
  const upload = { part_count: 3, part_size: 16 * MIB, size: 32 * MIB + 5 }

  test('parts cover the file once, with a short last part', () => {
    expect(partRange(1, 16 * MIB, upload.size)).toEqual([0, 16 * MIB])
    expect(partRange(3, 16 * MIB, upload.size)).toEqual([32 * MIB, 32 * MIB + 5])
  })

  test('only parts the bucket lacks or has at the wrong size are sent again', () => {
    expect(missingParts(upload, [])).toEqual([1, 2, 3])
    expect(missingParts(upload, [{ part_number: 2, size: 16 * MIB }])).toEqual([1, 3])
    // A part that broke off is shorter than planned.
    expect(missingParts(upload, [{ part_number: 1, size: 16 * MIB }, { part_number: 2, size: 7 }, { part_number: 3, size: 5 }])).toEqual([2])
    expect(missingParts(upload, [{ part_number: 1, size: 16 * MIB }, { part_number: 2, size: 16 * MIB }, { part_number: 3, size: 5 }])).toEqual([])
  })

  test('a single PUT is one part of the whole size', () => {
    const single = { part_count: 1, part_size: null, size: 10 }
    expect(missingParts(single, [])).toEqual([1])
    expect(missingParts(single, [{ part_number: 1, size: 10 }])).toEqual([])
  })

  test('a resumed upload takes only the same file', () => {
    expect(sameFile({ name: 'video.mp4', size: 3 }, { name: 'video.mp4', size: 3 })).toBe(true)
    expect(sameFile({ name: 'video.mp4', size: 3 }, { name: 'video.mp4', size: 4 })).toBe(false)
  })
})

test('a folder upload creates every folder once, parents first', () => {
  expect(foldersOf(['Photos/2024/a.jpg', 'Photos/b.jpg', 'Photos/2024/Trip/c.jpg', 'top.txt'])).toEqual(['Photos', 'Photos/2024', 'Photos/2024/Trip'])
})

describe('ZIP paths', () => {
  test('names are safe on every system', () => {
    expect(zipSafeName('Q1: plan?.pdf')).toBe('Q1_ plan_.pdf')
    expect(zipSafeName('notes. ')).toBe('notes')
    expect(zipSafeName('CON.txt')).toBe('_CON.txt')
    expect(zipSafeName('..')).toBe('_')
  })

  test('paths keep the folder tree under the zipped folder', () => {
    expect(zipPath('Projects', { folder_path: 'Alpha/Docs', node: { name: 'plan.txt' } })).toBe('Projects/Alpha/Docs/plan.txt')
    expect(zipPath('Projects', { folder_path: '', node: { name: 'a.txt' } })).toBe('Projects/a.txt')
    expect(zipPath('', { folder_path: '', node: { name: 'a.txt' } })).toBe('a.txt')
  })

  test('paths that would overwrite each other get a number', () => {
    expect(uniqueZipPaths(['a/x?.txt', 'a/x_.txt', 'a/X_.txt', 'b/x_.txt'].map((path) => path.replace('?', '_')))).toEqual([
      'a/x_.txt',
      'a/x_ (2).txt',
      'a/X_ (3).txt',
      'b/x_.txt',
    ])
  })
})

describe('thumbnails', () => {
  test('sizes keep the aspect ratio and never grow', () => {
    expect(scaledSize(4000, 3000, 480)).toEqual({ width: 480, height: 360 })
    expect(scaledSize(1080, 1920, 480)).toEqual({ width: 270, height: 480 })
    expect(scaledSize(100, 50, 480)).toEqual({ width: 100, height: 50 })
    expect(scaledSize(10000, 1, 480)).toEqual({ width: 480, height: 1 })
  })

  test('only images and videos without a finished thumbnail need one', () => {
    const file = (mime: string, media: DriveNode['media']) => ({ kind: 'file' as const, mime, name: 'f', media })
    expect(needsThumbnail(file('image/png', null))).toBe(true)
    expect(needsThumbnail(file('video/webm', { status: 'pending' }))).toBe(true)
    expect(needsThumbnail(file('image/png', { status: 'ready' }))).toBe(false)
    expect(needsThumbnail(file('image/heic', { status: 'failed' }))).toBe(false)
    expect(needsThumbnail(file('application/pdf', null))).toBe(false)
    expect(needsThumbnail({ kind: 'folder', mime: null, name: 'f', media: null })).toBe(false)
  })
})
