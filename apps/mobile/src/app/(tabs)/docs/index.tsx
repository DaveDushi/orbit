import { router } from 'expo-router'
import { type ReactNode, useMemo, useState } from 'react'
import { Pressable, RefreshControl, ScrollView, View } from 'react-native'
import { SafeAreaView } from 'react-native-safe-area-context'

import { MenuButton } from '../../../components/Drawer'
import { docSpaces, pageTitle, type TreeRow } from '../../../data/docs'
import { usePages, useTeamspaces } from '../../../queries'
import { alpha, type, useTheme } from '../../../theme'
import { ErrorView, HeaderTitle, Icon, type IconName, IconButton, LoadingView, PaneHeader, Text } from '../../../ui'
import { useOpenOnWeb } from '../../../web'

/** The web's docs sidebar at phone size: Teamspaces with their page trees, Private, and Trash at the bottom. */
export default function DocsScreen() {
  const { colors } = useTheme()
  const pages = usePages()
  const teamspaces = useTeamspaces()
  const openOnWeb = useOpenOnWeb()
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(new Set())
  const spaces = useMemo(() => (pages.data && teamspaces.data ? docSpaces(pages.data, teamspaces.data, collapsed) : null), [collapsed, pages.data, teamspaces.data])
  const refresh = () => Promise.all([pages.refetch(), teamspaces.refetch()])
  const toggle = (key: string) =>
    setCollapsed((current) => {
      const next = new Set(current)
      if (!next.delete(key)) next.add(key)
      return next
    })
  const muted = alpha(colors.mutedForeground, 0.7)

  return (
    <SafeAreaView edges={['top', 'left', 'right']} style={{ flex: 1, backgroundColor: colors.background }}>
      <PaneHeader style={{ minHeight: 44, paddingVertical: 6 }}>
        <MenuButton />
        <View style={{ flex: 1 }}>
          <HeaderTitle>Documents</HeaderTitle>
        </View>
        <IconButton icon="more" label="Docs options" onPress={() => openOnWeb('/docs')} />
        <IconButton icon="plus" label="New page" size={32} variant="primary" onPress={() => openOnWeb('/docs')} />
      </PaneHeader>
      {!spaces ? (
        pages.error || teamspaces.error ? (
          <ErrorView error={pages.error ?? teamspaces.error} onRetry={() => void refresh()} />
        ) : (
          <LoadingView />
        )
      ) : (
        <ScrollView contentContainerStyle={{ paddingHorizontal: 8, paddingTop: 8, paddingBottom: 16, gap: 14 }} refreshControl={<RefreshControl refreshing={pages.isRefetching} onRefresh={() => void refresh()} tintColor={colors.mutedForeground} />}>
          <View>
            <GroupHeader label="Teamspaces" />
            {spaces.teamspaces.map((space) => {
              const open = !collapsed.has(space.key)
              return (
                <View key={space.key}>
                  <TreeItem
                    label={space.name}
                    leading={<Icon name="people" size={15} color={muted} />}
                    paddingLeft={8}
                    onPress={() => toggle(space.key)}
                    actions={
                      <>
                        <IconButton icon="more" label={`${space.name} options`} size={22} iconSize={13} color={muted} onPress={() => openOnWeb('/docs')} />
                        <IconButton icon="plus" label={`New page in ${space.name}`} size={22} iconSize={13} color={muted} onPress={() => openOnWeb('/docs')} />
                      </>
                    }
                  />
                  {open ? <PageRows rows={space.rows} collapsed={collapsed} onToggle={toggle} /> : null}
                </View>
              )
            })}
          </View>
          <View>
            <GroupHeader label="Private" icon="lock" />
            {spaces.private.rows.length === 0 ? (
              <View style={{ height: 32, justifyContent: 'center', paddingLeft: 40 }}>
                <Text style={[type(13), { color: alpha(colors.mutedForeground, 0.6) }]}>No pages inside</Text>
              </View>
            ) : (
              <PageRows rows={spaces.private.rows} collapsed={collapsed} onToggle={toggle} />
            )}
          </View>
        </ScrollView>
      )}
      <Pressable
        accessibilityRole="link"
        accessibilityLabel="Trash, opens Orbit on the web"
        onPress={() => openOnWeb('/docs/trash')}
        style={({ pressed }) => ({ height: 49, flexDirection: 'row', alignItems: 'center', gap: 10, paddingHorizontal: 16, borderTopWidth: 1, borderTopColor: colors.border, backgroundColor: pressed ? colors.muted : 'transparent' })}
      >
        <Icon name="trash" size={15} color={colors.mutedForeground} />
        <Text style={[type(13, 'medium'), { color: colors.foreground }]}>Trash</Text>
      </Pressable>
    </SafeAreaView>
  )
}

