import { toast } from 'sonner'
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

/** The short text of a notice: the toast's title, and the line beside the mic. */
export function noticeTitle(notice: VoiceNotice): string {
  if (notice === 'denied' || notice === 'denied-system') return 'Microphone access is off'
  if (notice === 'speech-refused') return 'Voice input was refused'
  if (notice === 'unavailable') return 'Voice input is not available'
  if (notice === 'no-microphone') return 'No microphone found'
  return 'Voice input stopped'
}

/** A toast for a refusal or an error, with what to do; the desktop app also logs it. */
export function explain(notice: VoiceNotice, detail?: string) {
  desktop()?.log?.(`voice: ${notice}${detail ? ` (${detail})` : ''}`)
  const title = noticeTitle(notice)
  if (notice === 'denied' || notice === 'denied-system' || notice === 'speech-refused') {
    toast.error(title, { description: withDetails(deniedHelp(notice), detail), duration: 15_000 })
  } else if (notice === 'unavailable') {
    toast.error(title, {
      description: detail ? withDetails(detail) : 'This browser has no speech recognition. Use Chrome, Edge or Safari, or the Orbit app on Windows or a Mac.',
      duration: detail ? 15_000 : undefined,
    })
  } else if (notice === 'no-microphone') {
    toast.error(title, { description: withDetails('Connect a microphone and try again.', detail) })
  } else if (notice === 'network') {
    toast.error(title, { description: withDetails(detail ?? 'The speech service could not be reached. Try again later.') })
  } else {
    toast.error(title, { description: withDetails('', detail) || undefined })
  }
}
