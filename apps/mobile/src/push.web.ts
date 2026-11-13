/** The web preview has no APNs: nothing registers. */
import { useMutation, useQuery } from '@tanstack/react-query'

import type { OrbitClient } from './data/orbit'
import type { ThisDevice } from './data/push'

export function usePushNotifications(_client: OrbitClient) {}

export async function unregisterPush(_client: OrbitClient) {}

export function useThisDevice(_client: OrbitClient) {
  const state = useQuery({ queryKey: ['push-this-device'], queryFn: async (): Promise<ThisDevice> => 'unsupported' })
  const none = async () => undefined
  return { state, turnOn: useMutation({ mutationFn: none }), turnOff: useMutation({ mutationFn: none }) }
}
