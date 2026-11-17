import { describe, expect, test } from 'bun:test'
import { createDictation, joinSpeech, probeOutcome, speechRecognition, type MicProbe, type Recognition, type VoiceNotice, type VoiceStatus } from './voiceInput'

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

function setup(text = '', Recognition: typeof FakeRecognition | null = FakeRecognition, probes: MicProbe[] = []) {
  const field = { text, statuses: [] as VoiceStatus[], notices: [] as VoiceNotice[], details: [] as (string | undefined)[], probed: 0 }
  const dictation = createDictation(Recognition, {
    text: () => field.text,
    setText: (next) => {
      field.text = next
      dictation.edited(next)
    },
    status: (status) => field.statuses.push(status),
    notice: (notice, detail) => {
      field.notices.push(notice)
      field.details.push(detail)
    },
    probe: () => {
      field.probed++
      return Promise.resolve(probes.shift() ?? { state: 'blocked', detail: 'NotAllowedError: Permission denied' })
    },
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

describe('probeOutcome', () => {
  test('tells the OS, the site, a missing and a busy microphone apart', () => {
    const error = (name: string, message: string) => Object.assign(new Error(message), { name })
    expect(probeOutcome(null).state).toBe('works')
    expect(probeOutcome(error('NotAllowedError', 'Permission denied by system'))).toEqual({ state: 'system', detail: 'NotAllowedError: Permission denied by system' })
    expect(probeOutcome(error('NotAllowedError', 'Permission denied')).state).toBe('blocked')
    expect(probeOutcome(error('NotFoundError', 'Requested device not found')).state).toBe('none')
    expect(probeOutcome(error('NotReadableError', 'Could not start audio source')).state).toBe('busy')
    expect(probeOutcome('odd').state).toBe('unknown')
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

  test('a refused microphone shows "denied" and explains with the microphone check; the next tap asks again', async () => {
    const { field, dictation, recognition } = setup()
    dictation.toggle()
    recognition().fail('not-allowed')
    expect(field.statuses).toEqual(['listening'])
    await Promise.resolve()
    expect(field.statuses).toEqual(['listening', 'denied'])
    expect(field.notices).toEqual(['denied'])
    expect(field.details).toEqual(['speech: not-allowed; microphone: NotAllowedError: Permission denied'])
    dictation.toggle()
    expect(field.statuses.at(-1)).toBe('listening')
  })

  // The Windows app (WebView2): the recognizer said `not-allowed`, but the microphone check allowed it.
  test('when the microphone works after a refusal, it tries once more, and says so if the recognizer still refuses', async () => {
    const works: MicProbe = { state: 'works', detail: 'works' }
    const { field, dictation, recognition } = setup('', FakeRecognition, [works, works])
    dictation.toggle()
    const first = recognition()
    first.fail('not-allowed')
    expect(first.aborted).toBe(true)
    await Promise.resolve()
    const second = recognition()
    expect(second).not.toBe(first)
    expect(second.started).toBe(true)
    second.say(['hello', true])
    expect(field.text).toBe('hello')
    second.fail('not-allowed')
    await Promise.resolve()
    expect(recognition()).toBe(second)
    expect(field.probed).toBe(2)
    expect(field.notices).toEqual(['speech-refused'])
    expect(field.statuses.at(-1)).toBe('denied')
  })

  test('an OS block and a missing microphone are told apart', async () => {
    const { field, dictation, recognition } = setup('', FakeRecognition, [
      { state: 'system', detail: 'NotAllowedError: Permission denied by system' },
      { state: 'none', detail: 'NotFoundError: Requested device not found' },
    ])
    dictation.toggle()
    recognition().fail('not-allowed')
    await Promise.resolve()
    dictation.toggle()
    recognition().fail('not-allowed')
    await Promise.resolve()
    expect(field.notices).toEqual(['denied-system', 'no-microphone'])
    expect(field.statuses).toEqual(['listening', 'denied', 'listening', 'idle'])
  })

  test('a tap or a dispose while the microphone is checked drops the late answer', async () => {
    const { field, dictation, recognition } = setup()
    dictation.toggle()
    recognition().fail('not-allowed')
    dictation.dispose()
    await Promise.resolve()
    expect(field.notices).toEqual([])
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
