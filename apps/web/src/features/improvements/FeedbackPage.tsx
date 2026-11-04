import { Navigate } from 'react-router'
import { toast } from 'sonner'
import type { ImprovementRequest, ImprovementStatus } from '@orbit/api/improvements'
import { ApiProblem } from '@orbit/api/problem'
import { confirmAction } from '@/components/common/confirmAction'
import { SettingsCard } from '@/components/common/SettingsCard'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { SettingsRow } from '@/features/settings/components/SettingsParts'
import { useImprovementSettings, useImprovements, useWithdrawImprovement } from './api'

const STATUS_LABELS: Record<ImprovementStatus, string> = {
  new: 'New',
  building: 'Building',
  review: 'In review',
  live: 'Live',
  declined: 'Declined',
}

const STATUS_VARIANTS: Record<ImprovementStatus, 'default' | 'secondary' | 'outline' | 'destructive'> = {
  new: 'outline',
  building: 'secondary',
  review: 'secondary',
  live: 'default',
  declined: 'destructive',
}

function date(value: string) {
  return new Date(value).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' })
}

/** Admin → Feedback & fixes: the requests sent with "Suggest a change" and what the agent reported. */
export function FeedbackPage() {
  const settings = useImprovementSettings()
  if (settings.isPending) return <SettingsCard title="Feedback & fixes"><p role="status">Loading…</p></SettingsCard>
  if (settings.isError) return <SettingsCard title="Feedback & fixes"><p role="alert">Settings could not be loaded. <Button variant="ghost" onClick={() => void settings.refetch()}>Retry</Button></p></SettingsCard>
  // the page is there only while the switch is on (Admin → General)
  if (!settings.data.enabled) return <Navigate to="/admin" replace />
  return <FeedbackList />
}

function FeedbackList() {
  const requests = useImprovements()
  return (
    <SettingsCard
      title="Feedback & fixes"
      description="Changes asked for with Suggest a change, newest first. An agent on this server builds them and reports the status here."
      flush
    >
      <div className="flex flex-col divide-y text-[13px]">
        {requests.isPending ? <SettingsRow role="status">Loading requests…</SettingsRow> : null}
        {requests.isError ? <SettingsRow role="alert">Requests could not be loaded. <Button variant="ghost" onClick={() => void requests.refetch()}>Retry</Button></SettingsRow> : null}
        {requests.data?.length === 0 ? <SettingsRow className="text-muted-foreground">No requests yet. Open your account menu and choose Suggest a change.</SettingsRow> : null}
        {requests.data?.map((request) => <RequestRow key={request.id} request={request} />)}
      </div>
    </SettingsCard>
  )
}

/** The text and the note are untrusted: React renders them as plain text, never as HTML or markdown. */
function RequestRow({ request }: { request: ImprovementRequest }) {
  const withdraw = useWithdrawImprovement()
  const changed = request.updated_at !== request.created_at

  const onWithdraw = async () => {
    const confirmed = await confirmAction({
      title: 'Withdraw this request?',
      description: 'The agent will not build it. You cannot undo this.',
      confirmLabel: 'Withdraw',
      danger: true,
    })
    if (!confirmed) return
    withdraw.mutate(request.id, {
      onError: (error) => toast.error(error instanceof ApiProblem ? error.detail : 'The request could not be withdrawn.'),
    })
  }

  return (
    <SettingsRow className="flex-col items-stretch gap-2" data-status={request.status}>
      <div className="flex min-w-0 items-start gap-3">
        <p className="min-w-0 flex-1 text-[13px] break-words whitespace-pre-wrap text-foreground">{request.text}</p>
        <Badge variant={STATUS_VARIANTS[request.status]}>{STATUS_LABELS[request.status]}</Badge>
      </div>
      {request.note || request.link ? (
        <div className="flex flex-col gap-1 rounded-md bg-muted/50 px-3 py-2 text-xs text-muted-foreground">
          {request.note ? <p className="break-words whitespace-pre-wrap"><span className="font-medium text-foreground">Note: </span>{request.note}</p> : null}
          {request.link ? <p className="truncate font-mono">{request.link}</p> : null}
        </div>
      ) : null}
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
        <span>{request.author.name}</span>
        <span>Sent <time dateTime={request.created_at}>{date(request.created_at)}</time></span>
        {changed ? <span>Updated <time dateTime={request.updated_at}>{date(request.updated_at)}</time></span> : null}
        <span className="truncate font-mono" title="Page and window size when it was sent">{request.context.path} · {request.context.viewport}</span>
        {request.status === 'new' ? (
          <Button variant="ghost" size="sm" className="ml-auto" disabled={withdraw.isPending} onClick={() => void onWithdraw()}>
            {withdraw.isPending ? 'Withdrawing…' : 'Withdraw'}
          </Button>
        ) : null}
      </div>
    </SettingsRow>
  )
}
