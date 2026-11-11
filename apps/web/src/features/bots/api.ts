import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import {
  createWebhookBot,
  deleteWebhookBot,
  readWebhookBotDeliveries,
  readWebhookBots,
  rotateWebhookBotSecret,
  setWebhookBotAvatar,
  updateWebhookBot,
  type WebhookBotInput,
} from '@orbit/api/bots'
import { apiClient } from '@/api/client'

const botsKey = (workspaceId: string) => ['bots', workspaceId] as const

/** Agentic → Bots: the workspace's webhook bots, for owners and admins. */
export function useWebhookBots(workspaceId: string, enabled = true) {
  return useQuery({ queryKey: botsKey(workspaceId), queryFn: () => readWebhookBots(apiClient, workspaceId), enabled })
}

export function useWebhookBotDeliveries(workspaceId: string, id: string) {
  return useQuery({ queryKey: [...botsKey(workspaceId), id, 'deliveries'], queryFn: () => readWebhookBotDeliveries(apiClient, workspaceId, id) })
}

/** A bot's name and avatar are in the member lists too. */
function useRefreshBots(workspaceId: string) {
  const queryClient = useQueryClient()
  return () => {
    void queryClient.invalidateQueries({ queryKey: botsKey(workspaceId) })
    void queryClient.invalidateQueries({ predicate: (query) => query.queryKey.at(-1) === 'members' })
  }
}

/** Saves a bot; a new bot comes back with its signing secret (shown once). */
export function useSaveWebhookBot(workspaceId: string) {
  const refresh = useRefreshBots(workspaceId)
  return useMutation({
    mutationFn: async ({ id, input, avatar }: { id: string | null; input: WebhookBotInput; avatar?: Blob | null }) => {
      const saved = id ? { bot: await updateWebhookBot(apiClient, workspaceId, id, input), signing_secret: null } : await createWebhookBot(apiClient, workspaceId, input)
      // `undefined`: the avatar stays as it is.
      if (avatar !== undefined) await setWebhookBotAvatar(apiClient, workspaceId, saved.bot.id, avatar)
      return saved
    },
    onSettled: refresh,
  })
}

export function useRotateWebhookBotSecret(workspaceId: string) {
  const refresh = useRefreshBots(workspaceId)
  return useMutation({ mutationFn: (id: string) => rotateWebhookBotSecret(apiClient, workspaceId, id), onSettled: refresh })
}

export function useDeleteWebhookBot(workspaceId: string) {
  const refresh = useRefreshBots(workspaceId)
  return useMutation({ mutationFn: (id: string) => deleteWebhookBot(apiClient, workspaceId, id), onSettled: refresh })
}
