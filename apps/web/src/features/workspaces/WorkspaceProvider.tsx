import { useEffect, useMemo, type ReactNode } from 'react'
import { Navigate, useLocation, useNavigate } from 'react-router'
import type { WorkspaceRecord } from '@/api/generated/types.gen'
import { useWorkspaces } from './api'
import { selectedWorkspaceId, switchWorkspaceHref } from './navigation'
import { WorkspaceContext } from './workspaceContext'
import { OrbitUnavailable } from '@/components/common/OrbitUnavailable'
import { LoadingScreen } from '@/components/common/LoadingScreen'
import { firstLoadFailed } from '@/lib/connection'
const preferenceKey = 'orbit:selected_workspace'
const emptyWorkspaces: WorkspaceRecord[] = []

export function WorkspaceProvider({ children }: { children: ReactNode }) {
  const query = useWorkspaces()
  const location = useLocation()
  const navigate = useNavigate()
  const workspaces = query.data ?? emptyWorkspaces
  const workspaceId = selectedWorkspaceId(
    workspaces,
    location.search,
    window.localStorage.getItem(preferenceKey),
  )
  const workspace = workspaces.find((item) => item.id === workspaceId)

  useEffect(() => {
    if (workspaceId) window.localStorage.setItem(preferenceKey, workspaceId)
  }, [workspaceId])

  const value = useMemo(() => workspace ? {
    workspace,
    workspaces,
    selectWorkspace: (id: string) => navigate(switchWorkspaceHref(location.pathname, location.search, id)),
  } : null, [location.pathname, location.search, navigate, workspace, workspaces])

  // With workspaces from before, a refetch that lost the connection keeps the app on screen (the banner in the shell
  // tells the user); one the server refused does not.
  if (firstLoadFailed(query)) return <OrbitUnavailable detail="Your workspaces could not be loaded." onRetry={() => void query.refetch()} />
  if (query.data === undefined) return <LoadingScreen />
  // No workspace yet (the root user right after setup, or someone whose last workspace is gone): create one.
  if (!value) return <Navigate to="/onboarding" replace />
  return <WorkspaceContext.Provider value={value}>{children}</WorkspaceContext.Provider>
}
