import { View } from 'react-native'

import { ACCOUNT_NAV, Note, SettingsCard, SettingsShell, Toggle } from '../../../components/settings/Shell'
import { NOTIFICATION_CHOICES } from '../../../data/account-api'
import { useNotificationPrefs, useSaveNotificationPrefs } from '../../../hooks/account'
import { errorMessage } from '../../../session-context'
import { type, useTheme } from '../../../theme'
import { Button, Text } from '../../../ui'

/**
 * The web's Notifications page, the part that is the account's: what Orbit notifies about. This device, Devices,
 * Other devices and Sounds are about the browser's web push and sounds; the app has no push notifications yet.
 */
export default function NotificationsScreen() {
  const { colors } = useTheme()
  const prefs = useNotificationPrefs()
  const save = useSaveNotificationPrefs()
  const current = prefs.data

  return (
    <SettingsShell title="Account settings" nav={ACCOUNT_NAV} refreshing={prefs.isRefetching} onRefresh={() => void prefs.refetch()}>
      <SettingsCard title="Notify me about" description="These apply to all your devices.">
        {!current ? (
          prefs.isError ? (
            <View style={{ gap: 8, alignItems: 'flex-start' }}>
              <Note error>Your choices could not be loaded.</Note>
              <Button label="Retry" variant="ghost" onPress={() => void prefs.refetch()} />
            </View>
          ) : (
            <Note>Loading…</Note>
          )
        ) : (
          <View style={{ gap: 16 }}>
            <View>
              {NOTIFICATION_CHOICES.map(({ field, label }, index) => (
                <View key={field} style={{ flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', gap: 16, paddingTop: index === 0 ? 0 : 12, paddingBottom: index === NOTIFICATION_CHOICES.length - 1 ? 0 : 12, borderTopWidth: index === 0 ? 0 : 1, borderTopColor: colors.border }}>
                  <Text style={[type(14, 'medium'), { flex: 1 }]}>{label}</Text>
                  <Toggle label={label} value={current[field]} onChange={(checked) => save.mutate({ ...current, [field]: checked })} />
                </View>
              ))}
            </View>
            <Text style={[type(12), { color: colors.mutedForeground }]}>Do not disturb turns all of these off. Muted channels never notify.</Text>
            {save.isError ? <Note error>Could not save your choice. {errorMessage(save.error)}</Note> : null}
          </View>
        )}
      </SettingsCard>
    </SettingsShell>
  )
}
