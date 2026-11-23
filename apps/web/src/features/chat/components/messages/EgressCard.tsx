import { useContext, useEffect, useState } from 'react'
import { Globe } from 'reicon-react'
import { toast } from 'sonner'
import { allowAllRequest, cardRequests, egressCardView, type EgressRequest } from '@orbit/api/agentSandbox'
import { ApiProblem } from '@orbit/api/problem'
import { Button } from '@/components/ui/button'
import { useAnswerEgress, useEgressRequests } from '@/features/agent/sandboxApi'
import { WorkspaceContext } from '@/features/workspaces/workspaceContext'

/**
 * The buttons under an agent's "wants to connect" message: one row per host, with its state and the answers this
 * viewer may give (the server decides them), and "Allow all" when it may allow several. The answer goes to Orbit's own
 * route; once no host of the card waits, the run continues by itself as its driver's. A request asked on two cards is
 * on both, and both show its answer.
 */
export function EgressCard({ conversationId, messageId }: { conversationId: string; messageId: string }) {
  const workspaceId = useContext(WorkspaceContext)?.workspace.id ?? ''
  const requests = useEgressRequests(workspaceId, conversationId)
  const mine = cardRequests(requests.data ?? [], messageId)
  const answer = useAnswerEgress(workspaceId)
  const now = useNow(mine.find((request) => request.status === 'pending')?.expires_at ?? null)
  // A card that came in after the list was read (no request was waiting, so it does not reload): read it once more.
  const missing = requests.isSuccess && mine.length === 0
  const { refetch } = requests
  useEffect(() => {
    if (missing) void refetch()
  }, [missing, messageId, refetch])
  if (!workspaceId || mine.length === 0) return null
  const all = allowAllRequest(mine, now)
  return (
    <ul data-slot="egress-card" className="mt-1.5 grid max-w-[640px] min-w-0 divide-y rounded-lg border bg-card">
      {mine.map((request) => <EgressRow key={request.id} workspaceId={workspaceId} request={request} />)}
      {all ? (
        <li className="flex min-w-0 items-center gap-2 p-3">
          <Button
            size="sm"
            disabled={answer.isPending}
            onClick={() =>
              answer.mutate(
                { requestId: all.id, answer: 'allow_once', all: true },
                { onError: (error) => toast.error(error instanceof ApiProblem ? error.detail : 'The answer could not be sent.') },
              )
            }
          >
            Allow all
          </Button>
          <span className="text-xs text-muted-foreground">Allows every waiting host on this card once, for 30 minutes in this thread.</span>
        </li>
      ) : null}
    </ul>
  )
}

function EgressRow({ workspaceId, request }: { workspaceId: string; request: EgressRequest }) {
  const answer = useAnswerEgress(workspaceId)
  const now = useNow(request.expires_at)
  const view = egressCardView(request, now)
  return (
    <li data-slot="egress-request" data-status={view.status} className="flex min-w-0 flex-col gap-2 p-3">
      <div className="flex min-w-0 items-center gap-2">
        <Globe className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
        <span className="min-w-0 truncate font-mono text-[13px] text-foreground">
          {request.host}
          {request.port === 443 ? null : `:${request.port}`}
        </span>
        <span className="ml-auto shrink-0 text-xs text-muted-foreground data-[status=allowed]:text-emerald-600 dark:data-[status=allowed]:text-emerald-400 data-[status=denied]:text-destructive" data-status={view.status}>
          {{ pending: 'Waiting', expired: 'Expired', allowed: 'Allowed', denied: 'Denied' }[view.status]}
        </span>
      </div>
      <p className="text-xs text-muted-foreground">{view.text}</p>
      {view.buttons.length > 0 ? (
        <div className="flex flex-wrap items-center gap-2">
          {view.buttons.map((button) => (
            <Button
              key={button.answer}
              size="sm"
              variant={button.answer === 'allow_once' || (button.answer === 'allow_always' && !request.can_allow_once) ? 'default' : 'outline'}
              className={button.answer === 'deny' ? 'text-destructive' : undefined}
              disabled={answer.isPending}
              onClick={() =>
                answer.mutate(
                  { requestId: request.id, answer: button.answer },
                  { onError: (error) => toast.error(error instanceof ApiProblem ? error.detail : 'The answer could not be sent.') },
                )
              }
            >
              {button.label}
            </Button>
          ))}
        </div>
      ) : null}
      {view.note ? <p className="text-xs text-muted-foreground">{view.note}</p> : null}
    </li>
  )
}

/** The time, ticking once when `until` passes, so a waiting card turns "expired" on its own. */
function useNow(until: number | null) {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    if (until === null || until <= now) return
    const timer = setTimeout(() => setNow(Date.now()), Math.min(until - now + 50, 2 ** 31 - 1))
    return () => clearTimeout(timer)
  }, [until, now])
  return now
}
