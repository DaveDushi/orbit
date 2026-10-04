import type { QueryClient } from '@tanstack/react-query'
import { clearLocalCache } from '@/lib/localCache'

export async function clearExpiredSession(
  queryClient: QueryClient,
  navigate: (path: string) => void,
) {
  await queryClient.cancelQueries({ queryKey: ['current-user'] })
  queryClient.removeQueries({ queryKey: ['current-user'] })
  // Whoever signs in next may be another user.
  void clearLocalCache()
  navigate('/login')
}
