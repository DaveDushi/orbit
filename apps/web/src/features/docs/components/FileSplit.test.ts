import { expect, test } from 'bun:test'
import { FILE_SPLIT_MIN, defaultFileShare, fitFileShare, parseFileShare, revealFileShare, snapFileShare, stepFileShare } from './FileSplit'

test('a bad or old saved value means the default size, and a saved 0 or 1 stays', () => {
  for (const bad of [null, '', 'wide', 'NaN', 'Infinity', '-0.1', '1.5', '52', '[object Object]']) expect(parseFileShare(bad)).toBeNull()
  expect(parseFileShare('0')).toBe(0)
  expect(parseFileShare('1')).toBe(1)
  expect(parseFileShare('0.25')).toBe(0.25)
})

test('the default is the width the panel had before the divider', () => {
  expect(defaultFileShare(1000) * 1000).toBeCloseTo(520)
  expect(defaultFileShare(2000) * 2000).toBeCloseTo(760)
  expect(defaultFileShare(600) * 600).toBeCloseTo(360)
  expect(defaultFileShare(300)).toBe(1)
})

test('a drag snaps to the edge below the smallest useful size on either side', () => {
  expect(snapFileShare((FILE_SPLIT_MIN - 1) / 1000, 1000)).toBe(0)
  expect(snapFileShare(FILE_SPLIT_MIN / 1000, 1000)).toBe(FILE_SPLIT_MIN / 1000)
  expect(snapFileShare(1 - FILE_SPLIT_MIN / 1000, 1000)).toBe(1 - FILE_SPLIT_MIN / 1000)
  expect(snapFileShare(1 - (FILE_SPLIT_MIN - 1) / 1000, 1000)).toBe(1)
  // Too narrow for both sides: the nearer edge wins.
  expect(snapFileShare(0.2, 400)).toBe(0)
  expect(snapFileShare(0.8, 400)).toBe(1)
})

test('a smaller window keeps both sides useful and does not hide a side by itself', () => {
  expect(fitFileShare(0.1, 2000)).toBe(FILE_SPLIT_MIN / 2000)
  expect(fitFileShare(0.1, 1000)).toBe(FILE_SPLIT_MIN / 1000)
  expect(fitFileShare(0.9, 1000)).toBe(1 - FILE_SPLIT_MIN / 1000)
  expect(fitFileShare(0.5, 400)).toBe(1)
  expect(fitFileShare(0, 400)).toBe(0)
  expect(fitFileShare(1, 2000)).toBe(1)
  expect(fitFileShare(null, 2000) * 2000).toBeCloseTo(760)
})

test('the arrow keys leave a hidden side and do not stick in the snap', () => {
  expect(stepFileShare(0, 1000, 1)).toBe(FILE_SPLIT_MIN / 1000)
  expect(stepFileShare(1, 1000, -1)).toBe(1 - FILE_SPLIT_MIN / 1000)
  expect(stepFileShare(0, 1000, -1)).toBe(0)
  expect(stepFileShare(1, 1000, 1)).toBe(1)
  expect(stepFileShare(0.5, 1000, 1)).toBeCloseTo(0.516)
  expect(stepFileShare(FILE_SPLIT_MIN / 1000, 1000, -1)).toBe(0)
})

test('a click on a file shows it: only a size that hides the file changes', () => {
  expect(revealFileShare(0)).toBeNull()
  expect(revealFileShare(1)).toBe(1)
  expect(revealFileShare(0.3)).toBe(0.3)
  expect(revealFileShare(null)).toBeNull()
})
