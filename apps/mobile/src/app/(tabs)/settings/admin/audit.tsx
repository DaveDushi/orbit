import { View } from 'react-native'

import { AdminShell, Badge, Note, SettingsCard, SettingsRow } from '../../../../components/settings/Shell'
import { auditActor } from '../../../../data/admin-api'
import { useAuditEvents } from '../../../../hooks/admin'
import { fonts, type, useTheme } from '../../../../theme'
import { Button, Text } from '../../../../ui'

/** The web's Admin → Audit log, read-only and newest first. The CSV export is a desktop download, so it is left out. */
export default function AdminAuditScreen() {
  const { colors } = useTheme()
  const audit = useAuditEvents()
  const events = audit.data?.pages.flatMap((page) => page.items) ?? []
  return (
    <AdminShell refreshing={audit.isRefetching} onRefresh={() => void audit.refetch()}>
      <SettingsCard title="Audit log" description="Sign-ins, account changes and other security events from every workspace." flush>
        {audit.isPending ? <View style={{ padding: 16 }}><Note>Loading events…</Note></View> : null}
        {audit.isError ? <View style={{ padding: 16 }}><Note error>Events could not be loaded.</Note></View> : null}
        {audit.data && events.length === 0 ? <View style={{ padding: 16 }}><Note>No events yet.</Note></View> : null}
        {events.map((event, index) => (
          <SettingsRow key={event.id} first={index === 0}>
            <View style={{ flex: 1, minWidth: 0, gap: 2 }}>
              <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}>
                <Text numberOfLines={1} style={{ flexShrink: 1, fontFamily: fonts.mono, fontSize: 12, color: colors.foreground }}>
                  {event.action}
                </Text>
                {event.outcome === 'failure' ? <Badge label="Failed" tone="destructive" /> : null}
              </View>
              <Text numberOfLines={1} style={[type(12), { color: colors.mutedForeground }]}>{`${auditActor(event)} · ${new Date(event.occurred_at).toLocaleString()}`}</Text>
            </View>
          </SettingsRow>
        ))}
        {audit.hasNextPage ? (
          <View style={{ padding: 8, alignItems: 'center', borderTopWidth: 1, borderTopColor: colors.border }}>
            <Button label={audit.isFetchingNextPage ? 'Loading…' : 'Load older events'} variant="ghost" disabled={audit.isFetchingNextPage} onPress={() => void audit.fetchNextPage()} />
          </View>
        ) : null}
      </SettingsCard>
      <Text style={[type(12), { color: colors.mutedForeground }]}>Export the log as CSV from Orbit in a browser.</Text>
    </AdminShell>
  )
}
