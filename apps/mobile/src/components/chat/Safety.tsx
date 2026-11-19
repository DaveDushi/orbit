import { useState } from 'react'
import { Pressable, View } from 'react-native'

import * as chat from '../../data/chat-api'
import type { MessageRecord } from '../../data/orbit'
import { showError } from '../../hooks/chat'
import { useOrbit } from '../../orbit-context'
import { useTheme } from '../../theme'
import { Icon, Input, Text } from '../../ui'
import { FormSheet } from './Sheet'

export const REPORT_REASONS: readonly { value: chat.ChatReportReason; label: string }[] = [
  { value: 'spam', label: 'Spam' },
  { value: 'harassment', label: 'Harassment' },
  { value: 'inappropriate', label: 'Inappropriate content' },
  { value: 'other', label: 'Other' },
]

export const REPORT_NOTE_MAX = 500

/** Report a message: a reason, an optional note, then the thank-you. Shows while `message` is set; each opening starts empty. */
export function ReportSheet({ message, onClose }: { message: MessageRecord | null; onClose: () => void }) {
  const { colors } = useTheme()
  const { client, workspaceId } = useOrbit()
  const [openFor, setOpenFor] = useState<string | null>(null)
  const [reason, setReason] = useState<chat.ChatReportReason | null>(null)
  const [note, setNote] = useState('')
  const [busy, setBusy] = useState(false)
  const [sent, setSent] = useState(false)
  // reset when it opens, not when it closes, so the sheet keeps its content while it slides away
  if (message && message.id !== openFor) {
    setOpenFor(message.id)
    setReason(null)
    setNote('')
    setSent(false)
  }
  if (!message && openFor !== null) setOpenFor(null)

  const submit = async () => {
    if (!message || !reason) return
    setBusy(true)
    try {
      await chat.reportMessage(client, workspaceId, message.id, reason, note.trim())
      setSent(true)
    } catch (error) {
      showError('Could not report the message', error)
    } finally {
      setBusy(false)
    }
  }

  if (sent) {
    return (
      <FormSheet visible={message !== null} title="Report sent" onClose={onClose}>
        <View className="items-center gap-3 pt-6">
          <Icon name="checkCircle" size={32} color={colors.primary} />
          <Text className="text-center text-[14px] leading-[21px]">Thank you. Workspace owners and admins will look at your report.</Text>
        </View>
      </FormSheet>
    )
  }
  return (
    <FormSheet visible={message !== null} title="Report message" submitLabel="Report" canSubmit={reason !== null} busy={busy} onSubmit={() => void submit()} onClose={onClose}>
      <View className="gap-2">
        <Text className="font-noto-medium text-[14px] leading-[19.25px]">Reason</Text>
        <View accessibilityRole="radiogroup" className="rounded-lg border border-input overflow-hidden">
          {REPORT_REASONS.map((option, index) => {
            const selected = option.value === reason
            return (
              <Pressable
                key={option.value}
                accessibilityRole="radio"
                accessibilityState={{ selected }}
                accessibilityLabel={option.label}
                onPress={() => setReason(option.value)}
                className={`h-11 flex-row items-center gap-2 px-3 border-input ${index === 0 ? 'border-t-0' : 'border-t'} ${selected ? 'bg-muted' : ''} active:bg-muted`}
              >
                <Text className="font-noto-medium text-[14px] flex-1">{option.label}</Text>
                {selected ? <Icon name="check" size={16} color={colors.primary} /> : null}
              </Pressable>
            )
          })}
        </View>
      </View>
      <Input
        label="Note"
        description={`Optional. Tell the workspace owners and admins more about the problem. At most ${REPORT_NOTE_MAX} characters.`}
        value={note}
        onChangeText={setNote}
        maxLength={REPORT_NOTE_MAX}
        multiline
      />
    </FormSheet>
  )
}
