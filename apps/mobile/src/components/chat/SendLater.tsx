import { router } from 'expo-router'
import { useState } from 'react'
import { Pressable, View } from 'react-native'

import { initialSendTime, scheduledHere, sendLaterPresets, whenLabel } from '../../data/scheduled'
import { useScheduledMessages } from '../../hooks/scheduled'
import { useTheme } from '../../theme'
import { Icon, Text } from '../../ui'
import { DateTimeField } from './DateTimeField'
import { FormSheet } from './Sheet'

/** Picks the time of a message: a quick choice, or a date and time (local). */
export function SendLaterSheet({ title = 'Send later', initial, pending = false, onPick, onClose }: { title?: string; initial?: Date; pending?: boolean; onPick: (at: Date) => void; onClose: () => void }) {
  const [now] = useState(() => new Date())
  const [custom, setCustom] = useState(() => initialSendTime(now, initial))
  const valid = custom > new Date()
  return (
    <FormSheet visible title={title} description="The message is posted as you at this time." submitLabel="Schedule" canSubmit={valid && !pending} busy={pending} onSubmit={() => onPick(custom)} onClose={onClose}>
      <View className="rounded-lg border border-border overflow-hidden">
        {sendLaterPresets(now).map((preset, index) => (
          <Pressable
            key={preset.label}
            accessibilityRole="button"
            accessibilityLabel={`${preset.label}, ${whenLabel(preset.at, now)}`}
            disabled={pending}
            onPress={() => onPick(preset.at)}
            className={`h-12 flex-row items-center justify-between px-3 active:bg-muted ${index > 0 ? 'border-t border-border' : ''}`}
          >
            <Text className="text-[15px]">{preset.label}</Text>
            <Text className="text-[13px] text-muted-foreground">{whenLabel(preset.at, now)}</Text>
          </Pressable>
        ))}
      </View>
      <View className="gap-2">
        <Text className="font-noto-medium text-[14px]">Date and time</Text>
        <DateTimeField value={custom} minimum={now} onChange={setCustom} />
        {!valid ? <Text className="text-[12px] text-destructive">Choose a time in the future.</Text> : null}
      </View>
    </FormSheet>
  )
}

/** Above the composer: how many of the person's messages wait for this conversation; a tap opens the list. */
export function ScheduledBar({ conversationId }: { conversationId: string }) {
  const { colors } = useTheme()
  const { items, failed } = scheduledHere(useScheduledMessages().data ?? [], conversationId)
  if (items.length === 0) return null
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityHint="Opens your scheduled messages"
      onPress={() => router.push('/chat/scheduled')}
      className="mx-2 -mb-1 h-8 flex-row items-center gap-2 px-2 rounded-md active:bg-muted/60"
    >
      <Icon name="clock" size={16} color={colors.mutedForeground} />
      <Text numberOfLines={1} className="text-[12px] flex-1 text-muted-foreground">
        {items.length === 1 ? '1 message scheduled here' : `${items.length} messages scheduled here`}
        {failed > 0 ? <Text className="text-[12px] text-destructive"> · {failed} not sent</Text> : ` · next ${whenLabel(items[0]!.send_at)}`}
      </Text>
      <Text className="font-noto-medium text-[12px] text-foreground">View</Text>
    </Pressable>
  )
}
