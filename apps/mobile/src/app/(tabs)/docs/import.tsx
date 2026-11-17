import { useQuery, useQueryClient } from '@tanstack/react-query'
import { router, useLocalSearchParams } from 'expo-router'
import { useEffect, useState } from 'react'
import { Linking, Pressable, ScrollView, View } from 'react-native'

import { keys } from '../../../data/keys'
import {
  cancelImport,
  coveredByParent,
  importDestination,
  importRows,
  importSelection,
  isActiveImport,
  newNotionImport,
  NOTION_POLL_MS,
  notionImport,
  notionImports,
  startImport,
  STATUS_LABEL,
} from '../../../data/notion-import'
import { toggleId } from '../../../data/tasks'
import { useOrbit } from '../../../orbit-context'
import { useTeamspaces } from '../../../queries'
import { errorMessage } from '../../../session-context'
import { alpha, useTheme } from '../../../theme'
import { Button, Icon, IconButton, Input, KeyboardView, PaneHeader, SafeAreaView, Spinner, Text } from '../../../ui'

const NOTION_TOKENS = 'https://www.notion.so/profile/integrations'

/**
 * The web's Import from Notion. A Notion token lets Orbit read the pages it can see; the user checks pages (each with
 * everything under it) and a space, and Orbit imports them in the background. `?id=` follows one import.
 */
