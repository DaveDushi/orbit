import { describe, expect, test } from 'bun:test'
import { createDictation, joinSpeech, speechRecognition, type Recognition, type VoiceNotice, type VoiceStatus } from './voiceInput'

/** A recognizer the test drives: `say` sends the results list, `fail` an error, `end` the end. */
class FakeRecognition implements Recognition {
  static last: FakeRecognition | null = null
  lang = ''
  continuous = false
  interimResults = false
  onresult: Recognition['onresult'] = null
  onerror: Recognition['onerror'] = null
  onend: Recognition['onend'] = null
  started = false
  stopped = false
  aborted = false
  constructor() {
    FakeRecognition.last = this
  }
  start() {
    this.started = true
  }
  stop() {
    this.stopped = true
  }
  abort() {
    this.aborted = true
  }
  say(...results: [string, boolean][]) {
    this.onresult?.({ results: results.map(([transcript, isFinal]) => ({ isFinal, 0: { transcript } })) })
  }
  fail(error: string) {
    this.onerror?.({ error })
    this.onend?.()
  }
  end() {
    this.onend?.()
  }
}

function setup(text = '', Recognition: typeof FakeRecognition | null = FakeRecognition) {
  const field = { text, statuses: [] as VoiceStatus[], notices: [] as VoiceNotice[] }
  const dictation = createDictation(Recognition, {
    text: () => field.text,
    setText: (next) => {
      field.text = next
      dictation.edited(next)
    },
    status: (status) => field.statuses.push(status),
    notice: (notice) => field.notices.push(notice),
    lang: 'de-CH',
  })
  return { field, dictation, recognition: () => FakeRecognition.last! }
}

describe('joinSpeech', () => {
  test('adds one space between the typed text and the spoken text', () => {
    expect(joinSpeech('Hello', 'world')).toBe('Hello world')
    expect(joinSpeech('Hello ', ' world ')).toBe('Hello world')
    expect(joinSpeech('Hello\n', 'world')).toBe('Hello\nworld')
  })

  test('empty pieces add nothing', () => {
    expect(joinSpeech('', '', 'hi')).toBe('hi')
    expect(joinSpeech('Hello', '', '  ')).toBe('Hello')
  })
})

describe('speechRecognition', () => {
  test('takes the standard name, then the prefixed one, else none (Firefox)', () => {
    expect(speechRecognition({ SpeechRecognition: FakeRecognition })).toBe(FakeRecognition)
    expect(speechRecognition({ webkitSpeechRecognition: FakeRecognition })).toBe(FakeRecognition)
    expect(speechRecognition({})).toBeNull()
  })
})

describe('createDictation', () => {
  test('a tap starts a continuous dictation with live results in the language given', () => {
    const { field, dictation, recognition } = setup()
    dictation.toggle()
    expect(recognition()).toMatchObject({ started: true, continuous: true, interimResults: true, lang: 'de-CH' })
    expect(field.statuses).toEqual(['listening'])
  })

  test('partial words replace each other, final ones stay, all after the typed text', () => {
    const { field, dictation, recognition } = setup('Note:')
    dictation.toggle()
    recognition().say(['first', false])
    expect(field.text).toBe('Note: first')
    recognition().say(['first part.', true], [' sec', false])
    expect(field.text).toBe('Note: first part. sec')
    recognition().say(['first part.', true], [' second', true])
    expect(field.text).toBe('Note: first part. second')
  })

  test('a second tap stops (the last words still come), and the end makes it idle; nothing else happens', () => {
    const { field, dictation, recognition } = setup()
    dictation.toggle()
    dictation.toggle()
    expect(recognition().stopped).toBe(true)
    recognition().say(['hello', true])
    recognition().end()
    expect(field.text).toBe('hello')
    expect(field.statuses).toEqual(['listening', 'idle'])
  })

  test('a change it did not write (typing, send) ends it and keeps the text', () => {
    const { field, dictation, recognition } = setup()
    dictation.toggle()
    recognition().say(['hello', false])
    field.text = 'hello!'
    dictation.edited(field.text)
    expect(recognition().aborted).toBe(true)
    recognition().say(['hello there', true])
    expect(field.text).toBe('hello!')
    expect(field.statuses).toEqual(['listening', 'idle'])
  })

  test('a refused microphone shows "denied" and explains; the next tap asks again', () => {
    const { field, dictation, recognition } = setup()
    dictation.toggle()
    recognition().fail('not-allowed')
    expect(field.statuses).toEqual(['listening', 'denied'])
    expect(field.notices).toEqual(['denied'])
    dictation.toggle()
    expect(field.statuses.at(-1)).toBe('listening')
  })

  test('a blocked microphone (Permissions API) shows "denied" before a tap', () => {
    const { field, dictation } = setup()
    dictation.blocked()
    expect(field.statuses).toEqual(['denied'])
  })

  test('no speech service shows "not available"; silence and network errors leave it idle', () => {
    const { field, dictation, recognition } = setup()
    dictation.toggle()
    recognition().fail('service-not-allowed')
    expect(field.statuses).toEqual(['listening', 'unavailable'])
    dictation.toggle()
    recognition().fail('no-speech')
    dictation.toggle()
    recognition().fail('network')
    expect(field.statuses).toEqual(['listening', 'unavailable', 'listening', 'idle', 'listening', 'idle'])
    expect(field.notices).toEqual(['unavailable', 'network'])
  })

  test('without a recognizer a tap only explains', () => {
    FakeRecognition.last = null
    const { field, dictation } = setup('typed', null)
    dictation.toggle()
    expect(FakeRecognition.last).toBeNull()
    expect(field.statuses).toEqual([])
    expect(field.notices).toEqual(['unavailable'])
    expect(field.text).toBe('typed')
  })

  test('dispose aborts a running dictation', () => {
    const { dictation, recognition } = setup()
    dictation.toggle()
    dictation.dispose()
    expect(recognition().aborted).toBe(true)
  })
})
