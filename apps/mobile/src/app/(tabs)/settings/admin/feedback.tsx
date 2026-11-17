import type { ImprovementRequest, ImprovementStatus } from '@orbit/api/improvements'
import { View } from 'react-native'

import { AdminShell, Badge, type BadgeTone, Note, RowText, SettingsCard, SettingsRow, Toggle } from '../../../../components/settings/Shell'
import { confirmAction, showError } from '../../../../hooks/chat'
import { useImprovementSettings, useImprovements, useSaveImprovementSettings, useWithdrawImprovement } from '../../../../hooks/improvements'
import { Button, Text } from '../../../../ui'

const STATUS: Record<ImprovementStatus, { label: string; tone: BadgeTone }> = {
  new: { label: 'New', tone: 'outline' },
  building: { label: 'Building', tone: 'secondary' },
  review: { label: 'In review', tone: 'secondary' },
  live: { label: 'Live', tone: 'primary' },
  declined: { label: 'Declined', tone: 'destructive' },
}

const date = (value: string) => new Date(value).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' })

/** The web's Admin → General "Suggest a change" switch and Admin → Feedback & fixes: the requests and what the agent reported. */
export default function AdminFeedbackScreen() {
  const settings = useImprovementSettings()
  const save = useSaveImprovementSettings()
  const enabled = settings.data?.enabled ?? false
  const requests = useImprovements(enabled)
  return (
    <AdminShell refreshing={requests.isRefetching} onRefresh={() => void Promise.all([settings.refetch(), enabled ? requests.refetch() : null])}>
      <SettingsCard title="Suggest a change" description="Instance admins send change requests from their account menu; an agent on this server builds them.">
        {settings.isError ? (
          <Note error>Settings could not be loaded.</Note>
        ) : (
          <View className="flex-row items-center gap-3">
            <Text className="text-[14px] flex-1">{settings.isPending ? 'Loading…' : enabled ? 'On' : 'Off'}</Text>
            <Toggle
              label="Suggest a change"
              value={enabled}
              disabled={settings.isPending || save.isPending}
              onChange={(value) => save.mutate(value, { onError: (error) => showError('Could not save', error) })}
            />
          </View>
        )}
      </SettingsCard>
      {enabled ? (
        <SettingsCard title="Feedback & fixes" description="Changes asked for with Suggest a change, newest first. An agent reports the status here." flush>
          {requests.isPending ? <SettingsRow first><Note loading>Loading requests…</Note></SettingsRow> : null}
          {requests.isError ? (
            <SettingsRow first>
              <Note error>Requests could not be loaded.</Note>
              <Button label="Retry" variant="ghost" onPress={() => void requests.refetch()} />
            </SettingsRow>
          ) : null}
          {requests.data?.length === 0 ? <SettingsRow first><RowText title="No requests yet" subtitle="Open your account menu and choose Suggest a change." /></SettingsRow> : null}
          {requests.data?.map((request, index) => <RequestRow key={request.id} request={request} first={index === 0} />)}
        </SettingsCard>
      ) : null}
    </AdminShell>
  )
}

/** The text and the note are untrusted: plain text only. */
function RequestRow({ request, first }: { request: ImprovementRequest; first: boolean }) {
  const withdraw = useWithdrawImprovement()
  const status = STATUS[request.status]
  const onWithdraw = async () => {
    if (!(await confirmAction('Withdraw this request?', 'The agent will not build it. You cannot undo this.', 'Withdraw'))) return
    withdraw.mutate(request.id, { onError: (error) => showError('The request could not be withdrawn', error) })
  }
  return (
    <SettingsRow first={first}>
      <View className="flex-1 gap-2">
        <View className="flex-row items-start gap-3">
          <Text className="text-[13px] leading-[19px] flex-1">{request.text}</Text>
          <Badge label={status.label} tone={status.tone} />
        </View>
        {request.note || request.link ? (
          <View className="gap-1 rounded-md bg-muted/50 px-3 py-2">
            {request.note ? (
              <Text className="text-[12px] text-muted-foreground">
                <Text className="font-noto-medium text-[12px] text-foreground">Note: </Text>
                {request.note}
              </Text>
            ) : null}
            {request.link ? <Text numberOfLines={1} selectable className="text-[12px] text-muted-foreground">{request.link}</Text> : null}
          </View>
        ) : null}
        <View className="flex-row flex-wrap items-center gap-x-3 gap-y-1">
          <Text className="text-[12px] text-muted-foreground">{request.author.name}</Text>
          <Text className="text-[12px] text-muted-foreground">Sent {date(request.created_at)}</Text>
          {request.updated_at !== request.created_at ? <Text className="text-[12px] text-muted-foreground">Updated {date(request.updated_at)}</Text> : null}
          <Text numberOfLines={1} className="text-[12px] text-muted-foreground">
            {request.context.path} · {request.context.viewport}
          </Text>
          {request.status === 'new' ? <Button label={withdraw.isPending ? 'Withdrawing…' : 'Withdraw'} size="sm" variant="ghost" disabled={withdraw.isPending} onPress={() => void onWithdraw()} /> : null}
        </View>
      </View>
    </SettingsRow>
  )
}
