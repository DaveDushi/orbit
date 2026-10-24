import { View } from 'react-native'

import { AdminShell, Badge, Note, SettingsCard, SettingsRow } from '../../../../components/settings/Shell'
import { auditActor } from '../../../../data/admin-api'
import { useAuditEvents } from '../../../../hooks/admin'
import { Button, Text } from '../../../../ui'

/** The web's Admin → Audit log, read-only and newest first. The CSV export is a desktop download, so it is left out. */
export default function AdminAuditScreen() {
  const audit = useAuditEvents()
  const events = audit.data?.pages.flatMap((page) => page.items) ?? []
  return (
    <AdminShell refreshing={audit.isRefetching} onRefresh={() => void audit.refetch()}>
      <SettingsCard title="Audit log" description="Sign-ins, account changes and other security events from every workspace." flush>
        {audit.isPending ? <View className="p-4"><Note>Loading events…</Note></View> : null}
        {audit.isError ? <View className="p-4"><Note error>Events could not be loaded.</Note></View> : null}
        {audit.data && events.length === 0 ? <View className="p-4"><Note>No events yet.</Note></View> : null}
        {events.map((event, index) => (
          <SettingsRow key={event.id} first={index === 0}>
            <View className="flex-1 min-w-0 gap-0.5">
              <View className="flex-row items-center gap-1.5">
                <Text numberOfLines={1} className="shrink font-mono text-[12px] text-foreground leading-[21px]">
                  {event.action}
                </Text>
                {event.outcome === 'failure' ? <Badge label="Failed" tone="destructive" /> : null}
              </View>
              <Text numberOfLines={1} className="text-[12px] text-muted-foreground">{`${auditActor(event)} · ${new Date(event.occurred_at).toLocaleString()}`}</Text>
            </View>
          </SettingsRow>
        ))}
        {audit.hasNextPage ? (
          <View className="p-2 items-center border-t border-border">
            <Button label={audit.isFetchingNextPage ? 'Loading…' : 'Load older events'} variant="ghost" disabled={audit.isFetchingNextPage} onPress={() => void audit.fetchNextPage()} />
          </View>
        ) : null}
      </SettingsCard>
      <Text className="text-[12px] text-muted-foreground">Export the log as CSV from Orbit in a browser.</Text>
    </AdminShell>
  )
}
