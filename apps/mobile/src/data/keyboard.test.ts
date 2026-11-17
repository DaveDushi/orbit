import { describe, expect, test } from 'bun:test'

import { keyboardCover, sheetTop } from './keyboard'

// iPhone 16: an 852 pt screen, a 336 pt keyboard (top at 516), a 34 pt home indicator.
const keyboardTop = 852 - 336

describe('keyboardCover', () => {
  test('a full-screen composer above the home indicator is covered by the keyboard less the indicator', () => {
    expect(keyboardCover({ rootTop: 0, y: 107, height: 852 - 107 - 34, keyboardTop, screenHeight: 852 })).toBe(336 - 34)
  })

  test('in a page sheet the sheet top counts: a view measured from the sheet is lower on the screen', () => {
    // build 14: the sheet's 59 pt offset was left out, so the send button stayed 59 pt under the keyboard
    const sheet = 852 - 59
    const top = sheetTop(852, sheet)
    expect(top).toBe(59)
    const view = { y: 48, height: sheet - 48 - 34 }
    expect(keyboardCover({ rootTop: top, ...view, keyboardTop, screenHeight: 852 })).toBe(336 - 34)
    expect(keyboardCover({ rootTop: 0, ...view, keyboardTop, screenHeight: 852 })).toBe(336 - 34 - 59)
  })

  test('nothing is covered when the keyboard hides or the view ends above it', () => {
    expect(keyboardCover({ rootTop: 0, y: 0, height: 852 - 34, keyboardTop: 852, screenHeight: 852 })).toBe(0)
    expect(keyboardCover({ rootTop: 0, y: 100, height: 200, keyboardTop, screenHeight: 852 })).toBe(0)
  })

  test('a view measured too low is padded by the keyboard at most (build 19: the composer under the header)', () => {
    // the measure after a tapped notification put the view's bottom a keyboard lower than it is
    expect(keyboardCover({ rootTop: 0, y: 107, height: 852 - 107 - 34 + 336, keyboardTop, screenHeight: 852 })).toBe(336)
  })
})
