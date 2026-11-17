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
  /** The Orbit app's recognizer (desktop 0.1.10+): the microphone's peak level (0–1), about 10 times a second. */
  onlevel?: ((level: number) => void) | null
  /** The Orbit app's recognizer: the speech model's one-time download (`percent`), then the transcription after stop. */
  onprogress?: ((progress: { phase: string; percent?: number }) => void) | null
  start(): void
  stop(): void
  abort(): void
}

export type RecognitionConstructor = new () => Recognition

/** `transcribing`: stopped, the last words are still coming (at once in a browser, up to a second in the Windows app). */
export type VoiceStatus = 'unavailable' | 'denied' | 'idle' | 'listening' | 'transcribing'

/**
 * What a tap or an error must explain to the person. After a refusal (`not-allowed`) the microphone itself is
 * checked: `denied` (the site or app blocks it), `denied-system` (the OS blocks it), `speech-refused` (the microphone
 * works, but the speech recognizer still refuses).
 */
export type VoiceNotice = 'unavailable' | 'denied' | 'denied-system' | 'speech-refused' | 'no-microphone' | 'network' | 'failed'

/** What `getUserMedia` says about the microphone, with the error's name and text for the person to send us. */
export interface MicProbe {
  state: 'works' | 'system' | 'blocked' | 'none' | 'busy' | 'unknown'
  detail: string
}

/** Reads the result of `getUserMedia({ audio: true })`: `null` when it gave a stream. */
export function probeOutcome(error: unknown): MicProbe {
  if (error === null) return { state: 'works', detail: 'works' }
  const { name = 'Error', message = String(error) } = (error ?? {}) as { name?: string; message?: string }
  const detail = message ? `${name}: ${message}` : name
  // Chromium says "Permission denied by system" when the OS blocks it, "Permission denied" for the site.
  if (name === 'NotAllowedError' || name === 'SecurityError') return { state: /system/i.test(message) ? 'system' : 'blocked', detail }
  if (name === 'NotFoundError' || name === 'OverconstrainedError') return { state: 'none', detail }
  if (name === 'NotReadableError' || name === 'AbortError') return { state: 'busy', detail }
  return { state: 'unknown', detail }
}

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
  /** The microphone's level (0–1), only from a recognizer that reports it. */
  level: (level: number) => void
  /** The speech model's download in percent while it records, `null` when there is none (any more). */
  download: (percent: number | null) => void
  /** Asks for the microphone itself (`getUserMedia`, the stream stops at once). */
  probe: () => Promise<MicProbe>
  lang: string
}

/**
 * Tap to start, tap to stop. One dictation keeps the field's text from its start and every text it wrote: a change
 * that it did not write (typing, emoji, send) ends it and keeps the text.
 */
export function createDictation(Recognition: RecognitionConstructor | null, io: DictationIo) {
  let session: { recognition: Recognition; base: string; written: Set<string> } | null = null
  let status: VoiceStatus = Recognition ? 'idle' : 'unavailable'
  // Each start and each cancel counts: a microphone check that comes back later belongs to its turn only.
  let turn = 0

  const set = (next: VoiceStatus) => {
    status = next
    io.status(next)
  }

  /** Ends the dictation without its last words (they are not final yet). */
  const cancel = () => {
    turn++
    const current = session
    session = null
    current?.recognition.abort()
    io.download(null)
    if (status === 'listening' || status === 'transcribing') set('idle')
  }

  /**
   * The recognizer refused (`not-allowed`). Asking for the microphone itself shows why, and in WebView2 it also
   * raises the app's permission request, which allows and saves the microphone: then one more try can work.
   */
  function recheck(Speech: RecognitionConstructor, message: string, retried: boolean) {
    const mine = turn
    void io.probe().then((probe) => {
      if (session || turn !== mine) return
      if (probe.state === 'works' && !retried) return start(Speech, true)
      const detail = `speech: not-allowed${message ? ` (${message})` : ''}; microphone: ${probe.detail}`
      const notice: VoiceNotice =
        probe.state === 'works' ? 'speech-refused' : probe.state === 'system' ? 'denied-system' : probe.state === 'none' ? 'no-microphone' : probe.state === 'busy' ? 'failed' : 'denied'
      set(notice === 'no-microphone' || notice === 'failed' ? 'idle' : 'denied')
      io.notice(notice, detail)
    })
  }

  function start(Speech: RecognitionConstructor, retried = false) {
    turn++
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
        // It stays "listening" until the microphone check answers.
        session = null
        recognition.abort()
        recheck(Speech, event.message ?? '', retried)
      } else if (event.error === 'service-not-allowed' || event.error === 'language-not-supported') {
        // The Windows app's recognizer says what to turn on in `message`.
        set('unavailable')
        io.notice('unavailable', event.message || undefined)
      } else if (event.error === 'audio-capture') io.notice('no-microphone', event.message || undefined)
      else if (event.error === 'network') io.notice('network', event.message || undefined)
      else if (event.error !== 'aborted' && event.error !== 'no-speech') io.notice('failed', event.message || event.error)
    }
    recognition.onlevel = (level) => {
      if (session === current) io.level(level)
    }
    recognition.onprogress = ({ phase, percent }) => {
      if (session !== current) return
      if (phase === 'download') io.download(percent ?? 0)
      else if (phase === 'transcribing') {
        io.download(null)
        set('transcribing')
      }
    }
    // Every dictation ends here: a stop, an error, or the recognizer's own end after a silence.
    recognition.onend = () => {
      if (session !== current) return
      session = null
      io.download(null)
      if (status === 'listening' || status === 'transcribing') set('idle')
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
    /**
     * A tap: stop while it listens (the button shows "transcribing" at once, the last words still come; a tap then
     * does nothing), else start (also after "denied": it asks again).
     */
    toggle() {
      if (session && status === 'transcribing') return
      if (session) {
        set('transcribing')
        return session.recognition.stop()
      }
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
