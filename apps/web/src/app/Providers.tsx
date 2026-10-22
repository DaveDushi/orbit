import { QueryClient, QueryClientProvider, useQueryClient } from '@tanstack/react-query'
import { useEffect, useState, type ReactNode } from 'react'
import { BrowserRouter, useNavigate } from 'react-router'
import { toast } from 'sonner'
import { READ_ONLY_EVENT, UNAUTHORIZED_EVENT } from '@/api/client'
import { ThemeProvider } from '@/lib/theme'
import { TooltipProvider } from '@/components/ui/tooltip'
import { clearExpiredSession } from './authSession'

function UnauthorizedSessionHandler() {
  const queryClient = useQueryClient()
  const navigate = useNavigate()

  useEffect(() => {
    const unauthorized = () => void clearExpiredSession(queryClient, (path) => navigate(path, { replace: true }))
    window.addEventListener(UNAUTHORIZED_EVENT, unauthorized)
    return () => window.removeEventListener(UNAUTHORIZED_EVENT, unauthorized)
  }, [navigate, queryClient])

  return null
}

/**
 * Orbit Cloud: a write refused because the workspace has no subscription gets one clear toast (not a generic failure)
 * with a way to the billing page. `ReadOnlyBanner` fetches the billing state again.
 */
function ReadOnlyWorkspaceHandler() {
  const navigate = useNavigate()

  useEffect(() => {
    const refused = (event: Event) => {
      const detail = event instanceof CustomEvent && typeof event.detail === 'string' ? event.detail : 'This workspace is read-only.'
      toast.error(detail, {
        id: READ_ONLY_EVENT,
        action: { label: 'Billing', onClick: () => navigate('/settings/billing') },
      })
    }
    window.addEventListener(READ_ONLY_EVENT, refused)
    return () => window.removeEventListener(READ_ONLY_EVENT, refused)
  }, [navigate])

  return null
}

export function Providers({ children }: { children: ReactNode }) {
  const [queryClient] = useState(
    () =>
      new QueryClient({
        defaultOptions: {
          queries: { retry: false, staleTime: 30_000 },
          mutations: { retry: false },
        },
      }),
  )

  return (
    <ThemeProvider>
      <QueryClientProvider client={queryClient}>
        <BrowserRouter>
          <UnauthorizedSessionHandler />
          <ReadOnlyWorkspaceHandler />
          <TooltipProvider>{children}</TooltipProvider>
        </BrowserRouter>
      </QueryClientProvider>
    </ThemeProvider>
  )
}
