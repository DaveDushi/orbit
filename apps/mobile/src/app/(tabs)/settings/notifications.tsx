import { useState } from 'react'
import { Platform, View } from 'react-native'

import { ACCOUNT_NAV, Note, RowIcon, RowText, SettingsCard, SettingsRow, SettingsShell, Toggle } from '../../../components/settings/Shell'
import { NOTIFICATION_CHOICES } from '../../../data/account-api'
import { shortDate } from '../../../data/format'
import { useNotificationPrefs, useSaveNotificationPrefs } from '../../../hooks/account'
import { usePushBrowsers, useRemovePushBrowser, useSendTestPush } from '../../../hooks/settings'
import { useIsGuest, useOrbit } from '../../../orbit-context'
import { useThisDevice } from '../../../push'
import { errorMessage } from '../../../session-context'
import { Button, Text } from '../../../ui'

/**
 * The web's Notifications page on the phone: This iPhone (its push notifications on or off, and a test), the browsers
 * that get them (Devices), and what Orbit notifies about. The web's Other devices (the browser window's focus) and
 * Sounds (the browser's sounds) are about a browser; iOS sets the phone's sounds.
 */
export default function NotificationsScreen() {
  const prefs = useNotificationPrefs()
  const save = useSaveNotificationPrefs()
  const current = prefs.data
  // a guest has only chat: no task or page rows
  const guest = useIsGuest()
  const choices = NOTIFICATION_CHOICES.filter((choice) => !guest || choice.chat)

  return (
    <SettingsShell title="Account settings" nav={ACCOUNT_NAV} refreshing={prefs.isRefetching} onRefresh={() => void prefs.refetch()}>
      <ThisDevice />
      <Browsers />
      <SettingsCard title="Notify me about" description="These apply to all your devices.">
        {!current ? (
          prefs.isError ? (
            <View className="gap-2 items-start">
              <Note error>Your choices could not be loaded.</Note>
              <Button label="Retry" variant="ghost" onPress={() => void prefs.refetch()} />
            </View>
          ) : (
            <Note loading>Loading…</Note>
          )
        ) : (
          <View className="gap-4">
            <View>
              {choices.map(({ field, label }, index) => (
                <View key={field} className={`flex-row items-center justify-between gap-4 border-border ${index === 0 ? 'pt-0 border-t-0' : 'pt-3 border-t'} ${index === choices.length - 1 ? 'pb-0' : 'pb-3'}`}>
                  <Text className="font-noto-medium text-[14px] flex-1">{label}</Text>
                  {/* a server without `followed_tasks` has it on */}
                  <Toggle label={label} value={current[field] ?? true} onChange={(checked) => save.mutate({ ...current, [field]: checked })} />
                </View>
              ))}
            </View>
            <Text className="text-[12px] text-muted-foreground">Do not disturb turns all of these off. Muted channels never notify.</Text>
            {save.isError ? <Note error>Could not save your choice. {errorMessage(save.error)}</Note> : null}
          </View>
        )}
      </SettingsCard>
    </SettingsShell>
  )
}

const DEVICE_TEXT = {
  unsupported: 'Push notifications work in the iPhone app.',
  blocked: 'iOS blocks notifications for Orbit. Allow them in the Settings app, under Notifications → Orbit.',
  on: 'Notifications are on for this iPhone.',
  off: 'Notifications are off on this iPhone.',
} as const

/** This phone: on or off, and a test notification to every device of the person. */
function ThisDevice() {
  const { client } = useOrbit()
  const { state, turnOn, turnOff } = useThisDevice(client)
  const test = useSendTestPush()
  const [sent, setSent] = useState<number | null>(null)
  const current = state.data
  const label = Platform.OS === 'ios' && Platform.isPad ? 'iPad' : 'iPhone'
  return (
    <SettingsCard title={`This ${label}`} description="Get a notification here when you are not looking at Orbit.">
      {!current ? (
        <Note loading>Checking this device…</Note>
      ) : (
        <View className="gap-3">
          <Text className="text-[14px] text-muted-foreground">{DEVICE_TEXT[current].replace('iPhone', label)}</Text>
          {current === 'unsupported' ? null : (
            <View className="flex-row flex-wrap gap-2">
              {current === 'on' ? (
                <>
                  <Button label="Send a test notification" variant="outline" loading={test.isPending} onPress={() => { setSent(null); test.mutate(undefined, { onSuccess: setSent }) }} />
                  <Button label="Turn off" variant="outline" loading={turnOff.isPending} onPress={() => turnOff.mutate()} />
                </>
              ) : (
                <Button label={current === 'blocked' ? 'Open the Settings app' : 'Turn on notifications'} loading={turnOn.isPending} onPress={() => turnOn.mutate()} />
              )}
            </View>
          )}
          {sent !== null ? <Note>Sent to {sent} {sent === 1 ? 'device' : 'devices'}.</Note> : null}
          {test.isError ? <Note error>Could not send the test notification. {errorMessage(test.error)}</Note> : null}
          {turnOn.isError || turnOff.isError ? <Note error>Could not change the notifications. {errorMessage(turnOn.error ?? turnOff.error)}</Note> : null}
        </View>
      )}
    </SettingsCard>
  )
}

/** The browsers that get the person's notifications (web push). The server does not list the iPhones here yet. */
function Browsers() {
  const browsers = usePushBrowsers()
  const remove = useRemovePushBrowser()
  const list = browsers.data ?? []
  return (
    <SettingsCard title="Browsers" description="Every browser that gets your notifications. Removing one stops its notifications." flush>
      {browsers.isPending ? <View className="p-4"><Note loading>Loading devices…</Note></View> : null}
      {browsers.isError ? (
        <View className="p-4 gap-2 items-start">
          <Note error>Devices could not be loaded.</Note>
          <Button label="Retry" variant="ghost" onPress={() => void browsers.refetch()} />
        </View>
      ) : null}
      {browsers.isSuccess && list.length === 0 ? <View className="p-4"><Note>No browser gets notifications.</Note></View> : null}
      {list.map((device, index) => (
        <SettingsRow key={device.id} first={index === 0}>
          <RowIcon icon={/Android|iOS|iPadOS/.test(device.label) ? 'mobile' : 'monitor'} />
          <RowText title={device.label} subtitle={`Added ${shortDate(device.created_at)}`} />
          <Button label="Remove" variant="ghost" size="sm" disabled={remove.isPending} onPress={() => remove.mutate(device.id)} />
        </SettingsRow>
      ))}
      {remove.isError ? <View className="p-4"><Note error>Could not remove the device. {errorMessage(remove.error)}</Note></View> : null}
    </SettingsCard>
  )
}
