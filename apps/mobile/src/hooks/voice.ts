/** Voice input: Apple's speech recognition writes what you say into a text field, on the device where it can. */
import { requireOptionalNativeModule } from 'expo'
import { useEffect, useRef, useState } from 'react'
import { Alert, Linking, Platform } from 'react-native'

import { joinSpeech, pickLocale } from '../data/voice'

type SpeechModule = typeof import('expo-speech-recognition').ExpoSpeechRecognitionModule

/** The native code ships with a new build: older builds and Expo Go lack it, and there the mic button hides. */
const speech: SpeechModule | null =
  Platform.OS === 'web' || requireOptionalNativeModule('ExpoSpeechRecognition')
    ? (require('expo-speech-recognition') as typeof import('expo-speech-recognition')).ExpoSpeechRecognitionModule
    : null

/** `hidden`: no speech module. `unavailable`: the recognizer is off (no Siri or Dictation, or no network). */
export type VoiceStatus = 'hidden' | 'unavailable' | 'denied' | 'idle' | 'listening'

/**
 * One dictation: the field's text when it started, the finished segments, and every text it wrote (two fast results
 * can come before the field renders the first, so the last one alone would look like an edit).
 */
interface Session {
  base: string
  finals: string
  written: Set<string>
}

function explain(title: string, message: string, settings: boolean) {
  if (Platform.OS === 'web') return window.alert(`${title}\n\n${message}`)
  Alert.alert(title, message, settings ? [{ text: 'Cancel', style: 'cancel' }, { text: 'Open Settings', onPress: () => void Linking.openSettings() }] : undefined)
}

const explainDenied = () => explain('Microphone access is off', 'To speak a message, allow Orbit to use the microphone and speech recognition in Settings.', true)
const explainUnavailable = () =>
  explain('Voice input is not available', 'Speech recognition is off on this iPhone right now. Turn on Siri or Dictation in Settings, or try again later.', false)

/**
 * Tap to start, tap to stop: the spoken words show live after the field's text and stay editable; nothing is sent.
 * A change to the field from anything else (typing, emoji, send) ends the dictation and keeps the text.
 */
export function useVoiceInput(text: string, setText: (text: string) => void) {
  const [status, setStatus] = useState<VoiceStatus>(() => (!speech ? 'hidden' : speech.isRecognitionAvailable() ? 'idle' : 'unavailable'))
  const session = useRef<Session | null>(null)
  const latest = useRef({ text, setText })

  useEffect(() => {
    latest.current = { text, setText }
    if (session.current && !session.current.written.has(text)) {
      session.current = null
      speech?.abort()
      setStatus('idle')
    }
  }, [text, setText])

  useEffect(() => {
    if (!speech) return
    void speech.getPermissionsAsync().then(
      (permission) => !permission.granted && !permission.canAskAgain && setStatus((current) => (current === 'idle' ? 'denied' : current)),
      () => {},
    )
    const subscriptions = [
      // Continuous recognition: partial results cover the current segment, a final one ends it.
      speech.addListener('result', (event) => {
        const current = session.current
        if (!current) return
        const said = event.results[0]?.transcript ?? ''
        const next = joinSpeech(current.base, current.finals, said)
        if (event.isFinal) current.finals = joinSpeech(current.finals, said)
        current.written.add(next)
        latest.current.setText(next)
      }),
      speech.addListener('error', (event) => {
        if (!session.current) return
        if (event.error === 'not-allowed') {
          setStatus('denied')
          explainDenied()
        } else if (event.error === 'service-not-allowed' || event.error === 'language-not-supported') {
          setStatus('unavailable')
          explainUnavailable()
        } else if (event.error !== 'aborted' && event.error !== 'no-speech') {
          explain('Voice input stopped', event.message || event.error, false)
        }
      }),
      speech.addListener('end', () => {
        if (!session.current) return
        session.current = null
        setStatus((current) => (current === 'listening' ? 'idle' : current))
      }),
    ]
    return () => {
      for (const subscription of subscriptions) subscription.remove()
      if (session.current) speech.abort()
      session.current = null
    }
  }, [])

  const toggle = async () => {
    if (!speech) return
    // Stop waits for the last final result, then `end`.
    if (status === 'listening') return speech.stop()
    if (!speech.isRecognitionAvailable()) {
      setStatus('unavailable')
      return explainUnavailable()
    }
    let permission = await speech.getPermissionsAsync()
    if (!permission.granted && permission.canAskAgain) permission = await speech.requestPermissionsAsync()
    if (!permission.granted) {
      setStatus('denied')
      return explainDenied()
    }
    const { locales } = await speech.getSupportedLocales({}).catch(() => ({ locales: [] as string[] }))
    const base = latest.current.text
    session.current = { base, finals: '', written: new Set([base]) }
    setStatus('listening')
    speech.start({
      lang: pickLocale(Intl.DateTimeFormat().resolvedOptions().locale, locales),
      interimResults: true,
      continuous: true,
      // Only where the device has the model for this language; else Apple's servers recognize it.
      requiresOnDeviceRecognition: true,
      addsPunctuation: true,
    })
  }

  return { status, toggle }
}
