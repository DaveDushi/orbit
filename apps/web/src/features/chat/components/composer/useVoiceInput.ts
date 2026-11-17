import { useEffect, useRef, useState, type RefObject } from 'react'
import { createDictation, probeOutcome, speechRecognition, type MicProbe, type VoiceNotice, type VoiceStatus } from './voiceInput'

interface Latest {
  text: string
  setText: (text: string) => void
  onNotice: (notice: VoiceNotice, detail?: string) => void
}

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

/** The dictation reads the field of the last render. */
function fieldOf(latest: RefObject<Latest>, status: (status: VoiceStatus) => void) {
  return {
    text: () => latest.current.text,
    setText: (next: string) => latest.current.setText(next),
    status,
    notice: (notice: VoiceNotice, detail?: string) => latest.current.onNotice(notice, detail),
    probe: probeMicrophone,
    lang: navigator.language,
  }
}

/**
 * Voice input for a text field: `toggle` starts and stops it, and the words go into the field with `setText`.
 * `onNotice` explains a refusal or an error.
 */
export function useVoiceInput(text: string, setText: (text: string) => void, onNotice: (notice: VoiceNotice, detail?: string) => void) {
  const latest = useRef<Latest>({ text, setText, onNotice })
  const [status, setStatus] = useState<VoiceStatus>(() => (speechRecognition() ? 'idle' : 'unavailable'))
  const dictation = useRef<ReturnType<typeof createDictation> | null>(null)

  useEffect(() => {
    latest.current = { text, setText, onNotice }
    dictation.current?.edited(text)
  })

  useEffect(() => {
    const created = createDictation(speechRecognition(), fieldOf(latest, setStatus))
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
  }, [])

  return { status, toggle: () => dictation.current?.toggle() }
}
