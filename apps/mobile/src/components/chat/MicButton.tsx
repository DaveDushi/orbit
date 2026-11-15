import { showError } from '../../hooks/chat'
import { useVoiceInput } from '../../hooks/voice'
import { useTheme } from '../../theme'
import { IconButton } from '../../ui'

/**
 * The composer's voice input: a mic that fills while it listens, and a crossed-out mic when the microphone is not
 * allowed or speech recognition is off (a tap explains why). Without the speech module it does not show.
 */
export function MicButton({ text, setText }: { text: string; setText: (text: string) => void }) {
  const { colors } = useTheme()
  const voice = useVoiceInput(text, setText)
  if (voice.status === 'hidden') return null
  const listening = voice.status === 'listening'
  const off = voice.status === 'denied' || voice.status === 'unavailable'
  return (
    <IconButton
      icon={off ? 'micOff' : 'mic'}
      label={listening ? 'Stop voice input' : 'Voice input'}
      size={32}
      iconSize={20}
      variant={listening ? 'primary' : 'ghost'}
      color={off ? colors.mutedForeground : undefined}
      onPress={() => void voice.toggle().catch((error) => showError('Voice input did not start', error))}
    />
  )
}
