import { useEffect, useState } from 'react'
import { View } from 'react-native'
import { cardRequests, egressCardView, type EgressRequest } from '@orbit/api/agentSandbox'

import { problemDetail } from '../../data/agentic'
import { useAnswerEgress, useEgressRequests } from '../../hooks/agentSandbox'
import { useTheme } from '../../theme'
import { Button, Icon, Text } from '../../ui'

const STATUS_LABEL = { pending: 'Waiting', expired: 'Expired', allowed: 'Allowed', denied: 'Denied' } as const

/**
 * The web's `EgressCard`: under an agent's "wants to connect" message, one row per host with its state and the answers
 * this viewer may give (the server decides them). The answer goes to Orbit's own route; the run continues as its driver's.
 */
export function EgressCard({ conversationId, messageId }: { conversationId: string; messageId: string }) {
  const requests = useEgressRequests(conversationId)
  const mine = cardRequests(requests.data ?? [], messageId)
  if (mine.length === 0) return null
  return (
    <View className="mt-1.5 rounded-lg border border-border bg-card">
      {mine.map((request, index) => <EgressRow key={request.id} request={request} first={index === 0} />)}
    </View>
  )
}

function EgressRow({ request, first }: { request: EgressRequest; first: boolean }) {
  const { colors } = useTheme()
  const answer = useAnswerEgress()
  const now = useNow(request.expires_at)
  const view = egressCardView(request, now)
  const statusColor = view.status === 'allowed' ? 'text-emerald-600 dark:text-emerald-400' : view.status === 'denied' ? 'text-destructive' : 'text-muted-foreground'
  return (
    <View className={`gap-2 p-3 border-border ${first ? '' : 'border-t'}`}>
      <View className="flex-row items-center gap-2">
        <Icon name="cloudConnection" size={16} color={colors.mutedForeground} />
        <Text numberOfLines={1} className="flex-1 font-mono text-[13px] text-foreground">
          {request.host}
          {request.port === 443 ? '' : `:${request.port}`}
        </Text>
        <Text className={`text-[12px] ${statusColor}`}>{STATUS_LABEL[view.status]}</Text>
      </View>
      <Text className="text-[12px] leading-[17px] text-muted-foreground">{view.text}</Text>
      {view.buttons.length > 0 ? (
        <View className="flex-row flex-wrap gap-2">
          {view.buttons.map((button) => (
            <Button
              key={button.answer}
              label={button.label}
              size="sm"
              variant={button.answer === 'deny' ? 'destructive' : button.answer === 'allow_once' || !request.can_allow_once ? 'primary' : 'outline'}
              disabled={answer.isPending}
              onPress={() => answer.mutate({ requestId: request.id, answer: button.answer })}
            />
          ))}
        </View>
      ) : null}
      {view.note ? <Text className="text-[12px] leading-[17px] text-muted-foreground">{view.note}</Text> : null}
      {answer.isError ? <Text className="text-[12px] leading-[17px] text-destructive">{problemDetail(answer.error, 'The answer could not be sent.')}</Text> : null}
    </View>
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
