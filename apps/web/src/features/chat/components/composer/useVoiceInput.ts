import { useEffect, useRef, useState, type RefObject } from 'react'
import { createDictation, probeOutcome, speechRecognition, type MicProbe, type VoiceNotice, type VoiceStatus } from './voiceInput'
import { explain } from './voiceNotice'

interface Latest {
  text: string
  setText: (text: string) => void
  onNotice: (notice: VoiceNotice, detail?: string) => void
}

/** How long an error stays beside the mic (the toast has the details). */
const NOTICE_MS = 6000

/** Asks for the microphone and lets it go at once: the answer says why the recognizer refused. */
async function probeMicrophone(): Promise<MicProbe> {
  if (!navigator.mediaDevices?.getUserMedia) return { state: 'unknown', detail: 'no getUserMedia' }
  try {
    const stream = await navigator.mediaDevices.getUserMedia({ audio: true })
    for (const track of stream.getTracks()) track.stop()
    return probeOutcome(null)
  } catch (error) {
    return probeOutcome(error)
  }
}

/**
 * The microphone's level (0–1; -1 while the recognizer has not reported one, as in a browser), outside React state: only
 * the meter re-renders 10 times a second, not the whole field.
 */
export interface LevelStore {
  get: () => number
  set: (level: number) => void
  subscribe: (listener: () => void) => () => void
}

function levelStore(): LevelStore {
  let level = -1
  const listeners = new Set<() => void>()
  return {
    get: () => level,
    set(next) {
      level = next
      for (const listener of listeners) listener()
    },
    subscribe(listener) {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
  }
}

interface Hooks {
  status: (status: VoiceStatus) => void
  level: (level: number) => void
  download: (percent: number | null) => void
  noticed: (notice: VoiceNotice) => void
}

/** The dictation reads the field of the last render. */
function fieldOf(latest: RefObject<Latest>, hooks: Hooks) {
  return {
    text: () => latest.current.text,
    setText: (next: string) => latest.current.setText(next),
    status: hooks.status,
    level: hooks.level,
    download: hooks.download,
    notice: (notice: VoiceNotice, detail?: string) => {
      hooks.noticed(notice)
      latest.current.onNotice(notice, detail)
    },
    probe: probeMicrophone,
    lang: navigator.language,
  }
}

/**
 * Voice input for a text field: `toggle` starts and stops it, and the words go into the field with `setText`.
 * `onNotice` explains a refusal or an error. `since` is when the listening started (for its timer), `download` the
 * speech model's one-time download in percent (the Orbit app on Windows) and `level` the microphone's level. `notice`
 * is the last refusal or error for a few seconds, to show beside the mic.
 */
export function useVoiceInput(text: string, setText: (text: string) => void, onNotice: (notice: VoiceNotice, detail?: string) => void) {
  const latest = useRef<Latest>({ text, setText, onNotice })
  const [status, setStatus] = useState<VoiceStatus>(() => (speechRecognition() ? 'idle' : 'unavailable'))
  const [since, setSince] = useState(0)
  const [download, setDownload] = useState<number | null>(null)
  const [notice, setNotice] = useState<VoiceNotice | null>(null)
  const [level] = useState(levelStore)
  const dictation = useRef<ReturnType<typeof createDictation> | null>(null)

  useEffect(() => {
    latest.current = { text, setText, onNotice }
    dictation.current?.edited(text)
  })

  useEffect(() => {
    const status = (next: VoiceStatus) => {
      if (next === 'listening') {
        setSince(Date.now())
        setNotice(null)
        level.set(-1)
      }
      setStatus(next)
    }
    const created = createDictation(speechRecognition(), fieldOf(latest, { status, level: level.set, download: setDownload, noticed: setNotice }))
    dictation.current = created
    // Firefox and older Safari cannot query the microphone: the first tap finds out.
    navigator.permissions
      ?.query({ name: 'microphone' as PermissionName })
      .then((permission) => permission.state === 'denied' && created.blocked())
      .catch(() => {})
    return () => {
      created.dispose()
      dictation.current = null
    }
  }, [level])

  useEffect(() => {
    if (!notice) return
    const timer = setTimeout(() => setNotice(null), NOTICE_MS)
    return () => clearTimeout(timer)
  }, [notice])

  return { status, since, download, level, notice, toggle: () => dictation.current?.toggle() }
}

export type VoiceInput = ReturnType<typeof useVoiceInput>

/** Voice input for a text field; a refusal or an error shows a toast with what to do (`explain`). */
export function useFieldVoice(text: string, setText: (text: string) => void): VoiceInput {
  return useVoiceInput(text, setText, explain)
}

/** The field's placeholder while voice input listens: it says what is going on and how to stop. */
export function voicePlaceholder(voice: VoiceInput): string | null {
  return voice.status === 'listening' ? 'Listening… click the mic to stop' : null
}

/** `0:07`, `1:05`. */
export function formatElapsed(ms: number): string {
  const seconds = Math.max(0, Math.floor(ms / 1000))
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`
}
