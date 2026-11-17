import Constants from 'expo-constants'
import { usePathname } from 'expo-router'
import { useState } from 'react'
import { useWindowDimensions, View } from 'react-native'

import { improvementContext } from '../data/improvements'
import { useSendImprovement } from '../hooks/improvements'
import { errorMessage } from '../session-context'
import { Input, Text } from '../ui'
import { FormSheet } from './chat/Sheet'

const MAX_TEXT_CHARS = 4000
const PLACEHOLDER = 'For example: “Show the due date on the cards of the board view.” or “Make the chat Send button easier to see in dark mode.”'

/** The web's "Suggest a change": an installation admin asks for a change; an agent on this server builds it. */
export function SuggestChangeSheet({ onClose, onSent }: { onClose: () => void; onSent: () => void }) {
  const send = useSendImprovement()
  const pathname = usePathname()
  const { width, height } = useWindowDimensions()
  const [text, setText] = useState('')
  const submit = () => {
    if (text.trim() === '' || send.isPending) return
    // the page and the window as the web sends them; the revision names the iPhone app's version
    const context = improvementContext(pathname, width, height, Constants.expoConfig?.version)
    send.mutate({ text, context }, { onSuccess: onSent })
  }
  return (
    <FormSheet visible title="Suggest a change" submitLabel="Send" canSubmit={text.trim() !== ''} busy={send.isPending} onSubmit={submit} onClose={() => (send.isPending ? undefined : onClose())}>
      <Input label="What should change?" autoFocus multiline maxLength={MAX_TEXT_CHARS} placeholder={PLACEHOLDER} value={text} onChangeText={setText} accessibilityLabel="What should change?" />
      <View className="flex-row justify-between gap-3">
        <Text className="text-[12px] leading-[16px] flex-1 text-muted-foreground">An agent builds it on this server, sends screenshots to the owner for review, and reports back under Admin → Feedback & fixes.</Text>
        <Text className="text-[12px] leading-[16px] text-muted-foreground">
          {text.length} / {MAX_TEXT_CHARS}
        </Text>
      </View>
      {send.isError ? (
        <Text accessibilityRole="alert" className="text-[13px] text-destructive">
          {errorMessage(send.error)}
        </Text>
      ) : null}
    </FormSheet>
  )
}
