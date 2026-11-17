import Constants from 'expo-constants'
import { router, usePathname } from 'expo-router'
import { useEffect, useState } from 'react'
import { useWindowDimensions, View } from 'react-native'

import { improvementContext } from '../../data/improvements'
import { useImprovementSettings, useSaveImprovementSettings, useSendImprovement } from '../../hooks/improvements'
import { errorMessage } from '../../session-context'
import { Button, Input, Text } from '../../ui'
import { Dialog } from './Dialog'
import { Note, OptionHint, OptionText, SettingsCard, SettingsRow, Toggle } from './Shell'

const MAX_TEXT_CHARS = 4000

/** Admin → General: the "Suggest a change" switch. Off by default. */
export function ImprovementsCard() {
  const settings = useImprovementSettings()
  const save = useSaveImprovementSettings()
  return (
    <SettingsCard title="Suggest a change" description="Change requests from the admins of this Orbit, for an agent on this server." flush>
      <SettingsRow first>
        <OptionText title="Suggest a change">
          <OptionHint>On: instance admins find Suggest a change in their account menu, and the requests under Feedback & fixes. An agent on this server reads them with orbit improvements. Members never see it.</OptionHint>
          {save.isError ? <OptionHint error>{errorMessage(save.error)}</OptionHint> : null}
        </OptionText>
        <Toggle label="Suggest a change" value={settings.data?.enabled ?? false} disabled={!settings.data || save.isPending} onChange={(checked) => save.mutate(checked)} />
      </SettingsRow>
    </SettingsCard>
  )
}

/** An installation admin asks for a change; an agent on this server builds it. The context names the app's screen. */
export function SuggestChangeDialog({ visible, onClose }: { visible: boolean; onClose: () => void }) {
  const send = useSendImprovement()
  const pathname = usePathname()
  const { width, height } = useWindowDimensions()
  const [text, setText] = useState('')
  const [sent, setSent] = useState(false)
  useEffect(() => {
    if (!visible) return
    setText('')
    setSent(false)
    send.reset()
    // only when the dialog opens
  }, [visible])
  const submit = () => {
    if (!text.trim() || send.isPending) return
    // the page and the window as the web sends them; the revision names the iPhone app's version
    send.mutate({ text, context: improvementContext(pathname, width, height, Constants.expoConfig?.version) }, { onSuccess: () => setSent(true) })
  }
  return (
    <Dialog
      visible={visible}
      title="Suggest a change"
      onClose={() => !send.isPending && onClose()}
      footer={
        sent ? (
          <>
            <Button label="Close" variant="outline" onPress={onClose} />
            <Button
              label="Feedback & fixes"
              onPress={() => {
                onClose()
                router.navigate('/settings/admin/feedback')
              }}
            />
          </>
        ) : (
          <>
            <Button label="Cancel" variant="outline" disabled={send.isPending} onPress={onClose} />
            <Button label={send.isPending ? 'Sending…' : 'Send'} loading={send.isPending} disabled={!text.trim()} onPress={submit} />
          </>
        )
      }
    >
      {sent ? (
        <Note>Request sent.</Note>
      ) : (
        <View className="gap-2">
          <Input
            label="What should change?"
            multiline
            autoFocus
            maxLength={MAX_TEXT_CHARS}
            placeholder="For example: “Show the due date on the cards of the board view.”"
            value={text}
            onChangeText={setText}
            description="An agent builds it on this server, sends screenshots to the owner for review, and reports back under Feedback & fixes."
          />
          <Text className="self-end text-[12px] text-muted-foreground">{text.length} / {MAX_TEXT_CHARS}</Text>
          {send.isError ? <Note error>{errorMessage(send.error)}</Note> : null}
        </View>
      )}
    </Dialog>
  )
}
