/**
 * Voice input with the browser's Web Speech API (`SpeechRecognition`, `webkitSpeechRecognition` in Chrome, Edge,
 * Safari, WebView2 and WKWebView): the spoken words show live after the field's text and stay editable; nothing is
 * sent. Firefox has no recognizer: the button shows "not available".
 */

/** The part of `SpeechRecognition` that voice input uses (TypeScript's DOM types do not have it). */
export interface Recognition {
  lang: string
  continuous: boolean
  interimResults: boolean
  onresult: ((event: { results: ArrayLike<{ isFinal: boolean; 0?: { transcript: string } }> }) => void) | null
  onerror: ((event: { error: string; message?: string }) => void) | null
  onend: (() => void) | null
  start(): void
  stop(): void
  abort(): void
}

export type RecognitionConstructor = new () => Recognition

export type VoiceStatus = 'unavailable' | 'denied' | 'idle' | 'listening'

/** What a tap or an error must explain to the person. */
export type VoiceNotice = 'unavailable' | 'denied' | 'no-microphone' | 'network' | 'failed'

/** The browser's recognizer, or null (Firefox, WebKitGTK). */
export function speechRecognition(scope: object = globalThis): RecognitionConstructor | null {
  const found = scope as { SpeechRecognition?: RecognitionConstructor; webkitSpeechRecognition?: RecognitionConstructor }
  return found.SpeechRecognition ?? found.webkitSpeechRecognition ?? null
}

/**
 * Joins the field's text and the spoken pieces with one space between them. The field's text stays as it is (a
 * trailing space or line break too), the spoken pieces are trimmed, and empty pieces add nothing.
 */
export function joinSpeech(text: string, ...spoken: string[]): string {
  return spoken.reduce((joined, part) => {
    const piece = part.trim()
    if (!piece) return joined
    if (!joined || /\s$/.test(joined)) return joined + piece
    return `${joined} ${piece}`
  }, text)
}

interface DictationIo {
  /** The field's text now. */
  text: () => string
  setText: (text: string) => void
  status: (status: VoiceStatus) => void
  notice: (notice: VoiceNotice, detail?: string) => void
  lang: string
}

/**
 * Tap to start, tap to stop. One dictation keeps the field's text from its start and every text it wrote: a change
 * that it did not write (typing, emoji, send) ends it and keeps the text.
 */
export function createDictation(Recognition: RecognitionConstructor | null, io: DictationIo) {
  let session: { recognition: Recognition; base: string; written: Set<string> } | null = null
  let status: VoiceStatus = Recognition ? 'idle' : 'unavailable'

  const set = (next: VoiceStatus) => {
    status = next
    io.status(next)
  }

  /** Ends the dictation without its last words (they are not final yet). */
  const cancel = () => {
    const current = session
    session = null
    current?.recognition.abort()
    if (status === 'listening') set('idle')
  }

  function start(Speech: RecognitionConstructor) {
    const recognition = new Speech()
    recognition.lang = io.lang
    recognition.continuous = true
    recognition.interimResults = true
    const base = io.text()
    const current = { recognition, base, written: new Set([base]) }
    // The results list holds every segment of this dictation: final ones and the one that is still changing.
    recognition.onresult = (event) => {
      if (session !== current) return
      let finals = ''
      let partial = ''
      for (const result of Array.from(event.results)) {
        const said = result[0]?.transcript ?? ''
        if (result.isFinal) finals = joinSpeech(finals, said)
        else partial = joinSpeech(partial, said)
      }
      const next = joinSpeech(base, finals, partial)
      current.written.add(next)
      io.setText(next)
    }
    recognition.onerror = (event) => {
      if (session !== current) return
      if (event.error === 'not-allowed') {
        set('denied')
        io.notice('denied')
      } else if (event.error === 'service-not-allowed' || event.error === 'language-not-supported') {
        set('unavailable')
        io.notice('unavailable')
      } else if (event.error === 'audio-capture') io.notice('no-microphone')
      else if (event.error === 'network') io.notice('network')
      else if (event.error !== 'aborted' && event.error !== 'no-speech') io.notice('failed', event.message || event.error)
    }
    // Every dictation ends here: a stop, an error, or the recognizer's own end after a silence.
    recognition.onend = () => {
      if (session !== current) return
      session = null
      if (status === 'listening') set('idle')
    }
    session = current
    set('listening')
    try {
      recognition.start()
    } catch (error) {
      session = null
      set('idle')
      io.notice('failed', error instanceof Error ? error.message : String(error))
    }
  }

  return {
    /** A tap: stop while it listens (the last words still come), else start (also after "denied": it asks again). */
    toggle() {
      if (session) return session.recognition.stop()
      if (!Recognition) return io.notice('unavailable')
      start(Recognition)
    },
    /** The field's text changed: a change it did not write ends the dictation. */
    edited(text: string) {
      if (session && !session.written.has(text)) cancel()
    },
    /** The microphone is blocked for this site (Permissions API): the button shows it before a tap. */
    blocked() {
      if (status === 'idle') set('denied')
    },
    dispose: cancel,
  }
}
