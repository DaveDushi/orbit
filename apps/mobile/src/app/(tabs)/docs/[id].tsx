import { useQuery } from '@tanstack/react-query'
import { router, useLocalSearchParams } from 'expo-router'
import { useEffect, useMemo, useState } from 'react'
import { Image, KeyboardAvoidingView, Platform, RefreshControl, ScrollView, TextInput, View } from 'react-native'
import { SafeAreaView } from 'react-native-safe-area-context'

import { CommentsSheet } from '../../../components/docs/Comments'
import { IconDialog } from '../../../components/docs/IconDialog'
import { ActionSheet, ConfirmDialog, type ConfirmRequest, PromptDialog, type PromptRequest, type SheetAction } from '../../../components/settings/Dialog'
import { absoluteUrl, type DocBlock, docBlocks, pageTitle, trashPrompt, validCoverUrl, visibleThreads } from '../../../data/docs'
import { keys } from '../../../data/keys'
import * as api from '../../../data/orbit'
import { pickImage } from '../../../hooks/account'
import { useAddParagraphs, useFavorites, usePageThreads, useSavePage, useToggleFavorite, useTrashPage, useUploadCover } from '../../../hooks/docs'
import { useOrbit } from '../../../orbit-context'
import { usePages } from '../../../queries'
import { errorMessage } from '../../../session-context'
import { alpha, fonts, type, useTheme } from '../../../theme'
import { Button, CountBadge, ErrorView, Icon, IconButton, LoadingView, PaneHeader, Text } from '../../../ui'

const HEADING_SIZE: Record<number, number> = { 1: 24, 2: 20, 3: 17 }

/**
 * A page: its cover, icon and title (editable), its blocks as text, and a field that adds plain paragraphs at the end.
 * Editing the blocks themselves needs the web's BlockNote editor, which has no React Native version.
 */