function GroupHeader({ label, icon }: { label: string; icon?: IconName }) {
  const { colors } = useTheme()
  const openOnWeb = useOpenOnWeb()
  return (
    <View style={{ height: 28, flexDirection: 'row', alignItems: 'center', gap: 6, paddingLeft: 8, paddingRight: 8 }}>
      {icon ? <Icon name={icon} size={14} color={colors.mutedForeground} /> : null}
      <Text style={[type(12, 'medium'), { lineHeight: 16, flex: 1, color: colors.mutedForeground }]}>{label}</Text>
      <IconButton icon="plus" label={`New ${label === 'Private' ? 'private page' : 'teamspace'}`} size={22} iconSize={13} color={alpha(colors.mutedForeground, 0.7)} onPress={() => openOnWeb('/docs')} />
    </View>
  )
}

function PageRows({ rows, collapsed, onToggle }: { rows: TreeRow[]; collapsed: ReadonlySet<string>; onToggle: (key: string) => void }) {
  const { colors } = useTheme()
  const openOnWeb = useOpenOnWeb()
  const muted = alpha(colors.mutedForeground, 0.7)
  return rows.map((row) => (
    <TreeItem
      key={row.page.id}
      label={pageTitle(row.page)}
      paddingLeft={28 + row.depth * 18}
      chevron={row.hasChildren ? { open: !collapsed.has(row.page.id), onPress: () => onToggle(row.page.id) } : null}
      leading={row.page.icon ? <Text style={type(14)}>{row.page.icon}</Text> : <Icon name="docs" size={15} color={muted} />}
      onPress={() => router.push(`/docs/${row.page.id}`)}
      actions={
        <>
          <IconButton icon="plus" label={`New page in ${pageTitle(row.page)}`} size={22} iconSize={13} color={muted} onPress={() => openOnWeb(`/docs/${row.page.id}`)} />
          <IconButton icon="more" label={`${pageTitle(row.page)} options`} size={22} iconSize={13} color={muted} onPress={() => openOnWeb(`/docs/${row.page.id}`)} />
        </>
      }
    />
  ))
}

/** One 32px tree row: an optional chevron slot, the icon, the name, and the row actions. */
function TreeItem({
  label,
  leading,
  paddingLeft,
  chevron,
  onPress,
  actions,
}: {
  label: string
  leading: ReactNode
  paddingLeft: number
  chevron?: { open: boolean; onPress: () => void } | null
  onPress: () => void
  actions: ReactNode
}) {
  const { colors } = useTheme()
  return (
    <View style={{ height: 32, marginTop: 1, flexDirection: 'row', alignItems: 'center' }}>
      <Pressable
        accessibilityRole="link"
        accessibilityLabel={label}
        onPress={onPress}
        style={({ pressed }) => ({ flex: 1, height: 32, flexDirection: 'row', alignItems: 'center', paddingLeft, borderRadius: 8, backgroundColor: pressed ? colors.muted : 'transparent' })}
      >
        {chevron !== undefined ? (
          <Pressable accessibilityRole="button" accessibilityLabel={chevron?.open ? `Collapse ${label}` : `Expand ${label}`} disabled={!chevron} onPress={chevron?.onPress} style={{ width: 16, marginRight: 6, alignItems: 'center' }}>
            {chevron ? <Icon name={chevron.open ? 'chevronDown' : 'chevronRight'} size={12} color={colors.mutedForeground} /> : null}
          </Pressable>
        ) : null}
        <View style={{ width: 15, alignItems: 'center', marginRight: chevron !== undefined ? 7 : 8 }}>{leading}</View>
        <Text numberOfLines={1} style={[type(13, 'medium'), { flex: 1 }]}>
          {label}
        </Text>
      </Pressable>
      <View style={{ flexDirection: 'row', gap: 2, paddingLeft: 4, paddingRight: 6 }}>{actions}</View>
    </View>
  )
}
