import { useQuery } from '@tanstack/react-query'
import { Linking, View } from 'react-native'

import { Note, SettingsCard, SettingsShell, WORKSPACE_NAV } from '../../../components/settings/Shell'
import { billingDescription, billingKey, billingRows, workspaceBilling } from '../../../data/billing'
import { fullDate } from '../../../data/format'
import { useAuthOptions } from '../../../hooks/account'
import { useOrbit } from '../../../orbit-context'
import { Button, Text } from '../../../ui'

/**
 * The web's Settings → Billing (Orbit Cloud): the workspace's plan, status, access and trial. Plans are bought and
 * managed through Stripe on the web, not in the iPhone app, so the owner opens the web page in the browser.
 */
export default function BillingScreen() {
  const { client, session, workspaceId } = useOrbit()
  const enabled = useAuthOptions().data?.billing_enabled ?? false
  const query = useQuery({ queryKey: billingKey(workspaceId), queryFn: () => workspaceBilling(client, workspaceId), enabled })
  const billing = query.data
  return (
    <SettingsShell title="Settings" nav={WORKSPACE_NAV} refreshing={query.isRefetching} onRefresh={enabled ? () => void query.refetch() : undefined}>
      {!enabled ? <SettingsCard title="Billing"><Note>This Orbit has no billing.</Note></SettingsCard> : null}
      {enabled && query.isPending ? <SettingsCard title="Billing"><Note loading>Loading billing…</Note></SettingsCard> : null}
      {query.isError ? <SettingsCard title="Billing"><Note error>Billing is not available right now.</Note></SettingsCard> : null}
      {billing ? (
        <SettingsCard title="Billing" description={billingDescription(billing)}>
          <View className="gap-3">
            {billingRows(billing, (iso) => fullDate(iso)).map((row) => (
              <View key={row.label} className="flex-row gap-3">
                <Text className="w-28 text-[13px] text-muted-foreground">{row.label}</Text>
                <Text className="flex-1 text-[13px]">{row.value}</Text>
              </View>
            ))}
            <Note>
              {billing.can_manage
                ? billing.access === 'full'
                  ? 'Change or cancel the plan in Orbit on the web.'
                  : 'Choose a plan in Orbit on the web. You pay through Stripe.'
                : 'Only the workspace owner can choose a plan.'}
            </Note>
            {billing.can_manage && !billing.exempt ? (
              <View className="flex-row">
                <Button label="Open billing on the web" variant="outline" icon="arrowUpRightSquare" onPress={() => void Linking.openURL(`${session.server}/settings/billing?workspace=${encodeURIComponent(workspaceId)}`).catch(() => undefined)} />
              </View>
            ) : null}
          </View>
        </SettingsCard>
      ) : null}
    </SettingsShell>
  )
}
