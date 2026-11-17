import type { ImprovementRequest, ImprovementStatus } from '@orbit/api/improvements'
import { useState } from 'react'
import { View } from 'react-native'

import { ConfirmDialog, type ConfirmRequest } from '../../../../components/settings/Dialog'
import { AdminShell, Badge, type BadgeTone, Note, SettingsCard } from '../../../../components/settings/Shell'
import { useImprovements, useImprovementSettings, useWithdrawImprovement } from '../../../../hooks/improvements'
import { errorMessage } from '../../../../session-context'
import { Button, Text } from '../../../../ui'

const STATUS: Record<ImprovementStatus, { label: string; tone: BadgeTone }> = {
  new: { label: 'New', tone: 'outline' },
  building: { label: 'Building', tone: 'secondary' },
  review: { label: 'In review', tone: 'secondary' },
  live: { label: 'Live', tone: 'primary' },
  declined: { label: 'Declined', tone: 'destructive' },
}

const date = (value: string) => new Date(value).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' })

/** Admin → Feedback & fixes: the requests sent with Suggest a change, and what the agent reported. */
export default function FeedbackScreen() {
  const settings = useImprovementSettings()
  const on = settings.data?.enabled === true
  const requests = useImprovements(on)
  return (
    <AdminShell refreshing={requests.isRefetching} onRefresh={() => void requests.refetch()}>
      {settings.isPending ? <Note loading>Loading…</Note> : null}
      {settings.isError ? <Note error>Settings could not be loaded.</Note> : null}
      {settings.data && !on ? (
        <SettingsCard title="Feedback & fixes">
          <Note>Suggest a change is off. Turn it on in Admin → General.</Note>
        </SettingsCard>
      ) : null}
      {on ? (
        <SettingsCard title="Feedback & fixes" description="Changes asked for with Suggest a change, newest first. An agent on this server builds them and reports the status here." flush>
          {requests.isPending ? <View className="p-4"><Note loading>Loading requests…</Note></View> : null}
          {requests.isError ? <View className="p-4"><Note error>Requests could not be loaded.</Note></View> : null}
          {requests.data?.length === 0 ? <View className="p-4"><Note>No requests yet. Open your account menu and choose Suggest a change.</Note></View> : null}
          {requests.data?.map((request, index) => <RequestRow key={request.id} request={request} first={index === 0} />)}
        </SettingsCard>
      ) : null}
    </AdminShell>
  )
}

/** The text and the note are untrusted: plain text only. */
function RequestRow({ request, first }: { request: ImprovementRequest; first: boolean }) {
  const withdraw = useWithdrawImprovement()
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null)
  const status = STATUS[request.status]
  return (
    <View className={`gap-2 px-4 py-3 ${first ? '' : 'border-t border-border'}`}>
      <View className="flex-row items-start gap-3">
        <Text selectable className="flex-1 text-[13px]">{request.text}</Text>
        <Badge label={status.label} tone={status.tone} />
      </View>
      {request.note || request.link ? (
        <View className="gap-1 rounded-md bg-muted/50 px-3 py-2">
          {request.note ? <Text selectable className="text-[12px] text-muted-foreground"><Text className="font-noto-medium text-[12px] text-foreground">Note: </Text>{request.note}</Text> : null}
          {request.link ? <Text selectable numberOfLines={1} className="font-mono text-[12px] text-muted-foreground">{request.link}</Text> : null}
        </View>
      ) : null}
      <Text className="text-[12px] text-muted-foreground">
        {request.author.name} · Sent {date(request.created_at)}
        {request.updated_at !== request.created_at ? ` · Updated ${date(request.updated_at)}` : ''} · {request.context.path} · {request.context.viewport}
      </Text>
      {request.status === 'new' ? (
        <View className="flex-row">
          <Button
            label={withdraw.isPending ? 'Withdrawing…' : 'Withdraw'}
            variant="ghost"
            size="sm"
            disabled={withdraw.isPending}
            onPress={() => setConfirm({ title: 'Withdraw this request?', description: 'The agent will not build it. You cannot undo this.', confirmLabel: 'Withdraw', danger: true, onConfirm: () => withdraw.mutate(request.id) })}
          />
        </View>
      ) : null}
      {withdraw.isError ? <Note error>The request could not be withdrawn. {errorMessage(withdraw.error)}</Note> : null}
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </View>
  )
}
