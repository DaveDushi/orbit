import { useState } from 'react'
import { Modal, ScrollView, View } from 'react-native'
import { TextInput } from '../../ui/scaled'

import type { PageThread } from '../../data/docs-api'
import { canChangeComment, canDeleteThread, paragraphBlocks, visibleThreads } from '../../data/docs'
import { relativeAgo } from '../../data/format'
import { useAddComment, usePageThreads, useThreadAction } from '../../hooks/docs'
import { useOrbit } from '../../orbit-context'
import { usePeople } from '../../queries'
import { errorMessage } from '../../session-context'
import { alpha, useTheme } from '../../theme'
import { Avatar, Button, IconButton, KeyboardSheetRoot, KeyboardView, LoadingView, PaneHeader, SafeAreaView, Text } from '../../ui'
import { ActionSheet, PromptDialog, type PromptRequest, type SheetAction } from '../settings/Dialog'

/**
 * The page's comments panel at phone size: every thread with its comments, a reply field under each, and a new
 * comment on the whole page at the bottom. Anyone resolves and reopens a thread; authors edit and delete their own
 * comments, and a thread's creator deletes the thread (the web's rules). Comments on a passage start on the web,
 * where the text can be selected.
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
      <KeyboardSheetRoot>
        <SafeAreaView edges={['top', 'bottom', 'left', 'right']} className="flex-1 bg-background">
          <PaneHeader className="border-b border-border">
            <Text className="font-noto-medium text-[13.5px] flex-1 pl-1">Comments</Text>
            <IconButton icon="close" label="Close comments" color={alpha(colors.mutedForeground, 0.7)} onPress={onClose} />
          </PaneHeader>
          <KeyboardView className="flex-1">
            {!threads.data ? (
              threads.isError ? (
                <View className="p-4">
                  <Text className="text-[13px] text-destructive">Comments could not be loaded.</Text>
                </View>
              ) : (
                <LoadingView />
              )
            ) : (
              <ScrollView className="flex-1" contentContainerClassName="p-3 gap-3" keyboardShouldPersistTaps="handled">
                {list.length === 0 ? <Text className="text-[13px] p-1 text-muted-foreground">No comments yet.</Text> : null}
                {list.map((thread) => (
                  <ThreadCard key={thread.id} pageId={pageId} thread={thread} />
                ))}
              </ScrollView>
            )}
            <View className="flex-row items-end gap-2 p-3 border-t border-border">
              <TextInput
                accessibilityLabel="Comment on this page"
                placeholder="Comment on this page"
                placeholderTextColor={colors.mutedForeground}
                value={text}
                onChangeText={setText}
                multiline
                className="font-noto text-[15px] leading-[1.45] flex-1 max-h-30 min-h-9 px-2.5 py-[7px] rounded-lg border border-input text-foreground"
              />
              <Button label="Comment" loading={add.isPending} disabled={!text.trim()} onPress={submit} />
            </View>
            {add.isError ? <Text className="text-[12px] px-3 pb-2 text-destructive">{errorMessage(add.error)}</Text> : null}
          </KeyboardView>
        </SafeAreaView>
      </KeyboardSheetRoot>
    </Modal>
  )
}

function ThreadCard({ pageId, thread }: { pageId: string; thread: PageThread }) {
  const { colors } = useTheme()
  const { user } = useOrbit()
  const people = usePeople()
  const reply = useAddComment(pageId)
  const act = useThreadAction(pageId)
  const [text, setText] = useState('')
  const [menu, setMenu] = useState<SheetAction[] | null>(null)
  const [prompt, setPrompt] = useState<PromptRequest | null>(null)
  const nameOf = (id: string) => people.find((person) => person.id === id)?.name ?? 'Someone'
  const comments = thread.comments.filter((comment) => !comment.deleted_at)
  const muted = alpha(colors.mutedForeground, 0.7)
  const commentMenu = (comment: PageThread['comments'][number]): SheetAction[] => [
    {
      label: 'Edit',
      icon: 'edit',
      onPress: () =>
        setPrompt({
          title: 'Edit comment',
          label: 'Comment',
          initial: comment.body_text.trim(),
          confirmLabel: 'Save',
          onSubmit: (value) => act.mutateAsync({ kind: 'edit', threadId: thread.id, commentId: comment.id, body: paragraphBlocks(value) }),
        }),
    },
    { label: 'Delete', icon: 'trash', destructive: true, onPress: () => act.mutate({ kind: 'delete-comment', threadId: thread.id, commentId: comment.id }) },
  ]
  return (
    <View className={`rounded-lg border border-border p-3 gap-2.5 ${thread.resolved ? 'opacity-75' : 'opacity-100'}`}>
      <View className="flex-row items-center gap-1 -mt-1 -mr-1.5">
        <Text className="flex-1 font-noto-medium text-[11px] text-muted-foreground">{thread.resolved ? 'Resolved' : ''}</Text>
        <Button
          label={thread.resolved ? 'Reopen' : 'Resolve'}
          size="xs"
          variant="ghost"
          disabled={act.isPending}
          onPress={() => act.mutate({ kind: thread.resolved ? 'reopen' : 'resolve', threadId: thread.id })}
        />
        {canDeleteThread(thread, user.id) ? (
          <IconButton
            icon="more"
            label="Thread options"
            size={22}
            iconSize={13}
            color={muted}
            onPress={() => setMenu([{ label: 'Delete thread', icon: 'trash', destructive: true, onPress: () => act.mutate({ kind: 'delete-thread', threadId: thread.id }) }])}
          />
        ) : null}
      </View>
      {thread.quote.trim() ? (
        <Text numberOfLines={2} className="text-[12px] pl-2 border-l-2 border-primary text-muted-foreground">
          {thread.quote.trim()}
        </Text>
      ) : null}
      {comments.map((comment) => (
        <View key={comment.id} className="flex-row gap-2">
          <Avatar id={comment.author_id} name={nameOf(comment.author_id)} size={24} />
          <View className="flex-1 gap-0.5">
            <View className="flex-row items-center">
              <Text className="flex-1 font-noto-medium text-[13px]">
                {nameOf(comment.author_id)} <Text className="text-[12px] text-muted-foreground">{relativeAgo(comment.created_at)}{comment.edited_at ? ' (edited)' : ''}</Text>
              </Text>
              {canChangeComment(comment, user.id) ? <IconButton icon="more" label="Comment options" size={22} iconSize={13} color={muted} onPress={() => setMenu(commentMenu(comment))} /> : null}
            </View>
            <Text selectable className="text-[14px] leading-[20px]">
              {comment.body_text.trim()}
            </Text>
          </View>
        </View>
      ))}
      <View className="flex-row items-center gap-2">
        <TextInput
          accessibilityLabel="Reply"
          placeholder="Reply…"
          placeholderTextColor={colors.mutedForeground}
          value={text}
          onChangeText={setText}
          className="font-noto text-[14px] leading-[1.45] flex-1 h-8 px-2.5 py-0 rounded-md bg-muted text-foreground"
        />
        {text.trim() ? <Button label="Reply" size="sm" loading={reply.isPending} onPress={() => reply.mutate({ text, threadId: thread.id }, { onSuccess: () => setText('') })} /> : null}
      </View>
      {reply.isError || act.isError ? <Text className="text-[12px] text-destructive">{errorMessage(reply.error ?? act.error)}</Text> : null}
      <ActionSheet visible={menu !== null} actions={menu ?? []} onClose={() => setMenu(null)} />
      <PromptDialog request={prompt} onClose={() => setPrompt(null)} />
    </View>
  )
}
