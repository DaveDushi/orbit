import { useEffect, useState, useSyncExternalStore } from 'react'
import { Mic, MicOff } from 'reicon-react'
import { Tip } from '@/components/common/Tip'
import { Button } from '@/components/ui/button'
import { LoadingText } from '@/components/ui/spinner'
import { formatElapsed, type LevelStore, type VoiceInput } from './useVoiceInput'
import { noticeTitle } from './voiceNotice'

function Elapsed({ since }: { since: number }) {
  const [now, setNow] = useState(Date.now)
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 250)
    return () => clearInterval(timer)
  }, [])
  return <span className="tabular-nums">{formatElapsed(now - since)}</span>
}

/** Five bars of the microphone's level; only a recognizer that reports it (the Orbit app) shows them. */
function LevelMeter({ level }: { level: LevelStore }) {
  const value = useSyncExternalStore(level.subscribe, level.get)
  if (value < 0) return null
  // Speech peaks are often below 0.3: the square root spreads them over the bars.
  const lit = Math.round(Math.sqrt(value) * 5)
  return (
    <span aria-hidden className="flex h-3 items-end gap-px">
      {[0.4, 0.6, 0.8, 1, 0.7].map((height, i) => (
        <span key={i} data-lit={i < lit || undefined} className="w-[3px] rounded-full bg-destructive/25 data-lit:bg-destructive" style={{ height: `${height * 100}%` }} />
      ))}
    </span>
  )
}

/**
 * What voice input is doing, beside the mic: a red dot, the time and the level while it listens (and the model's
 * one-time download), a spinner while the last words are transcribed, and a refusal or an error for a few seconds.
 */
export function VoiceStatus({ voice }: { voice: VoiceInput }) {
  if (voice.status === 'listening') {
    return (
      <span data-slot="voice-status" role="status" className="flex shrink-0 items-center gap-1.5 rounded-full bg-destructive/10 px-2 py-0.5 text-xs font-medium text-destructive">
        <span aria-hidden className="size-2 rounded-full bg-destructive motion-safe:animate-pulse" />
        <span className="sr-only">Listening</span>
        <Elapsed since={voice.since} />
        <LevelMeter level={voice.level} />
        {voice.download !== null ? <span className="font-normal text-muted-foreground">Getting voice input ready… {voice.download}%</span> : null}
      </span>
    )
  }
  if (voice.status === 'transcribing') return <LoadingText className="shrink-0 px-1 text-xs">Transcribing…</LoadingText>
  if (voice.notice) {
    return (
      <span data-slot="voice-status" role="status" className="min-w-0 truncate px-1 text-xs font-medium text-destructive">
        {noticeTitle(voice.notice)}
      </span>
    )
  }
  return null
}

/**
 * The mic of a text field: tap to start, tap to stop. While it listens it is red and pulses, with a stop square; after
 * stop it dims, and `VoiceStatus` shows a spinner until the last words are in. A crossed-out mic shows when the microphone is blocked or the
 * browser has no speech recognition (a tap explains why). Put `VoiceStatus` beside it, and `voicePlaceholder` in the field.
 */
export function MicButton({ voice, className = '' }: { voice: VoiceInput; className?: string }) {
  const { status } = voice
  const listening = status === 'listening'
  const off = status === 'denied' || status === 'unavailable'
  const label =
    listening ? 'Stop voice input' : status === 'transcribing' ? 'Transcribing…' : status === 'denied' ? 'Microphone access is off' : status === 'unavailable' ? 'Voice input is not available' : 'Voice input'
  return (
    <Tip label={label}>
      <Button
        type="button"
        variant="ghost"
        size="icon"
        aria-label={label}
        aria-pressed={listening}
        data-status={status}
        className={`relative data-[status=denied]:text-muted-foreground data-[status=listening]:bg-destructive data-[status=listening]:text-white data-[status=listening]:hover:bg-destructive/90 data-[status=listening]:dark:hover:bg-destructive/90 data-[status=transcribing]:opacity-50 data-[status=unavailable]:text-muted-foreground ${className}`}
        onClick={voice.toggle}
      >
        {listening ? (
          <>
            <span aria-hidden className="absolute inset-0 rounded-[inherit] ring-2 ring-destructive/60 motion-safe:animate-ping" />
            <span aria-hidden className="size-3 rounded-[3px] bg-current" />
          </>
        ) : off ? (
          <MicOff className="size-5" />
        ) : (
          <Mic className="size-5" />
        )}
      </Button>
    </Tip>
  )
}
