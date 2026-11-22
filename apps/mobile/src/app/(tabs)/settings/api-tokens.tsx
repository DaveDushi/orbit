import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { Share, View } from 'react-native'

import { CheckRow, SelectField } from '../../../components/settings/Fields'
import { ConfirmDialog, type ConfirmRequest } from '../../../components/settings/Dialog'
import { Badge, FieldLabel, Note, OptionHint, OptionText, RowIcon, RowText, SettingsCard, SettingsRow, SettingsShell, WORKSPACE_NAV } from '../../../components/settings/Shell'
import * as tokens from '../../../data/api-tokens'
import { relativeTime, shortDate } from '../../../data/format'
import { useOrbit } from '../../../orbit-context'
import { useProjects } from '../../../queries'
import { errorMessage } from '../../../session-context'
import { Button, Input, Text } from '../../../ui'

const LEVELS: readonly { value: tokens.AccessLevel; label: string }[] = [
  { value: 'none', label: 'No access' },
  { value: 'read', label: 'Read' },
  { value: 'write', label: 'Write' },
  { value: 'manage', label: 'Manage' },
]

const EMPTY: tokens.TokenForm = { name: '', scopes: ['tasks:read'], projectIds: [], allProjects: false, serviceAccount: false, expiration: 'never' }

/**
 * The web's Settings → API tokens: create a personal or service-account token with its access, and revoke tokens.
 * Owners and admins only (`api_tokens.manage`). The secret shows once; the app has no clipboard, so it is selectable
 * and has Share.
 */