export default function DocScreen() {
  const { id } = useLocalSearchParams<{ id: string }>()
  const { client, workspaceId, session } = useOrbit()
  const { colors } = useTheme()
  const page = useQuery({ queryKey: keys.page(workspaceId, id), queryFn: () => api.page(client, workspaceId, id) })
  const pages = usePages()
  const favorites = useFavorites()
  const threads = usePageThreads(id)
  const save = useSavePage(id)
  const addParagraphs = useAddParagraphs(id)
  const uploadCover = useUploadCover(id)
  const toggleFavorite = useToggleFavorite()
  const trash = useTrashPage()
  const blocks = useMemo(() => (page.data ? docBlocks(page.data.content) : []), [page.data])
  const [title, setTitle] = useState<string | null>(null)
  const [text, setText] = useState('')
  const [menu, setMenu] = useState<SheetAction[] | null>(null)
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null)
  const [prompt, setPrompt] = useState<PromptRequest | null>(null)
  const [choosingIcon, setChoosingIcon] = useState(false)
  const [commentsOpen, setCommentsOpen] = useState(false)
  const [notice, setNotice] = useState<string | null>(null)
  const muted = alpha(colors.mutedForeground, 0.7)
  const favorite = (favorites.data ?? []).includes(id)
  const openThreads = visibleThreads(threads.data ?? []).filter((thread) => !thread.resolved).length
  const locked = Boolean(page.data?.locked_at)
  const failed = (what: string) => (error: unknown) => setNotice(`${what} ${errorMessage(error)}`)

  // a new page from another device or a save: show the server's title unless one is being typed
  useEffect(() => setTitle(null), [page.data?.title])

  const saveTitle = () => {
    // Return and the blur that follows both land here: one save is enough
    if (save.isPending) return
    if (title === null || !page.data || title.trim() === page.data.title) return setTitle(null)
    save.mutate({ title: title.trim() }, { onSuccess: () => setTitle(null), onError: failed('Could not rename the page.') })
  }
  const saveIcon = (icon: string | null) => save.mutate({ icon }, { onError: failed('Could not change the icon.') })
  const coverFromLibrary = () =>
    pickImage({ square: false }).then(
      async (file) => {
        if (!file) return
        const url = await uploadCover.mutateAsync(file)
        await save.mutateAsync({ cover_url: url })
      },
      failed('Could not read the image.'),
    ).catch(failed('Could not upload the cover.'))
  const coverActions = (): SheetAction[] => [
    { label: 'Upload an image', icon: 'import', onPress: () => void coverFromLibrary() },
    {
      label: 'Paste an image URL',
      icon: 'link2',
      onPress: () =>
        setPrompt({
          title: 'Cover image',
          label: 'Image URL',
          placeholder: 'https://…',
          keyboard: 'url',
          confirmLabel: 'Use image',
          onSubmit: async (value) => {
            const url = validCoverUrl(value)
            if (!url) throw new Error('Enter an http(s) image URL.')
            await save.mutateAsync({ cover_url: url })
          },
        }),
    },
    ...(page.data?.cover_url ? [{ label: 'Remove cover', icon: 'trash' as const, destructive: true, onPress: () => save.mutate({ cover_url: null }, { onError: failed('Could not remove the cover.') }) }] : []),
  ]
  const pageMenu = (): SheetAction[] =>
    page.data
      ? [
          { label: favorite ? 'Remove from favorites' : 'Add to favorites', icon: 'star', onPress: () => toggleFavorite.mutate({ pageId: id, favorite: !favorite }) },
          { label: page.data.icon ? 'Change icon' : 'Add icon', icon: 'emojiHappy', disabled: locked, onPress: () => setChoosingIcon(true) },
          { label: page.data.cover_url ? 'Change cover' : 'Add cover', icon: 'sticker', disabled: locked, onPress: () => setMenu(coverActions()) },
          {
            label: 'Move to trash',
            icon: 'trash',
            destructive: true,
            onPress: () =>
              page.data &&
              setConfirm({
                ...trashPrompt(pages.data ?? [], page.data),
                confirmLabel: 'Move to trash',
                danger: true,
                onConfirm: () => page.data && trash.mutate(page.data, { onSuccess: () => (router.canGoBack() ? router.back() : router.navigate('/docs')), onError: failed('Could not move the page to the trash.') }),
              }),
          },
        ]
      : []

  return (
    <SafeAreaView edges={['top', 'left', 'right']} style={{ flex: 1, backgroundColor: colors.background }}>
      <PaneHeader style={{ paddingHorizontal: 12 }}>
        <IconButton icon="back" label="Back to docs" color={muted} onPress={() => (router.canGoBack() ? router.back() : router.navigate('/docs'))} />
        <Text numberOfLines={1} style={[type(13, 'medium'), { flex: 1 }]}>
          {page.data ? pageTitle(page.data) : ''}
        </Text>
        <View>
          <IconButton icon="comment" label={openThreads > 0 ? `Comments, ${openThreads} open` : 'Comments'} color={muted} onPress={() => setCommentsOpen(true)} />
          {openThreads > 0 ? (
            <View pointerEvents="none" style={{ position: 'absolute', top: -4, right: -6 }}>
              <CountBadge count={openThreads} />
            </View>
          ) : null}
        </View>
        <IconButton icon="star" label={favorite ? 'Remove from favorites' : 'Add to favorites'} color={favorite ? colors.primary : muted} onPress={() => toggleFavorite.mutate({ pageId: id, favorite: !favorite })} />
        <IconButton icon="more" label="Page options" color={muted} onPress={() => setMenu(pageMenu())} />
      </PaneHeader>
      {notice ? (
        <Text accessibilityRole="alert" onPress={() => setNotice(null)} style={[type(13), { marginHorizontal: 12, marginBottom: 4, padding: 10, borderRadius: 8, overflow: 'hidden', color: colors.destructive, backgroundColor: alpha(colors.destructive, 0.1) }]}>
          {notice}
        </Text>
      ) : null}
      {!page.data ? (
        page.isError ? (
          <ErrorView error={page.error} onRetry={() => void page.refetch()} />
        ) : (
          <LoadingView />
        )
      ) : (
        <KeyboardAvoidingView style={{ flex: 1 }} behavior={Platform.OS === 'ios' ? 'padding' : undefined} keyboardVerticalOffset={0}>
          <ScrollView
            contentContainerStyle={{ paddingBottom: 32 }}
            keyboardShouldPersistTaps="handled"
            refreshControl={<RefreshControl refreshing={page.isRefetching} onRefresh={() => void page.refetch()} tintColor={colors.mutedForeground} />}
          >
            {page.data.cover_url ? (
              <View>
                <Image
                  accessibilityLabel="Cover"
                  source={{ uri: absoluteUrl(session.server, page.data.cover_url), headers: page.data.cover_url.startsWith('/') ? { Authorization: `Bearer ${session.token}` } : undefined }}
                  style={{ height: 140, width: '100%', backgroundColor: colors.muted }}
                  resizeMode="cover"
                />
                {!locked ? (
                  <View style={{ position: 'absolute', right: 12, bottom: 8 }}>
                    <Button label="Change cover" size="xs" variant="secondary" onPress={() => setMenu(coverActions())} />
                  </View>
                ) : null}
              </View>
            ) : null}
            <View style={{ paddingHorizontal: 12, paddingTop: 24 }}>
              {page.data.icon ? (
                <Text accessibilityRole="button" accessibilityLabel="Change icon" onPress={locked ? undefined : () => setChoosingIcon(true)} style={{ fontSize: 40, lineHeight: 48, marginBottom: 8, alignSelf: 'flex-start' }}>
                  {page.data.icon}
                </Text>
              ) : null}
              {!locked && (!page.data.icon || !page.data.cover_url) ? (
                <View style={{ flexDirection: 'row', gap: 8, marginBottom: 10 }}>
                  {page.data.icon ? null : <Button label="😀 Add icon" variant="ghost" size="sm" muted onPress={() => setChoosingIcon(true)} />}
                  {page.data.cover_url ? null : <Button label="🖼️ Add cover" variant="ghost" size="sm" muted onPress={() => setMenu(coverActions())} />}
                </View>
              ) : null}
              {locked ? (
                <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6, marginBottom: 10 }}>
                  <Icon name="lock" size={13} color={colors.mutedForeground} />
                  <Text style={[type(12), { color: colors.mutedForeground }]}>Locked. Unlock it on the web to edit.</Text>
                </View>
              ) : null}
              <TextInput
                accessibilityLabel="Page title"
                value={title ?? page.data.title}
                placeholder="Untitled"
                placeholderTextColor={muted}
                editable={!locked}
                onChangeText={setTitle}
                onBlur={saveTitle}
                onSubmitEditing={saveTitle}
                returnKeyType="done"
                style={[type(16, 'bold'), { lineHeight: 20, marginBottom: 17, padding: 0, color: colors.foreground }]}
              />
              <View style={{ gap: 6 }}>
                {blocks.map((block) => (
                  <Block key={block.id} block={block} />
                ))}
              </View>
              {!locked ? (
                <View style={{ marginTop: 20, gap: 8 }}>
                  <TextInput
                    accessibilityLabel="Add text"
                    placeholder={blocks.length === 0 ? 'Start writing…' : 'Add text…'}
                    placeholderTextColor={muted}
                    value={text}
                    onChangeText={setText}
                    multiline
                    style={[type(16), { minHeight: 72, lineHeight: 24, padding: 10, borderRadius: 10, borderWidth: 1, borderColor: colors.input, textAlignVertical: 'top', color: colors.foreground }]}
                  />
                  <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
                    <Text style={[type(12), { flex: 1, color: colors.mutedForeground }]}>Each line becomes a paragraph at the end of the page.</Text>
                    <Button
                      label="Add"
                      size="sm"
                      loading={addParagraphs.isPending}
                      disabled={!text.trim()}
                      onPress={() => addParagraphs.mutate(text, { onSuccess: () => setText(''), onError: failed('Could not add the text; the page changed meanwhile, so it is reloaded.') })}
                    />
                  </View>
                </View>
              ) : null}
            </View>
          </ScrollView>
        </KeyboardAvoidingView>
      )}
      <ActionSheet visible={menu !== null} actions={menu ?? []} onClose={() => setMenu(null)} />
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
      <PromptDialog request={prompt} onClose={() => setPrompt(null)} />
      <IconDialog visible={choosingIcon} title="Page icon" current={page.data?.icon ?? null} onPick={saveIcon} onClose={() => setChoosingIcon(false)} />
      <CommentsSheet pageId={id} visible={commentsOpen} onClose={() => setCommentsOpen(false)} />
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
