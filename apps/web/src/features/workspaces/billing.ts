import { useMutation, useQuery } from '@tanstack/react-query'
import { apiClient } from '@/api/client'
import { queryKeys } from '@/api/queryKeys'
import { billingCheckout, billingPortal, getBilling } from '@/api/generated/sdk.gen'
import { useAuthOptions } from '@/features/auth/api'
import { useWorkspace } from './workspaceContext'

/** Orbit Cloud: workspaces need a subscription to be writable. Off on a self-hosted server. */
export function useBillingEnabled(): boolean {
  return useAuthOptions().data?.billing_enabled ?? false
}

/** The current workspace's subscription; only fetched in Orbit Cloud. */
export function useWorkspaceBilling() {
  const { workspace } = useWorkspace()
  const enabled = useBillingEnabled()
  return useQuery({
    queryKey: queryKeys.billing(workspace.id),
    enabled,
    queryFn: async () => {
      const { data } = await getBilling({ client: apiClient, path: { workspace_id: workspace.id }, throwOnError: true })
      if (!data) throw new Error('Billing response was empty.')
      return data
    },
  })
}

/** The workspace has no subscription that grants access: it can be read but not changed. */
export function useWorkspaceReadOnly(): boolean {
  return useWorkspaceBilling().data?.access === 'read_only'
}

/** Starts a Stripe Checkout for `plan` and sends the browser there. */
export function useBillingCheckout() {
  const { workspace } = useWorkspace()
  return useMutation({
    mutationFn: async (plan: string) => {
      const { data } = await billingCheckout({ client: apiClient, path: { workspace_id: workspace.id }, body: { plan }, throwOnError: true })
      if (!data) throw new Error('Checkout response was empty.')
      return data.url
    },
    onSuccess: (url) => window.location.assign(url),
  })
}

/** Opens the Stripe Billing Portal. */
export function useBillingPortal() {
  const { workspace } = useWorkspace()
  return useMutation({
    mutationFn: async () => {
      const { data } = await billingPortal({ client: apiClient, path: { workspace_id: workspace.id }, throwOnError: true })
      if (!data) throw new Error('Billing portal response was empty.')
      return data.url
    },
    onSuccess: (url) => window.location.assign(url),
  })
}
