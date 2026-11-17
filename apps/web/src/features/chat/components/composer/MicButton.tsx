import { Mic, MicOff } from 'reicon-react'
import { toast } from 'sonner'
import { Tip } from '@/components/common/Tip'
import { Button } from '@/components/ui/button'
import { useVoiceInput } from './useVoiceInput'
import type { VoiceNotice } from './voiceInput'

/** The desktop bridge (`apps/desktop/.../bridge.js`): `app` and `log` since Orbit desktop 0.1.6. */
type DesktopBridge = { app?: string; log?: (text: string) => void }

function desktop(): DesktopBridge | null {
  return '__orbitDesktop' in window ? ((window as { __orbitDesktop?: DesktopBridge }).__orbitDesktop ?? {}) : null
}

function platform(): string {
  return (navigator as Navigator & { userAgentData?: { platform?: string } }).userAgentData?.platform || navigator.platform
}

/** The app's version for a report; an app before 0.1.6 does not say it, and checks for updates only at its start. */
function appLine(app: DesktopBridge): string {
  return app.app ? `Orbit app ${app.app}.` : 'Orbit app 0.1.5 or older: quit Orbit in the tray menu and open it again, then install the update it offers.'
}

/** Where to turn the microphone on: the desktop app asks the OS, a browser its site settings. */
function deniedHelp(notice: VoiceNotice): string {
  const app = desktop()
  if (notice === 'speech-refused') return 'The microphone works, but speech recognition was refused.'
  if (!app) return notice === 'denied-system' ? 'Your system blocks the microphone for this browser: allow it in the system’s privacy settings, then try again.' : 'Allow the microphone for Orbit in the browser’s site settings, then try again.'
  if (/^win/i.test(platform())) return 'Open Windows Settings → Privacy & security → Microphone, and turn on “Microphone access” and “Let apps access your microphone”. Then try again.'
  if (/^mac/i.test(platform())) return 'Allow Orbit in System Settings → Privacy & Security → Microphone and Speech Recognition, then try again.'
  return 'Allow the microphone for Orbit in the system settings, then try again.'
}

/** The toast text, with the details we need when the person sends a screenshot. */
function withDetails(text: string, detail?: string): string {
  const app = desktop()
  return [text, detail && `Details: ${detail}.`, app && appLine(app)].filter(Boolean).join(' ')
}

function explain(notice: VoiceNotice, detail?: string) {
  desktop()?.log?.(`voice: ${notice}${detail ? ` (${detail})` : ''}`)
  if (notice === 'denied' || notice === 'denied-system' || notice === 'speech-refused') {
    toast.error(notice === 'speech-refused' ? 'Voice input was refused' : 'Microphone access is off', { description: withDetails(deniedHelp(notice), detail), duration: 15_000 })
  } else if (notice === 'unavailable') {
    toast.error('Voice input is not available', {
      description: detail ? withDetails(detail) : 'This browser has no speech recognition. Use Chrome, Edge or Safari, or the Orbit app on Windows or a Mac.',
      duration: detail ? 15_000 : undefined,
    })
  } else if (notice === 'no-microphone') {
    toast.error('No microphone found', { description: withDetails('Connect a microphone and try again.', detail) })
  } else if (notice === 'network') {
    toast.error('Voice input stopped', { description: withDetails(detail ?? 'The speech service could not be reached. Try again later.') })
  } else {
    toast.error('Voice input stopped', { description: withDetails('', detail) || undefined })
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
