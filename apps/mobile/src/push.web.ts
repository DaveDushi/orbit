/** The web preview has no APNs: nothing registers. */
import type { OrbitClient } from './data/orbit'

export function usePushNotifications(_client: OrbitClient) {}

export async function unregisterPush(_client: OrbitClient) {}