export default function ApiTokensScreen() {
  const { client, workspace, workspaceId } = useOrbit()
  const canManage = workspace.permissions.includes('api_tokens.manage')
  const queryClient = useQueryClient()
  const key = ['api-tokens', workspaceId]
  const list = useQuery({ queryKey: key, queryFn: () => tokens.apiTokens(client, workspaceId), enabled: canManage })
  const projects = useProjects()
  const onSettled = () => void queryClient.invalidateQueries({ queryKey: key })
  const create = useMutation({ mutationFn: (form: tokens.TokenForm) => tokens.createToken(client, workspaceId, form), onSettled })
  const revoke = useMutation({ mutationFn: (id: string) => tokens.revokeToken(client, workspaceId, id), onSettled })
  const [form, setForm] = useState<tokens.TokenForm>(EMPTY)
  const [issued, setIssued] = useState<string | null>(null)
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null)
  const set = (fields: Partial<tokens.TokenForm>) => setForm((current) => ({ ...current, ...fields }))
  const projectNames = new Map((projects.data ?? []).map((project) => [project.id, project.name]))
  const activeProjects = (projects.data ?? []).filter((project) => !project.deleted_at)

  if (!canManage) {
    return (
      <SettingsShell title="Settings" nav={WORKSPACE_NAV}>
        <SettingsCard title="API tokens">
          <Note>Only workspace owners and administrators can manage API tokens.</Note>
        </SettingsCard>
      </SettingsShell>
    )
  }

  const submit = () => {
    if (!tokens.tokenFormReady(form) || create.isPending) return
    create.mutate(form, {
      onSuccess: (secret) => {
        setIssued(secret)
        set({ name: '' })
      },
    })
  }

  return (
    <SettingsShell title="Settings" nav={WORKSPACE_NAV} refreshing={list.isRefetching} onRefresh={() => void list.refetch()}>
      <SettingsCard title="Create API token" description="Create a personal token, or select a non-login service account for an integration. The secret is shown only once.">
        <View className="gap-4">
          <Input label="Name" required value={form.name} maxLength={100} placeholder="Discord bot" onChangeText={(name) => set({ name })} />
          <SelectField label="Expiration" value={form.expiration} options={tokens.EXPIRATION_OPTIONS} onChange={(value) => set({ expiration: value as tokens.Expiration })} />
          {/* a choice card: tinted while checked */}
          <View className={`gap-1 rounded-lg border px-3 py-2 ${form.serviceAccount ? 'border-primary bg-primary/5' : 'border-border'}`}>
            <CheckRow label="Use a service account" checked={form.serviceAccount} onChange={(checked) => set(checked ? { serviceAccount: true, scopes: tokens.serviceAccountScopes(form.scopes) } : { serviceAccount: false })} />
            <OptionHint>For bots, such as a Discord bot. Changes show as made by this account, not by you. Tokens with the same name share one account.</OptionHint>
          </View>
          <View className="gap-4">
            <FieldLabel>Access</FieldLabel>
            {tokens.tokenResources(form.serviceAccount).map((resource) => {
              const level = tokens.accessLevel(resource, form.scopes)
              return (
                <View key={resource.name} className="gap-2">
                  <OptionText title={resource.name}>
                    <OptionHint>{resource.hint}</OptionHint>
                  </OptionText>
                  <View accessibilityRole="radiogroup" accessibilityLabel={resource.name} className="flex-row flex-wrap gap-1.5">
                    {LEVELS.filter((option) => option.value !== 'manage' || resource.manage).map((option) => {
                      const available = tokens.levelAvailable(resource, option.value)
                      return (
                        <Button
                          key={option.value}
                          label={option.label}
                          size="sm"
                          variant={option.value === level ? 'primary' : 'outline'}
                          disabled={!available}
                          onPress={available ? () => set({ scopes: tokens.withAccessLevel(resource, form.scopes, option.value) }) : undefined}
                        />
                      )
                    })}
                  </View>
                  {resource.read === 'tasks:read' && tokens.hasTaskScope(form.scopes) ? (
                    <View className="border-l border-border pl-4">
                      <CheckRow label="All projects, including future ones" checked={form.allProjects} onChange={(allProjects) => set({ allProjects })} />
                      {form.allProjects
                        ? null
                        : activeProjects.map((project) => (
                            <CheckRow
                              key={project.id}
                              label={project.name}
                              checked={form.projectIds.includes(project.id)}
                              onChange={(checked) => set({ projectIds: checked ? [...form.projectIds, project.id] : form.projectIds.filter((id) => id !== project.id) })}
                            />
                          ))}
                      {!form.allProjects && projects.isPending ? <Note loading>Loading projects…</Note> : null}
                      {!form.allProjects && projects.data && activeProjects.length === 0 ? <Note>This workspace has no active projects.</Note> : null}
                    </View>
                  ) : null}
                </View>
              )
            })}
          </View>
          <View className="items-start gap-2">
            <Button label="Create token" loading={create.isPending} disabled={!tokens.tokenFormReady(form)} onPress={submit} />
            {create.isError ? <Note error>The API token could not be created. {errorMessage(create.error)}</Note> : null}
          </View>
          {issued ? (
            <View accessibilityRole="alert" className="gap-2.5 rounded-lg border border-primary bg-primary/10 p-3">
              <Text className="font-noto-semibold text-[12px]">Copy this token now. You cannot see it again.</Text>
              <Text selectable className="font-mono text-[12px]">{issued}</Text>
              <View className="flex-row">
                <Button label="Share" icon="send" variant="outline" size="sm" onPress={() => void Share.share({ message: issued })} />
              </View>
            </View>
          ) : null}
        </View>
      </SettingsCard>

      <SettingsCard title="Active API tokens" description="Revoke tokens that are no longer in use." flush>
        {list.isPending ? <View className="p-4"><Note loading>Loading API tokens…</Note></View> : null}
        {list.isError ? <View className="p-4"><Note error>API tokens could not be loaded.</Note></View> : null}
        {list.data?.length === 0 ? <View className="p-4"><Note>No active API tokens.</Note></View> : null}
        {list.data?.map((token, index) => {
          const expired = tokens.isExpired(token.expires_at)
          const projectText = token.all_projects ? 'All projects · ' : token.project_ids.length ? `${token.project_ids.map((id) => projectNames.get(id) ?? 'Unavailable project').join(', ')} · ` : ''
          const expiry = token.expires_at ? `${expired ? 'expired' : 'expires'} ${shortDate(token.expires_at)}` : 'never expires'
          return (
            <SettingsRow key={token.id} first={index === 0}>
              <RowIcon icon="key" />
              <View className="flex-1 min-w-0 gap-1.5">
                <RowText
                  title={token.name}
                  badges={expired ? <Badge label="Expired" tone="destructive" /> : null}
                  subtitle={`${token.service_account_name ? `Service account: ${token.service_account_name}` : 'Personal token'} · ${token.token_prefix}… · ${projectText}${expiry} · created ${relativeTime(token.created_at)}`}
                />
                <View className="flex-row flex-wrap gap-1">
                  {token.scopes.map((scope) => <Badge key={scope} label={scope} tone="secondary" />)}
                </View>
              </View>
              <Button
                label="Revoke"
                variant="ghost"
                size="sm"
                disabled={revoke.isPending}
                onPress={() => setConfirm({ title: `Revoke ${token.name}?`, description: 'The integration will lose access immediately.', confirmLabel: 'Revoke', danger: true, onConfirm: () => revoke.mutate(token.id) })}
              />
            </SettingsRow>
          )
        })}
        {revoke.isError ? <View className="p-4"><Note error>The token could not be revoked. {errorMessage(revoke.error)}</Note></View> : null}
      </SettingsCard>
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </SettingsShell>
  )
}
