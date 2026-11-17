import type { CollabProblem, CollabState } from '@orbit/editor/collab/session'
import { isSafeLinkHref } from '@orbit/editor/links'
import { router } from 'expo-router'
import { type ComponentRef, memo, useEffect, useMemo, useRef, useState } from 'react'
import { type LayoutChangeEvent, Linking, type NativeScrollEvent, type NativeSyntheticEvent, type ScrollView } from 'react-native'

import { collabTicket, fileDataUrl, fileDeleted } from '../../data/docs-api'
import { type FormatState, whileMounted } from '../../data/page-editor'
import { usePageTrash } from '../../hooks/docs'
import { useOrbit } from '../../orbit-context'
import { useMembers, usePages } from '../../queries'
import { useTheme } from '../../theme'
import PageEditor, { type PageCardInfo, type PageEditorProps, type PageEditorRef } from './PageEditor.dom'

type EditorViewProps = Pick<PageEditorProps, 'kind' | 'docId' | 'epoch' | 'generation' | 'editable'> & {
  ref: React.Ref<PageEditorRef>
  onStatus: (state: CollabState) => void
  onProblem: (problem: CollabProblem) => void
  onProbe: () => void
  onFormat: (state: FormatState) => void
}

/**
 * The page editor (a docs page, or a task's description) with what it needs from the app: the session (tickets,
 * files), the theme, page and member names.
 * Every render sends all props to the web view, so it renders only when they change: the screen re-renders on every
 * selection change (the toolbar), and the callbacks are read through a ref.
 */
export const EditorView = memo(function EditorView({ ref, kind, docId, epoch, generation, editable, ...callbacks }: EditorViewProps) {
  const { client, workspaceId, session, user } = useOrbit()
  const { scheme, colors } = useTheme()
  const pages = usePages()
  const trash = usePageTrash()
  const members = useMembers()
  const [height, setHeight] = useState(120)
  const latest = useRef(callbacks)
  useEffect(() => {
    latest.current = callbacks
  })
  const mounted = useRef(true)
  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
    }
  }, [])

  const cards = useMemo(() => {
    const out: Record<string, PageCardInfo> = {}
    for (const item of trash.data ?? []) out[item.id] = { title: item.title, icon: item.icon, trashed: true }
    for (const item of pages.data ?? []) out[item.id] = { title: item.title, icon: item.icon, trashed: false }
    return out
  }, [pages.data, trash.data])
  const names = useMemo(() => Object.fromEntries((members.data ?? []).map((member) => [member.user_id, member.display_name])), [members.data])
  const editorColors = useMemo(
    () => ({ background: colors.background, foreground: colors.foreground, mutedForeground: colors.mutedForeground, muted: colors.muted, border: colors.border, primary: colors.primary }),
    [colors],
  )
  // the server shows everyone else this session's name and a color of its own
  const me = useMemo(() => ({ name: user.display_name, color: colors.mutedForeground }), [user.display_name, colors.mutedForeground])
  const actions = useMemo(
    () => ({
      getTicket: whileMounted(mounted, () => collabTicket(client, docId, kind === 'tasks' ? 'task' : 'page')),
      loadFile: whileMounted(mounted, (url: string) => fileDataUrl(session.server, session.token, url)),
      fileDeleted: whileMounted(mounted, (url: string) => fileDeleted(session.server, session.token, url)),
      openPage: whileMounted(mounted, async (target: string) => router.push(`/docs/${target}`)),
      // the editor already filters links; the app checks again before it hands one to the OS
      openUrl: whileMounted(mounted, async (url: string) => {
        if (isSafeLinkHref(url)) void Linking.openURL(url).catch(() => {})
      }),
      onStatus: whileMounted(mounted, async (state: CollabState) => latest.current.onStatus(state)),
      onProblem: whileMounted(mounted, async (problem: CollabProblem) => latest.current.onProblem(problem)),
      onProbe: whileMounted(mounted, async () => latest.current.onProbe()),
      onFormat: whileMounted(mounted, async (state: FormatState) => latest.current.onFormat(state)),
      onHeight: whileMounted(mounted, async (next: number) => setHeight(next)),
    }),
    [client, kind, docId, session.server, session.token],
  )
  const dom = useMemo(() => ({ scrollEnabled: false, style: { height } }), [height])

  return (
    <PageEditor
      ref={ref}
      dom={dom}
      server={session.server}
      workspaceId={workspaceId}
      kind={kind}
      docId={docId}
      epoch={epoch}
      generation={generation}
      editable={editable}
      theme={scheme}
      colors={editorColors}
      user={me}
      pages={cards}
      members={names}
      {...actions}
    />
  )
})

/** Room kept between the caret and the toolbar or the top of the screen while typing. */
const CARET_MARGIN = 24

/**
 * Keeps the editor's caret between the top of the screen and the toolbar while typing: the editor does not scroll
 * itself, the screen's `ScrollView` does. Spread `scrollProps` on that scroll view and put `onEditorLayout` on the
 * editor's wrapper, a direct child of the scroll view's content.
 */
export function useCaretFollow(format: FormatState) {
  const scroll = useRef<ComponentRef<typeof ScrollView>>(null)
  const scrollY = useRef(0)
  const viewportHeight = useRef(0)
  const editorTop = useRef(0)
  useEffect(() => {
    if (!format.focused || !format.caret) return
    const top = editorTop.current + format.caret.top
    const bottom = editorTop.current + format.caret.bottom
    const visibleTop = scrollY.current + CARET_MARGIN
    const visibleBottom = scrollY.current + viewportHeight.current - CARET_MARGIN
    if (bottom > visibleBottom) scroll.current?.scrollTo({ y: bottom - viewportHeight.current + CARET_MARGIN, animated: true })
    else if (top < visibleTop) scroll.current?.scrollTo({ y: Math.max(0, top - CARET_MARGIN), animated: true })
  }, [format])
  return {
    scrollProps: {
      ref: scroll,
      scrollEventThrottle: 16,
      onScroll: (event: NativeSyntheticEvent<NativeScrollEvent>) => (scrollY.current = event.nativeEvent.contentOffset.y),
      onLayout: (event: LayoutChangeEvent) => (viewportHeight.current = event.nativeEvent.layout.height),
    },
    onEditorLayout: (event: LayoutChangeEvent) => (editorTop.current = event.nativeEvent.layout.y),
  }
}
