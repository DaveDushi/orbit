import { Mic, MicOff } from 'reicon-react'
import { toast } from 'sonner'
import { Tip } from '@/components/common/Tip'
import { Button } from '@/components/ui/button'
import { useVoiceInput } from './useVoiceInput'
import type { VoiceNotice } from './voiceInput'

/** Where to turn the microphone on: the desktop app (its bridge sets `__orbitDesktop`) asks the OS, a browser its site settings. */
function deniedHelp(): string {
  const platform = (navigator as Navigator & { userAgentData?: { platform?: string } }).userAgentData?.platform || navigator.platform
  if (!('__orbitDesktop' in window)) return 'Allow the microphone for Orbit in the browser’s site settings, then try again.'
  if (/^win/i.test(platform)) return 'Turn on Windows Settings → Privacy & security → Microphone → “Microphone access” and “Let desktop apps access your microphone”, then try again.'
  if (/^mac/i.test(platform)) return 'Allow Orbit in System Settings → Privacy & Security → Microphone and Speech Recognition, then try again.'
  return 'Allow the microphone for Orbit in the system settings, then try again.'
}

function explain(notice: VoiceNotice, detail?: string) {
  if (notice === 'denied') {
    toast.error('Microphone access is off', { description: deniedHelp() })
  } else if (notice === 'unavailable') {
    toast.error('Voice input is not available', { description: 'This browser has no speech recognition. Use Chrome, Edge or Safari, or the Orbit app on Windows or a Mac.' })
  } else if (notice === 'no-microphone') {
    toast.error('No microphone found', { description: 'Connect a microphone and try again.' })
  } else if (notice === 'network') {
    toast.error('Voice input stopped', { description: 'The speech service could not be reached. Try again later.' })
  } else {
    toast.error('Voice input stopped', { description: detail })
  }
}

/**
 * The composer's voice input: tap to start, tap to stop. It fills while it listens, and shows a crossed-out mic when
 * the microphone is blocked or the browser has no speech recognition (a tap explains why).
 */
export function MicButton({ text, setText }: { text: string; setText: (text: string) => void }) {
  const voice = useVoiceInput(text, setText, explain)
  const listening = voice.status === 'listening'
  const off = voice.status === 'denied' || voice.status === 'unavailable'
  const label = listening ? 'Stop voice input' : voice.status === 'denied' ? 'Microphone access is off' : voice.status === 'unavailable' ? 'Voice input is not available' : 'Voice input'
  return (
    <Tip label={label}>
      <Button
        variant={listening ? 'default' : 'ghost'}
        size="icon"
        aria-label={label}
        aria-pressed={listening}
        data-status={voice.status}
        className="data-[status=denied]:text-muted-foreground data-[status=unavailable]:text-muted-foreground"
        onClick={voice.toggle}
      >
        {off ? <MicOff className="size-5" /> : <Mic className="size-5" />}
      </Button>
    </Tip>
  )
}