export default function NotionImportScreen() {
  const { id } = useLocalSearchParams<{ id?: string }>()
  const { colors } = useTheme()
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  const teamspaces = [...(useTeamspaces().data ?? [])].sort((a, b) => a.position - b.position)
  const base = [...keys.pages(workspaceId), 'notion-imports'] as const
  const list = useQuery({ queryKey: base, queryFn: () => notionImports(client, workspaceId), enabled: !id })
  const current = useQuery({
    queryKey: [...base, id ?? ''],
    queryFn: () => notionImport(client, workspaceId, id ?? ''),
    enabled: Boolean(id),
    // only while Orbit works on it: not while the user chooses, and never after an error
    refetchInterval: (query) => (['scanning', 'queued', 'importing'].includes(query.state.data?.status ?? '') ? NOTION_POLL_MS : false),
  })
  const [token, setToken] = useState('')
  const [checked, setChecked] = useState<string[]>([])
  const [search, setSearch] = useState('')
  const [space, setSpace] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const muted = alpha(colors.mutedForeground, 0.7)
  const item = current.data
  useEffect(() => {
    if (space === null && teamspaces[0]) setSpace(`teamspace:${teamspaces[0].id}`)
  }, [space, teamspaces])
  // the import ended: the new pages are in the tree
  useEffect(() => {
    if (item?.status === 'completed') void queryClient.invalidateQueries({ queryKey: keys.pages(workspaceId), exact: true })
  }, [item?.status, queryClient, workspaceId])

  const act = async (work: () => Promise<unknown>) => {
    setBusy(true)
    setError(null)
    try {
      await work()
    } catch (reason) {
      setError(errorMessage(reason))
    } finally {
      setBusy(false)
    }
  }
  const create = () =>
    act(async () => {
      const created = await newNotionImport(client, workspaceId, token.trim())
      setToken('')
      router.setParams({ id: created.id })
    })
  const nodes = item?.tree?.nodes ?? []
  const selection = importSelection(nodes, new Set(checked))
  const start = () =>
    act(async () => {
      if (!item || !selection || !space) return
      queryClient.setQueryData([...base, item.id], await startImport(client, workspaceId, item.id, selection, importDestination(space)))
      void current.refetch()
    })
  const cancel = () =>
    act(async () => {
      if (!item) return
      await cancelImport(client, workspaceId, item.id)
      await current.refetch()
    })

  return (
    <SafeAreaView edges={['top', 'left', 'right']} className="flex-1 bg-background">
      <PaneHeader className="px-3">
        <IconButton icon="back" label="Back to docs" color={muted} onPress={() => (router.canGoBack() ? router.back() : router.navigate('/docs'))} />
        <Icon name="import" size={15} color={colors.mutedForeground} />
        <Text className="font-noto-semibold text-[13px] flex-1">Import from Notion</Text>
        {item && isActiveImport(item.status) ? <Button label="Cancel import" size="sm" variant="ghost" disabled={busy} onPress={() => void cancel()} /> : null}
      </PaneHeader>
      <KeyboardView className="flex-1">
        <ScrollView contentContainerClassName="p-4 gap-3 pb-10" keyboardShouldPersistTaps="handled">
          {error ? <Text className="text-[13px] text-destructive">{error}</Text> : null}
          {!id ? (
            <>
              <Text className="text-[14px] leading-[20px]">
                Make a personal access token in Notion (Settings → Connections), or share the pages with a connection, and paste its token here. Orbit reads the pages it can see; you choose what to import.
              </Text>
              <Pressable accessibilityRole="link" onPress={() => void Linking.openURL(NOTION_TOKENS)}>
                <Text className="text-[13px] text-primary">Open Notion connections</Text>
              </Pressable>
              <Input label="Notion token" value={token} onChangeText={setToken} secureTextEntry autoCapitalize="none" autoCorrect={false} placeholder="ntn_…" />
              <Button label="Read my Notion pages" disabled={busy || token.trim().length < 10} loading={busy} onPress={() => void create()} />
              {(list.data ?? []).length > 0 ? (
                <View className="gap-1 pt-3">
                  <Text className="font-noto-medium text-[12px] text-muted-foreground">Earlier imports</Text>
                  {(list.data ?? []).map((entry) => (
                    <Pressable key={entry.id} accessibilityRole="button" onPress={() => router.setParams({ id: entry.id })} className="min-h-12 flex-row items-center gap-3 py-1.5 border-b border-border active:bg-muted">
                      <View className="flex-1">
                        <Text className="font-noto-medium text-[13px]">{entry.notion_workspace_name ?? 'Notion'}</Text>
                        <Text className="text-[12px] text-muted-foreground">
                          {STATUS_LABEL[entry.status]} · {new Date(entry.created_at).toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}
                        </Text>
                      </View>
                      <Icon name="chevronRight" size={14} color={muted} />
                    </Pressable>
                  ))}
                </View>
              ) : null}
            </>
          ) : !item ? (
            current.error ? <Text className="text-[13px] text-destructive">{errorMessage(current.error)}</Text> : <Spinner />
          ) : item.status === 'scanning' ? (
            <View className="items-center gap-3 py-10">
              <Spinner />
              <Text className="text-[14px] text-muted-foreground">Reading your Notion pages…</Text>
            </View>
          ) : item.status === 'ready' ? (
            <>
              <Text className="text-[14px]">
                {nodes.length} {nodes.length === 1 ? 'page' : 'pages'} in {item.notion_workspace_name ?? 'Notion'}. A checked page comes with everything under it.
              </Text>
              {item.tree?.truncated ? <Text className="text-[12px] text-muted-foreground">Only the first 5,000 pages are listed; import the rest in a later import.</Text> : null}
              <Input value={search} onChangeText={setSearch} placeholder="Search pages" autoCorrect={false} />
              <View className="rounded-lg border border-border">
                {importRows(nodes, search).map((node, index) => {
                  const covered = coveredByParent(nodes, new Set(checked), node.notion_id)
                  const on = covered || checked.includes(node.notion_id)
                  return (
                    <Pressable
                      key={node.notion_id}
                      accessibilityRole="checkbox"
                      accessibilityState={{ checked: on, disabled: covered }}
                      disabled={covered}
                      onPress={() => setChecked(toggleId(checked, node.notion_id))}
                      className={`min-h-11 flex-row items-center gap-3 px-3 ${index === 0 ? '' : 'border-t border-border'} active:bg-muted`}
                    >
                      <View className={`size-5 rounded border items-center justify-center ${on ? 'bg-primary border-primary' : 'border-input'}`}>{on ? <Icon name="check" size={13} color={colors.primaryForeground} /> : null}</View>
                      <Text className="text-[15px]">{node.icon ?? (node.kind === 'database' ? '🗂️' : '📄')}</Text>
                      <Text numberOfLines={1} className="text-[14px] flex-1">
                        {node.title.trim() || 'Untitled'}
                      </Text>
                      {node.child_count > 0 ? <Text className="text-[12px] text-muted-foreground">+{node.child_count}</Text> : null}
                    </Pressable>
                  )
                })}
              </View>
              <Text className="font-noto-medium text-[13px] text-muted-foreground pt-2">Import into</Text>
              <View className="flex-row flex-wrap gap-1.5">
                {[...teamspaces.map((teamspace) => ({ key: `teamspace:${teamspace.id}`, name: teamspace.name })), { key: 'private', name: 'Private' }].map((option) => (
                  <Pressable
                    key={option.key}
                    accessibilityRole="radio"
                    accessibilityState={{ checked: space === option.key }}
                    onPress={() => setSpace(option.key)}
                    className={`h-8 px-3 rounded-full justify-center border ${space === option.key ? 'border-primary bg-primary/10' : 'border-border'}`}
                  >
                    <Text className="text-[13px]">{option.name}</Text>
                  </Pressable>
                ))}
              </View>
              <Button label={selection && 'all' in selection ? 'Import everything' : 'Import'} disabled={busy || !selection || !space} loading={busy} onPress={() => void start()} />
            </>
          ) : (
            <View className="gap-2 py-4">
              <View className="flex-row items-center gap-2">
                {isActiveImport(item.status) ? <Spinner /> : <Icon name={item.status === 'completed' ? 'checkCircle' : 'warning'} size={18} color={item.status === 'completed' ? colors.primary : colors.destructive} />}
                <Text className="font-noto-semibold text-[15px]">{STATUS_LABEL[item.status]}</Text>
              </View>
              {item.progress.total > 0 ? (
                <Text className="text-[13px] text-muted-foreground">
                  {item.progress.done} of {item.progress.total} pages{item.progress.failed > 0 ? `, ${item.progress.failed} failed` : ''}
                </Text>
              ) : null}
              {item.error ? <Text className="text-[13px] text-destructive">{item.error}</Text> : null}
              {item.report ? (
                <Text className="text-[13px] text-muted-foreground">
                  {item.report.files_imported} files imported{item.report.missing_files > 0 ? `, ${item.report.missing_files} files missing` : ''}
                  {item.report.previously_imported > 0 ? `, ${item.report.previously_imported} pages were imported before` : ''}.
                </Text>
              ) : null}
              {!isActiveImport(item.status) ? (
                <View className="flex-row gap-2 pt-2">
                  <Button label="Open Docs" onPress={() => router.navigate('/docs')} />
                  <Button label="New import" variant="outline" onPress={() => router.setParams({ id: undefined })} />
                </View>
              ) : null}
            </View>
          )}
        </ScrollView>
      </KeyboardView>
    </SafeAreaView>
  )
}
