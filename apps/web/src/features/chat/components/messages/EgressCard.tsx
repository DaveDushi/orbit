import { useContext, useEffect, useState } from 'react'
import { Globe } from 'reicon-react'
import { toast } from 'sonner'
import { cardRequests, egressCardView, type EgressRequest } from '@orbit/api/agentSandbox'
import { ApiProblem } from '@orbit/api/problem'
import { Button } from '@/components/ui/button'
import { useAnswerEgress, useEgressRequests } from '@/features/agent/sandboxApi'
import { WorkspaceContext } from '@/features/workspaces/workspaceContext'

/**
 * The buttons under an agent's "wants to connect" message: one row per host, with its state and the answers this
 * viewer may give (the server decides them). The answer goes to Orbit's own route, so the run continues as its driver's.
 */
export function EgressCard({ conversationId, messageId }: { conversationId: string; messageId: string }) {
  const workspaceId = useContext(WorkspaceContext)?.workspace.id ?? ''
  const requests = useEgressRequests(workspaceId, conversationId)
  const mine = cardRequests(requests.data ?? [], messageId)
  if (!workspaceId || mine.length === 0) return null
  return (
    <ul data-slot="egress-card" className="mt-1.5 grid max-w-[640px] min-w-0 divide-y rounded-lg border bg-card">
      {mine.map((request) => <EgressRow key={request.id} workspaceId={workspaceId} request={request} />)}
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
