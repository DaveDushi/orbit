import { useState } from 'react'
import { KeyboardAvoidingView, Modal, Platform, ScrollView, TextInput, View } from 'react-native'
import { SafeAreaView } from 'react-native-safe-area-context'

import type { PageThread } from '../../data/docs-api'
import { visibleThreads } from '../../data/docs'
import { relativeAgo } from '../../data/format'
import { useAddComment, usePageThreads } from '../../hooks/docs'
import { usePeople } from '../../queries'
import { errorMessage } from '../../session-context'
import { alpha, type, useTheme } from '../../theme'
import { Avatar, Button, IconButton, LoadingView, PaneHeader, Text } from '../../ui'

/**
 * The page's comments panel at phone size: every thread with its comments, a reply field under each, and a new
 * comment on the whole page at the bottom. Comments on a passage start on the web, where the text can be selected.
 */
export function CommentsSheet({ pageId, visible, onClose }: { pageId: string; visible: boolean; onClose: () => void }) {
  const { colors } = useTheme()
  const threads = usePageThreads(pageId)
  const add = useAddComment(pageId)
  const [text, setText] = useState('')
  const list = visibleThreads(threads.data ?? [])
  const submit = () => {
    if (!text.trim()) return
    add.mutate({ text }, { onSuccess: () => setText('') })
  }
  return (
    <Modal visible={visible} animationType="slide" presentationStyle="pageSheet" onRequestClose={onClose}>
      <SafeAreaView edges={['top', 'bottom', 'left', 'right']} style={{ flex: 1, backgroundColor: colors.background }}>
        <PaneHeader style={{ borderBottomWidth: 1, borderBottomColor: colors.border }}>
          <Text style={[type(13.5, 'medium'), { flex: 1, paddingLeft: 4 }]}>Comments</Text>
          <IconButton icon="close" label="Close comments" color={alpha(colors.mutedForeground, 0.7)} onPress={onClose} />
        </PaneHeader>
        <KeyboardAvoidingView style={{ flex: 1 }} behavior={Platform.OS === 'ios' ? 'padding' : undefined}>
          {!threads.data ? (
            threads.isError ? (
              <View style={{ padding: 16 }}>
                <Text style={[type(13), { color: colors.destructive }]}>Comments could not be loaded.</Text>
              </View>
            ) : (
              <LoadingView />
            )
          ) : (
            <ScrollView style={{ flex: 1 }} contentContainerStyle={{ padding: 12, gap: 12 }} keyboardShouldPersistTaps="handled">
              {list.length === 0 ? <Text style={[type(13), { padding: 4, color: colors.mutedForeground }]}>No comments yet.</Text> : null}
              {list.map((thread) => (
                <ThreadCard key={thread.id} pageId={pageId} thread={thread} />
              ))}
            </ScrollView>
          )}
          <View style={{ flexDirection: 'row', alignItems: 'flex-end', gap: 8, padding: 12, borderTopWidth: 1, borderTopColor: colors.border }}>
            <TextInput
              accessibilityLabel="Comment on this page"
              placeholder="Comment on this page"
              placeholderTextColor={colors.mutedForeground}
              value={text}
              onChangeText={setText}
              multiline
              style={[type(15), { flex: 1, maxHeight: 120, minHeight: 36, paddingHorizontal: 10, paddingVertical: 7, borderRadius: 10, borderWidth: 1, borderColor: colors.input, color: colors.foreground }]}
            />
            <Button label="Comment" loading={add.isPending} disabled={!text.trim()} onPress={submit} />
          </View>
          {add.isError ? <Text style={[type(12), { paddingHorizontal: 12, paddingBottom: 8, color: colors.destructive }]}>{errorMessage(add.error)}</Text> : null}
        </KeyboardAvoidingView>
      </SafeAreaView>
    </Modal>
  )
}

function ThreadCard({ pageId, thread }: { pageId: string; thread: PageThread }) {
  const { colors } = useTheme()
  const people = usePeople()
  const reply = useAddComment(pageId)
  const [text, setText] = useState('')
  const nameOf = (id: string) => people.find((person) => person.id === id)?.name ?? 'Someone'
  const comments = thread.comments.filter((comment) => !comment.deleted_at)
  return (
    <View style={{ borderRadius: 10, borderWidth: 1, borderColor: colors.border, padding: 12, gap: 10, opacity: thread.resolved ? 0.75 : 1 }}>
      {thread.quote.trim() ? (
        <Text numberOfLines={2} style={[type(12), { paddingLeft: 8, borderLeftWidth: 2, borderLeftColor: colors.primary, color: colors.mutedForeground }]}>
          {thread.quote.trim()}
        </Text>
      ) : null}
      {thread.resolved ? <Text style={[type(11, 'medium'), { color: colors.mutedForeground }]}>Resolved</Text> : null}
      {comments.map((comment) => (
        <View key={comment.id} style={{ flexDirection: 'row', gap: 8 }}>
          <Avatar id={comment.author_id} name={nameOf(comment.author_id)} size={24} />
          <View style={{ flex: 1, gap: 2 }}>
            <Text style={type(13, 'medium')}>
              {nameOf(comment.author_id)} <Text style={[type(12), { color: colors.mutedForeground }]}>{relativeAgo(comment.created_at)}</Text>
            </Text>
            <Text selectable style={[type(14), { lineHeight: 20 }]}>
              {comment.body_text.trim()}
            </Text>
          </View>
        </View>
      ))}
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
        <TextInput
          accessibilityLabel="Reply"
          placeholder="Reply…"
          placeholderTextColor={colors.mutedForeground}
          value={text}
          onChangeText={setText}
          style={[type(14), { flex: 1, height: 32, paddingHorizontal: 10, paddingVertical: 0, borderRadius: 8, backgroundColor: colors.muted, color: colors.foreground }]}
        />
        {text.trim() ? <Button label="Reply" size="sm" loading={reply.isPending} onPress={() => reply.mutate({ text, threadId: thread.id }, { onSuccess: () => setText('') })} /> : null}
      </View>
      {reply.isError ? <Text style={[type(12), { color: colors.destructive }]}>{errorMessage(reply.error)}</Text> : null}
    </View>
  )
}
