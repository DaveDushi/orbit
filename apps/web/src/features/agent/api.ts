import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { disconnectChatgpt, readAgentStatus, startChatgptLogin } from '@orbit/api/agent'
import { apiClient } from '@/api/client'

const key = ['agent'] as const

/** The agent's state for the signed-in person. Polls while a ChatGPT sign-in waits for its code to be entered. */
export function useAgentStatus() {
  return useQuery({
    queryKey: key,
    queryFn: () => readAgentStatus(apiClient),
    refetchInterval: (query) => {
      const data = query.state.data
      return data?.enabled && data.login?.state === 'pending' && !data.chatgpt.connected ? 3_000 : false
    },
  })
}

export function useStartChatgptLogin() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: () => startChatgptLogin(apiClient),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: key }),
  })
}

export function useDisconnectChatgpt() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: () => disconnectChatgpt(apiClient),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: key }),
  })
}
