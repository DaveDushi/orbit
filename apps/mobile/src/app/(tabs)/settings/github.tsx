import { useQuery, useQueryClient } from '@tanstack/react-query'
import * as WebBrowser from 'expo-web-browser'
import { githubWorkspaceSettings } from '@orbit/api/generated/sdk.gen'
import { View } from 'react-native'

import { Note, RowIcon, RowText, SettingsCard, SettingsRow, SettingsShell, WORKSPACE_NAV } from '../../../components/settings/Shell'
import { useOrbit } from '../../../orbit-context'
import { Button } from '../../../ui'

/**
 * The web's Settings → GitHub: the workspace's GitHub App and its installed repositories. Registering the App posts a
 * form to GitHub from the browser, so Register opens the web page in an in-app browser; Install opens GitHub. Projects
 * pick their repository in their own settings.
 */
export default function GithubScreen() {
  const { client, session, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  const key = ['github-workspace', workspaceId]
  const query = useQuery({ queryKey: key, queryFn: async () => (await githubWorkspaceSettings({ client, path: { workspace_id: workspaceId }, throwOnError: true })).data })
  const open = async (url: string) => {
    await WebBrowser.openBrowserAsync(url, { presentationStyle: WebBrowser.WebBrowserPresentationStyle.PAGE_SHEET })
    void queryClient.invalidateQueries({ queryKey: key })
  }
  const settings = query.data
  return (
    <SettingsShell title="Settings" nav={WORKSPACE_NAV} refreshing={query.isRefetching} onRefresh={() => void query.refetch()}>
      {query.isPending ? <SettingsCard title="GitHub"><Note loading>Loading GitHub settings…</Note></SettingsCard> : null}
      {query.isError ? <SettingsCard title="GitHub"><Note error>GitHub settings are not available.</Note></SettingsCard> : null}
      {settings ? (
        <SettingsCard title="GitHub" description="Manage the GitHub App for this workspace. Connect repositories to projects in each project's settings." flush={!!settings.app_slug}>
          {!settings.app_slug ? (
            <View className="gap-3 items-start">
              {settings.can_manage ? (
                <>
                  <Note>Register a GitHub App from Orbit in the web app. GitHub sends you back to this workspace there.</Note>
                  {!settings.key_configured ? <Note error>Orbit is not ready to register a GitHub App yet. Ask the Orbit administrator to set the app key.</Note> : null}
                  <Button label="Register GitHub App" icon="arrowUpRightSquare" disabled={!settings.key_configured} onPress={() => void open(`${session.server}/settings/github?workspace=${encodeURIComponent(workspaceId)}`)} />
                </>
              ) : (
                <Note>Ask a workspace owner or admin to register the App.</Note>
              )}
            </View>
          ) : (
            <>
              <SettingsRow first>
                <RowIcon icon="link2" />
                <RowText title={`App: ${settings.app_slug}`} subtitle="After installation, open a project's settings to select a repository and issue label." />
              </SettingsRow>
              {settings.can_manage && settings.install_url ? (
                <View className="flex-row border-t border-border p-4">
                  <Button label="Install or change repositories" variant="outline" icon="arrowUpRightSquare" onPress={() => void open(settings.install_url!)} />
                </View>
              ) : null}
              {settings.repositories.length === 0 ? <View className="border-t border-border p-4"><Note>No repositories installed yet.</Note></View> : null}
              {settings.repositories.map((item) => (
                <SettingsRow key={`${item.installation_id}:${item.repository}`}>
                  <RowIcon icon="code" />
                  <RowText title={item.repository} />
                </SettingsRow>
              ))}
            </>
          )}
        </SettingsCard>
      ) : null}
    </SettingsShell>
  )
}
