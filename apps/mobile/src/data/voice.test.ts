import { describe, expect, test } from 'bun:test'

import { joinSpeech, pickLocale } from './voice'

describe('joinSpeech', () => {
  test('adds one space between the typed text and the spoken text', () => {
    expect(joinSpeech('Hello', 'world')).toBe('Hello world')
    expect(joinSpeech('Hello ', ' world ')).toBe('Hello world')
    expect(joinSpeech('Hello', ' world ')).toBe('Hello world')
  })

  test('keeps a line break the typed text ends with', () => {
    expect(joinSpeech('Hello\n', 'world')).toBe('Hello\nworld')
  })

  test('final segments and the current partial follow each other', () => {
    expect(joinSpeech('Note:', 'first part.', 'second')).toBe('Note: first part. second')
  })

  test('empty pieces add nothing', () => {
    expect(joinSpeech('', '', 'hi')).toBe('hi')
    expect(joinSpeech('Hello', '', '  ')).toBe('Hello')
    expect(joinSpeech('', '')).toBe('')
  })
})

describe('pickLocale', () => {
  const ios = ['en-US', 'en-GB', 'de-DE', 'de-CH', 'fr-FR']

  test('takes the device locale when the recognizer has it', () => {
    expect(pickLocale('de-CH', ios)).toBe('de-CH')
    expect(pickLocale('de_CH', ios)).toBe('de-CH')
  })

  test('drops Unicode extensions of the locale', () => {
    expect(pickLocale('de-CH-u-ca-gregory', ios)).toBe('de-CH')
  })

  test('falls back to a locale of the same language', () => {
    expect(pickLocale('de-AT', ios)).toBe('de-DE')
  })

  test('falls back to en-US for a language the recognizer lacks or no locale', () => {
    expect(pickLocale('he-IL', ios)).toBe('en-US')
    expect(pickLocale(undefined, ios)).toBe('en-US')
  })

  test('without a list (the web) the device locale is used as it is', () => {
    expect(pickLocale('fr-FR', [])).toBe('fr-FR')
  })
})
