/** The team agent's queries and writes for the signed-in person (the web's `features/agent/api.ts`). */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { disconnectChatgpt, readAgentStatus, startChatgptLogin } from '@orbit/api/agent'

import { useOrbit } from '../orbit-context'

const key = ['agent'] as const

/** Polls while a ChatGPT sign-in waits for its code to be entered. */
export function useAgentStatus() {
  const { client } = useOrbit()
  return useQuery({
    queryKey: key,
    queryFn: () => readAgentStatus(client),
    refetchInterval: (query) => {
      const data = query.state.data
      return data?.enabled && data.login?.state === 'pending' && !data.chatgpt.connected ? 3_000 : false
    },
  })
}

/** The server runs the team agent: its Account settings page is shown. */
export function useAgentEnabled(): boolean {
  return useAgentStatus().data?.enabled === true
}

export function useStartChatgptLogin() {
  const { client } = useOrbit()
  const queryClient = useQueryClient()
  return useMutation({ mutationFn: () => startChatgptLogin(client), onSettled: () => void queryClient.invalidateQueries({ queryKey: key }) })
}

export function useDisconnectChatgpt() {
  const { client } = useOrbit()
  const queryClient = useQueryClient()
  return useMutation({ mutationFn: () => disconnectChatgpt(client), onSettled: () => void queryClient.invalidateQueries({ queryKey: key }) })
}
