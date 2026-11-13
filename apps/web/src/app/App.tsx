import { ConnectionScreen } from './ConnectionScreen'
import { lazy, Suspense, useEffect } from 'react'
import { ConfirmationModalHost } from '@/components/common/ConfirmationModal'
import { Toaster } from '@/components/ui/sonner'
import { NewVersionNotice } from '@/app/NewVersionNotice'
import { Navigate, Outlet, Route, Routes, useNavigate } from 'react-router'
import { MovedTo } from '@/app/MovedTo'
import { setAppNavigate } from '@/lib/navigateBridge'
import { chatEnabled } from '@/app/shell/productNavigation'
import { AuthGate, GuestGate } from '@/features/auth/AuthGate'
import { AcceptInvitationPage } from '@/features/auth/pages/AcceptInvitationPage'
import { OAuthConsentPage } from '@/features/auth/pages/OAuthConsentPage'
import { LoginPage } from '@/features/auth/pages/LoginPage'
import { RecoveryPage } from '@/features/auth/pages/RecoveryPage'
import { ConfirmEmailPage } from '@/features/auth/pages/ConfirmEmailPage'
import { RegisterPage } from '@/features/auth/pages/RegisterPage'
import { SetupPage } from '@/features/auth/pages/SetupPage'
import { WorkspaceProvider } from '@/features/workspaces/WorkspaceProvider'
import { PublicSharePage } from '@/features/drive/pages/PublicSharePage'
import { GuestWorkspaceGate } from '@/features/workspaces/GuestWorkspaceGate'

