import { useQuery } from '@tanstack/react-query'
import { router, useLocalSearchParams } from 'expo-router'
import { useMemo } from 'react'
import { RefreshControl, ScrollView, View } from 'react-native'
import { SafeAreaView } from 'react-native-safe-area-context'

import { type DocBlock, docBlocks, pageTitle } from '../../../data/docs'
import { keys } from '../../../data/keys'
import * as api from '../../../data/orbit'
import { useOrbit } from '../../../orbit-context'
import { alpha, fonts, type, useTheme } from '../../../theme'
import { Button, ErrorView, Icon, IconButton, LoadingView, PaneHeader, Text } from '../../../ui'
import { useOpenOnWeb } from '../../../web'

const HEADING_SIZE: Record<number, number> = { 1: 24, 2: 20, 3: 17 }

/** A page, read-only: its title and its blocks as text. Editing stays on the web. */
export default function DocScreen() {
  const { id } = useLocalSearchParams<{ id: string }>()
  const { client, workspaceId } = useOrbit()
  const { colors } = useTheme()
  const openOnWeb = useOpenOnWeb()
  const page = useQuery({ queryKey: keys.page(workspaceId, id), queryFn: () => api.page(client, workspaceId, id) })
  const blocks = useMemo(() => (page.data ? docBlocks(page.data.content) : []), [page.data])
  const muted = alpha(colors.mutedForeground, 0.7)

  return (
    <SafeAreaView edges={['top', 'left', 'right']} style={{ flex: 1, backgroundColor: colors.background }}>
      <PaneHeader style={{ paddingHorizontal: 12 }}>
        <IconButton icon="back" label="Back to docs" color={muted} onPress={() => (router.canGoBack() ? router.back() : router.navigate('/docs'))} />
        <Text numberOfLines={1} style={[type(13, 'medium'), { flex: 1 }]}>
          {page.data ? pageTitle(page.data) : ''}
        </Text>
        <IconButton icon="comment" label="Comments" color={muted} onPress={() => openOnWeb(`/docs/${id}`)} />
        <IconButton icon="star" label="Favorite" color={muted} onPress={() => openOnWeb(`/docs/${id}`)} />
        <IconButton icon="more" label="Page options" color={muted} onPress={() => openOnWeb(`/docs/${id}`)} />
      </PaneHeader>
      {!page.data ? (
        page.isError ? (
          <ErrorView error={page.error} onRetry={() => void page.refetch()} />
        ) : (
          <LoadingView />
        )
      ) : (
        <ScrollView
          contentContainerStyle={{ paddingHorizontal: 12, paddingTop: 24, paddingBottom: 32 }}
          refreshControl={<RefreshControl refreshing={page.isRefetching} onRefresh={() => void page.refetch()} tintColor={colors.mutedForeground} />}
        >
          {page.data.icon ? <Text style={{ fontSize: 40, lineHeight: 48, marginBottom: 8 }}>{page.data.icon}</Text> : null}
          {!page.data.icon || !page.data.cover_url ? (
            <View style={{ flexDirection: 'row', gap: 8, marginBottom: 10 }}>
              {page.data.icon ? null : <Button label="😀 Add icon" variant="ghost" size="sm" muted onPress={() => openOnWeb(`/docs/${id}`)} />}
              {page.data.cover_url ? null : <Button label="🖼️ Add cover" variant="ghost" size="sm" muted onPress={() => openOnWeb(`/docs/${id}`)} />}
            </View>
          ) : null}
          <Text selectable style={[type(16, 'bold'), { lineHeight: 20, marginBottom: 17 }]}>
            {pageTitle(page.data)}
          </Text>
          <View style={{ gap: 6 }}>
            {blocks.map((block) => (
              <Block key={block.id} block={block} />
            ))}
          </View>
        </ScrollView>
      )}
    </SafeAreaView>
  )
}

function Block({ block }: { block: DocBlock }) {
  const { colors } = useTheme()
  const indent = { marginLeft: block.depth * 24 }
  const body = [type(16), { lineHeight: 24, flex: 1 }]
  switch (block.kind) {
    case 'heading': {
      const size = HEADING_SIZE[block.level ?? 1] ?? 17
      return (
        <Text selectable style={[type(size, 'semibold'), { lineHeight: Math.round(size * 1.3), marginTop: 8 }, indent]}>
          {block.text}
        </Text>
      )
    }
    case 'bullet':
    case 'numbered':
      return (
        <View style={[{ flexDirection: 'row', gap: 8 }, indent]}>
          <Text style={[type(16), { lineHeight: 24, minWidth: 16, color: colors.mutedForeground }]}>{block.kind === 'bullet' ? '•' : `${block.number}.`}</Text>
          <Text selectable style={body}>
            {block.text}
          </Text>
        </View>
      )
    case 'check':
      return (
        <View style={[{ flexDirection: 'row', alignItems: 'flex-start', gap: 8 }, indent]}>
          <View
            accessibilityLabel={block.checked ? 'Done' : 'Not done'}
            style={{ width: 16, height: 16, marginTop: 4, borderRadius: 4, borderWidth: 1, borderColor: block.checked ? colors.primary : colors.input, backgroundColor: block.checked ? colors.primary : 'transparent', alignItems: 'center', justifyContent: 'center' }}
          >
            {block.checked ? <Icon name="check" size={12} color={colors.primaryForeground} /> : null}
          </View>
          <Text selectable style={[body, block.checked && { color: colors.mutedForeground, textDecorationLine: 'line-through' }]}>
            {block.text}
          </Text>
        </View>
      )
    case 'quote':
      return (
        <View style={[{ borderLeftWidth: 3, borderLeftColor: colors.border, paddingLeft: 12 }, indent]}>
          <Text selectable style={body}>
            {block.text}
          </Text>
        </View>
      )
    case 'code':
      return (
        <View style={[{ borderRadius: 8, backgroundColor: colors.muted, padding: 12 }, indent]}>
          <Text selectable style={{ fontFamily: fonts.mono, fontSize: 13, lineHeight: 19, color: colors.foreground }}>
            {block.text}
          </Text>
        </View>
      )
    case 'divider':
      return <View style={[{ height: 1, marginVertical: 11, backgroundColor: colors.border }, indent]} />
    case 'other':
      return (
        <Text selectable style={[type(14), { lineHeight: 24, color: colors.mutedForeground }, indent]}>
          {block.text}
        </Text>
      )
    default:
      return (
        <Text selectable style={[...body, indent]}>
          {block.text || ' '}
        </Text>
      )
  }
}