// The signed-in app loads on first use, one chunk per area, so the sign-in pages load only what they need.
const loadShell = () => import('@/app/shell/AppShell')
const loadChat = () => import('@/features/chat/pages/ChatPage')
const loadTasks = () => import('./taskPages')
const loadHome = () => import('@/features/home/pages/HomePage')
const loadDocs = () => import('@/features/docs/pages/DocsPage')
const loadDrive = () => import('@/features/drive/pages/DrivePage')
const loadSettings = () => import('./settingsPages')
const loadApps = () => import('@/features/apps/AppPage')
const loadInbox = () => import('@/features/inbox/pages/InboxPage')
const loadOnboarding = () => import('@/features/onboarding/OnboardingPage')
const ChatPage = lazy(() => loadChat().then((module) => ({ default: module.ChatPage })))
const ProjectSettingsPage = lazy(() => loadTasks().then((module) => ({ default: module.ProjectSettingsPage })))
const TasksPage = lazy(() => loadTasks().then((module) => ({ default: module.TasksPage })))
const HomePage = lazy(() => loadHome().then((module) => ({ default: module.HomePage })))
const TaskTrashPage = lazy(() => loadTasks().then((module) => ({ default: module.TaskTrashPage })))
const TrashedTaskPage = lazy(() => loadTasks().then((module) => ({ default: module.TrashedTaskPage })))
const TrashedProjectPage = lazy(() => loadTasks().then((module) => ({ default: module.TrashedProjectPage })))
const DocsPage = lazy(() => loadDocs().then((module) => ({ default: module.DocsPage })))
const DrivePage = lazy(() => loadDrive().then((module) => ({ default: module.DrivePage })))
const AccountLayout = lazy(() => loadSettings().then((module) => ({ default: module.AccountLayout })))
const AdminLayout = lazy(() => loadSettings().then((module) => ({ default: module.AdminLayout })))
const SettingsLayout = lazy(() => loadSettings().then((module) => ({ default: module.SettingsLayout })))
const AdminAuditPage = lazy(() => loadSettings().then((module) => ({ default: module.AdminAuditPage })))
const AdminBackupsPage = lazy(() => loadSettings().then((module) => ({ default: module.AdminBackupsPage })))
const AdminEmailPage = lazy(() => loadSettings().then((module) => ({ default: module.AdminEmailPage })))
const AdminJobsPage = lazy(() => loadSettings().then((module) => ({ default: module.AdminJobsPage })))
const AdminLimitsPage = lazy(() => loadSettings().then((module) => ({ default: module.AdminLimitsPage })))
const AdminStoragePage = lazy(() => loadSettings().then((module) => ({ default: module.AdminStoragePage })))
const AdminSettingsPage = lazy(() => loadSettings().then((module) => ({ default: module.AdminSettingsPage })))
const AdminUsersPage = lazy(() => loadSettings().then((module) => ({ default: module.AdminUsersPage })))
const FeedbackPage = lazy(() => loadSettings().then((module) => ({ default: module.FeedbackPage })))
const GeneralPage = lazy(() => loadSettings().then((module) => ({ default: module.GeneralPage })))
const GithubPage = lazy(() => loadSettings().then((module) => ({ default: module.GithubPage })))
const EmojiPage = lazy(() => loadSettings().then((module) => ({ default: module.EmojiPage })))
const DangerZonePage = lazy(() => loadSettings().then((module) => ({ default: module.DangerZonePage })))
const ExportPage = lazy(() => loadSettings().then((module) => ({ default: module.ExportPage })))
const BillingPage = lazy(() => loadSettings().then((module) => ({ default: module.BillingPage })))
const MembersPage = lazy(() => loadSettings().then((module) => ({ default: module.MembersPage })))
const SecurityPage = lazy(() => loadSettings().then((module) => ({ default: module.SecurityPage })))
const SessionsPage = lazy(() => loadSettings().then((module) => ({ default: module.SessionsPage })))
const AgenticTabs = lazy(() => loadSettings().then((module) => ({ default: module.AgenticTabs })))
const AgenticWorkspace = lazy(() => loadSettings().then((module) => ({ default: module.AgenticWorkspace })))
const AgenticRoutines = lazy(() => loadSettings().then((module) => ({ default: module.AgenticRoutines })))
const AgenticAgents = lazy(() => loadSettings().then((module) => ({ default: module.AgenticAgents })))
const AgenticYou = lazy(() => loadSettings().then((module) => ({ default: module.AgenticYou })))
const ShortcutsPage = lazy(() => loadSettings().then((module) => ({ default: module.ShortcutsPage })))
const StickersPage = lazy(() => loadSettings().then((module) => ({ default: module.StickersPage })))
const StoragePage = lazy(() => loadSettings().then((module) => ({ default: module.StoragePage })))
const NotificationsPage = lazy(() => loadSettings().then((module) => ({ default: module.NotificationsPage })))
const ApiTokensPage = lazy(() => loadSettings().then((module) => ({ default: module.ApiTokensPage })))
const FileStoragePage = lazy(() => loadSettings().then((module) => ({ default: module.FileStoragePage })))
const ProfilePage = lazy(() => loadSettings().then((module) => ({ default: module.ProfilePage })))
const AppearancePage = lazy(() => loadSettings().then((module) => ({ default: module.AppearancePage })))
const AppPage = lazy(() => loadApps().then((module) => ({ default: module.AppPage })))
const AppsSettingsPage = lazy(() => loadSettings().then((module) => ({ default: module.AppsSettingsPage })))
const InboxPage = lazy(() => loadInbox().then((module) => ({ default: module.InboxPage })))
const OnboardingPage = lazy(() => loadOnboarding().then((module) => ({ default: module.OnboardingPage })))
const ViewsPage = lazy(() => loadTasks().then((module) => ({ default: module.ViewsPage })))

// A signed-in start URL loads its code while the session check runs, not after it, and the shell waits for the start
// page, so the first screen shows at once and not in parts. The editor is not loaded here: on a phone its download
// delayed the session check by seconds (the editor still starts when the page shows it).
const startPath = window.location.pathname
const startAreas: Record<string, () => Promise<unknown>> = {
  '': loadHome, tasks: loadTasks, 'tasks-trash': loadTasks, views: loadTasks, docs: loadDocs, drive: loadDrive, chat: loadChat, apps: loadApps,
  activity: loadInbox, onboarding: loadOnboarding, profile: loadSettings, admin: loadSettings, settings: loadSettings,
}
const signedInStart = !/^\/(setup|login|recovery|register|oauth|accept-invitation|confirm-email|s)(\/|$)/.test(startPath)
const startPage = signedInStart ? startAreas[startPath.split('/')[1]]?.() : undefined
if (signedInStart) void loadShell()
const AppShell = lazy(async () => {
  const [module] = await Promise.all([loadShell(), startPage])
  return { default: module.AppShell }
})

/** Hands the router's navigate function to non-component code (markdown links). */
function NavigateBridge() {
  const navigate = useNavigate()
  useEffect(() => {
    setAppNavigate((to) => void navigate(to))
  }, [navigate])
  return null
}

export default function App() {
  return (
    <>
      <NavigateBridge />
      <ConfirmationModalHost />
      <Toaster />
      <NewVersionNotice />
      <ConnectionScreen />
      {/* blank while the first screen's code loads, as before the split */}
      <Suspense fallback={null}>
      <Routes>
        <Route element={<GuestGate />}>
          <Route path="setup" element={<SetupPage />} />
          <Route path="login" element={<LoginPage />} />
          <Route path="recovery" element={<RecoveryPage />} />
          <Route path="register" element={<RegisterPage />} />
        </Route>
        <Route path="oauth/consent" element={<OAuthConsentPage />} />
        <Route path="accept-invitation" element={<AcceptInvitationPage />} />
        <Route path="confirm-email" element={<ConfirmEmailPage />} />
        {/* A public Drive link: no account, no session. */}
        <Route path="s/:token" element={<PublicSharePage />} />
        <Route element={<AuthGate />}>
          <Route path="onboarding" element={<OnboardingPage />} />
          <Route element={<WorkspaceProvider><Outlet /></WorkspaceProvider>}>
            <Route element={<AppShell />}>
              <Route element={<GuestWorkspaceGate />}>
                <Route index element={<HomePage />} />
                <Route path="apps/:slug" element={<AppPage />} />
                <Route path="tasks" element={<TasksPage />} />
                <Route path="tasks/projects/:projectId/settings/:section?" element={<ProjectSettingsPage />} />
                <Route path="tasks/:taskId" element={<TasksPage />} />
                <Route path="tasks-trash" element={<TaskTrashPage />} />
                <Route path="tasks-trash/task/:taskId" element={<TrashedTaskPage />} />
                <Route path="tasks-trash/project/:projectId" element={<TrashedProjectPage />} />
                <Route path="views" element={<ViewsPage />} />
                <Route path="views/:viewId" element={<TasksPage />} />
                <Route path="views/:viewId/:taskId" element={<TasksPage />} />
                <Route path="docs/trash" element={<DocsPage view="trash" />} />
                <Route path="docs/trash/:pageId" element={<DocsPage view="trash" />} />
                <Route path="docs/import/:importId?" element={<DocsPage view="import" />} />
                <Route path="docs/:pageId?" element={<DocsPage />} />
                <Route path="drive" element={<DrivePage />} />
                <Route path="drive/trash" element={<DrivePage view="trash" />} />
                <Route path="drive/:folderId" element={<DrivePage />} />
                <Route path="mail/*" element={<Navigate to="/tasks" replace />} />
                {chatEnabled ? (
                  <>
                    <Route path="chat" element={<ChatPage />} />
                    <Route path="chat/unreads" element={<ChatPage />} />
                    <Route path="chat/threads" element={<ChatPage />} />
                    <Route path="chat/:conversationId" element={<ChatPage />} />
                    <Route path="chat/:conversationId/thread/:messageId" element={<ChatPage />} />
                  </>
                ) : (
                  <Route path="chat/*" element={<Navigate to="/tasks" replace />} />
                )}
                <Route path="dm/*" element={<Navigate to={chatEnabled ? '/chat' : '/tasks'} replace />} />
                <Route path="activity" element={<InboxPage />} />
                <Route path="inbox" element={<MovedTo path="/activity" />} />
                {/* the Agentic page moved to the workspace settings; old links, bookmarks and agent messages still work */}
                <Route path="agentic/*" element={<MovedTo path="/settings/agentic" />} />
                <Route path="agentic/workspace" element={<MovedTo path="/settings/agentic/workspace" />} />
                <Route path="agentic/admin" element={<MovedTo path="/settings/agentic/agents" />} />
                <Route path="profile" element={<AccountLayout />}>
                  <Route index element={<ProfilePage />} />
                  <Route path="general" element={<ProfilePage />} />
                  <Route path="appearance" element={<AppearancePage />} />
                  <Route path="notifications" element={<NotificationsPage />} />
                  <Route path="security" element={<SecurityPage />} />
                  <Route path="sessions" element={<SessionsPage />} />
                  <Route path="agentic" element={<AgenticYou />} />
                  <Route path="shortcuts" element={<ShortcutsPage />} />
                  <Route path="storage" element={<StoragePage />} />
                  {/* the Agentic page has these now; old links, bookmarks and agent messages still work */}
                  <Route path="agent/*" element={<MovedTo path="/profile/agentic" />} />
                  <Route path="agent-memory" element={<MovedTo path="/profile/agentic" />} />
                </Route>
                <Route path="admin" element={<AdminLayout />}>
                  <Route index element={<AdminSettingsPage />} />
                  <Route path="general" element={<AdminSettingsPage />} />
                  <Route path="users" element={<AdminUsersPage />} />
                  <Route path="email" element={<AdminEmailPage />} />
                  <Route path="limits" element={<AdminLimitsPage />} />
                  <Route path="audit" element={<AdminAuditPage />} />
                  <Route path="storage" element={<AdminStoragePage />} />
                  <Route path="backups" element={<AdminBackupsPage />} />
                  <Route path="jobs" element={<AdminJobsPage />} />
                  <Route path="feedback" element={<FeedbackPage />} />
                  <Route path="agent" element={<MovedTo path="/settings/agentic/agents" />} />
                </Route>
                <Route path="settings" element={<SettingsLayout />}>
                  <Route index element={<GeneralPage />} />
                  <Route path="general" element={<GeneralPage />} />
                  {/* labels belong to a project now: each project's settings has them */}
                  <Route path="labels" element={<Navigate to="/tasks" replace />} />
                  <Route path="emoji" element={<EmojiPage />} />
                  <Route path="stickers" element={<StickersPage />} />
                  <Route path="github" element={<GithubPage />} />
                  <Route path="agentic" element={<AgenticTabs />}>
                    <Route index element={<AgenticYou />} />
                    <Route path="workspace" element={<AgenticWorkspace />} />
                    <Route path="agents" element={<AgenticAgents />} />
                    <Route path="routines" element={<AgenticRoutines />} />
                    {/* the Admin tab's default model is on the Agents tab now */}
                    <Route path="admin" element={<MovedTo path="/settings/agentic/agents" />} />
                  </Route>
                  <Route path="billing" element={<BillingPage />} />
                  <Route path="export" element={<ExportPage />} />
                  <Route path="danger-zone" element={<DangerZonePage />} />
                  <Route path="members" element={<MembersPage />} />
                  <Route path="agent/*" element={<MovedTo path="/settings/agentic/workspace" />} />
                  <Route path="agent-tools" element={<MovedTo path="/settings/agentic/workspace" />} />
                  <Route path="invitations" element={<Navigate to="/settings/members" replace />} />
                  {/* these moved to the account settings; old links and bookmarks still work */}
                  <Route path="sessions" element={<MovedTo path="/profile/sessions" />} />
                  <Route path="shortcuts" element={<MovedTo path="/profile/shortcuts" />} />
                  <Route path="notifications" element={<MovedTo path="/profile/notifications" />} />
                  <Route path="api-tokens" element={<ApiTokensPage />} />
                  <Route path="file-storage" element={<FileStoragePage />} />
                  <Route path="apps/*" element={<AppsSettingsPage />} />
                </Route>
                <Route path="*" element={<Navigate to="/tasks" replace />} />
              </Route>
            </Route>
          </Route>
        </Route>
      </Routes>
      </Suspense>
    </>
  )
}
