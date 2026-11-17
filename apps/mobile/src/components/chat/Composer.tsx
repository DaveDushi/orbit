import { useQueryClient } from '@tanstack/react-query'
import { router } from 'expo-router'
import { applyChatActions, applyChatEvents, chatQueryKeys } from '../../chat-cache'
import * as DocumentPicker from 'expo-document-picker'
import { requireOptionalNativeModule } from 'expo'
import type { PasteEventPayload, TextInputWrapperViewProps } from 'expo-paste-input'
import * as ImagePicker from 'expo-image-picker'
import { type ComponentType, type ReactNode, useMemo, useState } from 'react'
import { Image, Platform, Pressable, ScrollView, View } from 'react-native'
import { TextInput } from '../../ui/scaled'

import type { PageContext } from '../../data/ask-donna'
import { authorName, decodeMentions, newNonce, type Person, type ThreadPages, upsertThreadMessage } from '../../data/chat'
import * as chat from '../../data/chat-api'
import { encodeMentions, insertMention, type MentionQuery, mentionQueryAt, mentionSuggestions, notifiesEveryone } from '../../data/mentions'
import type { MessageRecord } from '../../data/orbit'
import { confirmAction, showError } from '../../hooks/chat'

import { canScheduleFrom } from '../../data/scheduled'
import { useScheduleMessage } from '../../hooks/scheduled'
import { useIsGuest, useOrbit } from '../../orbit-context'
import { useConversations, useMembers } from '../../queries'
import { useTheme } from '../../theme'
import { Avatar, Icon, IconButton, Spinner, Text } from '../../ui'
import { uploadPart } from '../../upload-part'
import { EmojiPicker } from './EmojiPicker'
import { MicButton } from './MicButton'
import { ScheduledBar, SendLaterSheet } from './SendLater'
import { ActionSheet } from './Sheet'
import { StickerPicker, useStickers } from './StickerPicker'

/** A picked file: it uploads at once, and the message names it once it is there. */
interface DraftFile {
  key: string
  name: string
  /** Local preview of an image. */
  previewUri: string | null
  status: 'uploading' | 'ready' | 'failed'
  file: chat.ChatFileRecord | null
}

/** The server's limit for a message body. */
const MESSAGE_MAX_LENGTH = 4000
/** `@channel` or `@here` asks first in a channel of this many members, as on the web. */
const CONFIRM_FROM_MEMBERS = 6
/** Unsent text per workspace, conversation and thread, while the app runs. */
const drafts = new Map<string, string>()

/** A message the server has not confirmed yet. */
export interface PendingMessage {
  nonce: string
  body: string
  files: number
}

/** What the composer writes: a new message (maybe quoting one) or an edit of your own. */
type Mode = { kind: 'new'; replyTo: MessageRecord | null } | { kind: 'edit'; message: MessageRecord }

/**
 * The composer's state and its writes for one conversation or thread: the draft, the picked files and their uploads,
 * the reply or edit target, and the messages on their way.
 */
export function useComposer(conversationId: string, threadRootId: string | null, pageContext?: () => PageContext | null) {
  const { client, workspaceId, user } = useOrbit()
  const queryClient = useQueryClient()
  const draftKey = `${workspaceId}:${conversationId}:${threadRootId ?? ''}`
  const [draft, setDraftState] = useState(() => drafts.get(draftKey) ?? '')
  // the draft outlives the screen, as the web keeps it per conversation and thread (in memory here: not a secret, not worth the keychain)
  const setDraft = (next: string | ((current: string) => string)) =>
    setDraftState((current) => {
      const value = typeof next === 'function' ? next(current) : next
      if (value) drafts.set(draftKey, value)
      else drafts.delete(draftKey)
      return value
    })
  const members = useMembers().data
  const conversations = useConversations().data
  const conversation = conversations?.find((item) => item.id === conversationId)
  // who `@` offers: everybody in a public channel, the members of a private channel or a DM (not suspended, not you)
  const mentionable: Person[] = useMemo(
    () =>
      (members ?? [])
        .filter((member) => !member.suspended_at && member.user_id !== user.id && (conversation?.kind === 'public' || (conversation?.member_ids.includes(member.user_id) ?? false)))
        .map((member) => ({ id: member.user_id, name: member.display_name })),
    [conversation, members, user.id],
  )
  // every member and channel encodes, also a name typed by hand
  const everybody: Person[] = useMemo(() => (members ?? []).map((member) => ({ id: member.user_id, name: member.display_name })), [members])
  const channels: Person[] = useMemo(() => (conversations ?? []).filter((item) => item.kind !== 'dm' && item.member_ids.includes(user.id)).map((item) => ({ id: item.id, name: item.name })), [conversations, user.id])
  const allNames: Person[] = useMemo(() => (conversations ?? []).map((item) => ({ id: item.id, name: item.name })), [conversations])
  const [files, setFiles] = useState<DraftFile[]>([])
  const [mode, setMode] = useState<Mode>({ kind: 'new', replyTo: null })
  const [alsoInChannel, setAlsoInChannel] = useState(false)
  const [pending, setPending] = useState<PendingMessage[]>([])
  const guest = useIsGuest()
  const scheduling = useScheduleMessage()
  const schedulingHere = threadRootId === null && pageContext === undefined && !guest

  const upload = (key: string, picked: { uri: string; name: string; width?: number; height?: number }) =>
    uploadPart(picked.uri, picked.name)
      .then((part) => chat.uploadFile(client, workspaceId, { part, width: picked.width, height: picked.height }))
      .then(
      (file) => setFiles((list) => list.map((item) => (item.key === key ? { ...item, status: 'ready', file } : item))),
      (error) => {
        setFiles((list) => list.map((item) => (item.key === key ? { ...item, status: 'failed' } : item)))
        showError('The file was not uploaded', error)
      },
    )

  const addFiles = (picked: { name: string; uri: string; image: boolean; width?: number; height?: number }[]) => {
    for (const item of picked) {
      const key = newNonce()
      setFiles((list) => [...list, { key, name: item.name, previewUri: item.image ? item.uri : null, status: 'uploading', file: null }])
      void upload(key, item)
    }
  }

  const received = (message: MessageRecord) => {
    applyChatActions(queryClient, workspaceId, [{ kind: 'upsert-message', conversationId, message }])
    if (message.thread_root_id) {
      queryClient.setQueryData<ThreadPages>(chatQueryKeys.thread(workspaceId, message.thread_root_id), (data) => data && upsertThreadMessage(data, message))
    }
  }

  const ready = files.filter((item) => item.status === 'ready')
  const uploading = files.some((item) => item.status === 'uploading')
  const canSend = !uploading && (draft.trim() !== '' || (mode.kind === 'new' && ready.length > 0))

  const send = async () => {
    const text = draft.trim()
    const body = encodeMentions(text, everybody, channels)
    if (!canSend) return
    if (body.length > MESSAGE_MAX_LENGTH) return showError('Message too long', new Error(`A message has at most ${MESSAGE_MAX_LENGTH} characters.`))
    if (mode.kind === 'edit') {
      const { message } = mode
      setDraft('')
      setMode({ kind: 'new', replyTo: null })
      if (body === message.body) return
      chat.editMessage(client, workspaceId, message.id, body).then(
        ({ result, events }) => {
          applyChatEvents(queryClient, workspaceId, events)
          received(result)
        },
        (error) => {
          setDraft(text)
          setMode({ kind: 'edit', message })
          showError('The message was not edited', error)
        },
      )
      return
    }
    const memberCount = conversation?.member_ids.length ?? 0
    if (!threadRootId && notifiesEveryone(body) && memberCount >= CONFIRM_FROM_MEMBERS) {
      const all = body.includes('<!channel>')
      const confirmed = await confirmAction(`Notify ${memberCount} people?`, `${all ? '@channel' : '@here'} sends a notification to everyone in this channel${all ? '' : ' who is online'}.`, 'Send')
      if (!confirmed) return
    }
    const item = { nonce: newNonce(), body, files: ready.length }
    const replyTo = mode.replyTo
    const fileIds = ready.flatMap((file) => (file.file ? [file.file.id] : []))
    setDraft('')
    setFiles([])
    setMode({ kind: 'new', replyTo: null })
    setPending((list) => [...list, item])
    chat.send(client, workspaceId, conversationId, { body, nonce: item.nonce, threadRootId, replyToId: replyTo?.id ?? null, fileIds, alsoInChannel: threadRootId !== null && alsoInChannel, pageContext: pageContext?.() }).then(
      ({ result, events }) => {
        applyChatEvents(queryClient, workspaceId, events)
        received(result)
        setPending((list) => list.filter((other) => other.nonce !== item.nonce))
      },
      (error) => {
        setPending((list) => list.filter((other) => other.nonce !== item.nonce))
        setDraft((current) => current || text)
        showError('Message not sent', error)
      },
    )
  }

  /**
   * A sticker goes at once, as a message of its own: it answers the reply target if there is one, and leaves the text
   * and the files as they are.
   */
  const sendSticker = (stickerId: string) => {
    const item = { nonce: newNonce(), body: '', files: 0 }
    const replyTo = mode.kind === 'new' ? mode.replyTo : null
    if (replyTo) setMode({ kind: 'new', replyTo: null })
    setPending((list) => [...list, item])
    chat.send(client, workspaceId, conversationId, { body: '', nonce: item.nonce, threadRootId, replyToId: replyTo?.id ?? null, stickerId, pageContext: pageContext?.() }).then(
      ({ result, events }) => {
        applyChatEvents(queryClient, workspaceId, events)
        received(result)
        setPending((list) => list.filter((other) => other.nonce !== item.nonce))
      },
      (error) => {
        setPending((list) => list.filter((other) => other.nonce !== item.nonce))
        showError('Sticker not sent', error)
      },
    )
  }

  return {
    sendSticker,
    draft,
    setDraft,
    files,
    removeFile: (key: string) => setFiles((list) => list.filter((item) => item.key !== key)),
    addFiles,
    mode,
    conversationId,
    reply: (message: MessageRecord) => setMode({ kind: 'new', replyTo: message }),
    edit: (message: MessageRecord) => {
      setMode({ kind: 'edit', message })
      // the field shows names; the save turns them back into tokens
      setDraft(decodeMentions(message.body, everybody, allNames))
    },
    cancelMode: () => {
      if (mode.kind === 'edit') setDraft('')
      setMode({ kind: 'new', replyTo: null })
    },
    alsoInChannel,
    setAlsoInChannel,
    pending,
    canSend,
    send: () => void send(),
    /** Who `@` and `#` offer in this conversation; `@channel` and `@here` only outside a thread. */
    mentions: { people: mentionable, channels, everyone: threadRootId === null },
    /** Send later is offered here (the main list of a conversation, not Ask Donna, not for a guest). */
    schedulingHere,
    canSchedule: canScheduleFrom({ text: draft, files: files.length, editing: mode.kind === 'edit', inThread: threadRootId !== null, askDonna: pageContext !== undefined, guest }),
    schedulePending: scheduling.isPending,
    /** Schedules the text and empties the composer; a quoted message stays out, as on the web. */
    scheduleAt: (at: Date, done: () => void) =>
      scheduling.mutate(
        { conversationId, body: encodeMentions(draft.trim(), everybody, channels), sendAt: at },
        {
          onSuccess: () => {
            setDraft('')
            setMode({ kind: 'new', replyTo: null })
            done()
          },
          onError: (error) => showError('The message could not be scheduled', error),
        },
      ),
  }
}

export type ComposerState = ReturnType<typeof useComposer>

/**
 * The web chat composer: the reply or edit banner, the picked files, the field, then attach / emoji / mention on the
 * left and send on the right. In a thread, "Also send to" the conversation sits under it.
 */
export function Composer({ state, placeholder, nameOf, alsoSendTo }: { state: ComposerState; placeholder: string; nameOf: (userId: string) => string; alsoSendTo?: string }) {
  const { colors, scheme } = useTheme()
  const [focused, setFocused] = useState(false)
  const [attachOpen, setAttachOpen] = useState(false)
  const [sendMenu, setSendMenu] = useState(false)
  const [stickerOpen, setStickerOpen] = useState(false)
  const stickers = useStickers().data ?? []
  const [laterOpen, setLaterOpen] = useState(false)
  const [emojiOpen, setEmojiOpen] = useState(false)
  const [cursor, setCursor] = useState<number | null>(null)
  const { draft, setDraft, mode } = state
  // a mention being typed at the cursor (the end while the field has not said where the cursor is)
  const mention: MentionQuery | null = focused ? mentionQueryAt(draft, cursor ?? draft.length) : null
  const suggestions = mention ? mentionSuggestions(mention, mention.trigger === '#' ? [] : state.mentions.people, state.mentions.channels, { everyone: state.mentions.everyone }) : []
  const pickMention = (name: string) => {
    if (!mention) return
    const next = insertMention(draft, mention, name)
    setDraft(next.text)
    setCursor(next.cursor)
  }
  const append = (text: string) => setDraft(`${draft}${draft && !draft.endsWith(' ') ? ' ' : ''}${text}`)
  // A pasted photo (the edit menu's Paste, or the keyboard's "Paste from Screenshots") becomes an attachment.
  const onPaste = (payload: PasteEventPayload) => {
    if (payload.type !== 'images' || mode.kind === 'edit') return
    state.addFiles(payload.uris.map((uri, index) => ({ name: `pasted-${Date.now()}-${index + 1}.${uri.split('.').pop() ?? 'png'}`, uri, image: true })))
  }

  const pickImages = async (camera: boolean) => {
    const options: ImagePicker.ImagePickerOptions = { mediaTypes: ['images', 'videos'], allowsMultipleSelection: !camera, selectionLimit: 10, quality: 0.9 }
    if (camera) {
      const permission = await ImagePicker.requestCameraPermissionsAsync()
      if (!permission.granted) return showError('No camera access', new Error('Allow Orbit to use the camera in Settings.'))
    }
    const result = camera ? await ImagePicker.launchCameraAsync(options) : await ImagePicker.launchImageLibraryAsync(options)
    if (result.canceled) return
    state.addFiles(
      result.assets.map((asset) => ({
        name: asset.fileName ?? `photo-${Date.now()}.${asset.mimeType?.split('/')[1] ?? 'jpg'}`,
        uri: asset.uri,
        image: asset.type === 'image',
        width: asset.width || undefined,
        height: asset.height || undefined,
      })),
    )
  }
  const pickDocuments = async () => {
    const result = await DocumentPicker.getDocumentAsync({ multiple: true, copyToCacheDirectory: true })
    if (result.canceled) return
    state.addFiles(
      result.assets.map((asset) => ({
        name: asset.name,
        uri: asset.uri,
        image: asset.mimeType?.startsWith('image/') ?? false,
      })),
    )
  }

  const banner = mode.kind === 'edit' ? 'Editing message' : mode.replyTo ? `Replying to ${authorName(mode.replyTo, nameOf)}` : null
  return (
    <View className="m-2 gap-1.5">
      {state.schedulingHere ? <ScheduledBar conversationId={state.conversationId} /> : null}
      {banner ? (
        <View className="flex-row items-center gap-1.5 pl-2.5">
          <Icon name={mode.kind === 'edit' ? 'edit' : 'reply'} size={14} color={colors.mutedForeground} />
          <Text numberOfLines={1} className="text-[12px] flex-1 text-muted-foreground">
            {banner}
          </Text>
          <IconButton icon="x" label={mode.kind === 'edit' ? 'Cancel editing' : 'Cancel reply'} size={24} iconSize={14} color={colors.mutedForeground} onPress={state.cancelMode} />
        </View>
      ) : null}
      <View className={`rounded-lg border bg-input/30 ${focused ? 'border-ring' : 'border-input'}`}>
        {state.files.length > 0 ? (
          <ScrollView horizontal keyboardShouldPersistTaps="handled" contentContainerClassName="gap-1.5 p-2 pb-0">
            {state.files.map((file) => (
              <View key={file.key} className={`h-14 flex-row items-center gap-2 pr-7 rounded-md border bg-background overflow-hidden ${file.previewUri ? 'pl-0' : 'pl-2.5'} ${file.status === 'failed' ? 'border-destructive' : 'border-border'}`}>
                {file.previewUri ? <Image source={{ uri: file.previewUri }} className="size-14" /> : <Icon name="attach" size={18} color={colors.mutedForeground} />}
                <View className="max-w-35">
                  <Text numberOfLines={1} className="font-noto-medium text-[12px]">
                    {file.name}
                  </Text>
                  <Text className={`text-[11px] ${file.status === 'failed' ? 'text-destructive' : 'text-muted-foreground'}`}>{file.status === 'uploading' ? 'Uploading…' : file.status === 'failed' ? 'Upload failed' : 'Ready'}</Text>
                </View>
                {file.status === 'uploading' ? <Spinner /> : null}
                <View className="absolute top-0.5 right-0.5">
                  <IconButton icon="x" label={`Remove ${file.name}`} size={22} iconSize={12} color={colors.mutedForeground} onPress={() => state.removeFile(file.key)} />
                </View>
              </View>
            ))}
          </ScrollView>
        ) : null}
        {suggestions.length > 0 ? (
          <View accessibilityRole="menu" accessibilityLabel={mention?.trigger === '#' ? 'Channels' : 'People'} className="p-1 border-b border-border">
            {suggestions.map((item) => (
              <Pressable
                key={item.id}
                accessibilityRole="menuitem"
                accessibilityLabel={`${mention?.trigger}${item.name}`}
                onPress={() => pickMention(item.name)}
                className="h-9 flex-row items-center gap-2 px-2 rounded-md active:bg-muted"
              >
                {mention?.trigger === '#' ? <Icon name="hashtag" size={16} color={colors.mutedForeground} /> : item.note ? <Icon name="people" size={16} color={colors.mutedForeground} /> : <Avatar id={item.id} name={item.name} size={20} />}
                <Text numberOfLines={1} className="font-noto-medium text-[14px] shrink">
                  {item.name}
                </Text>
                {item.note ? <Text className="text-[12px] text-muted-foreground">{item.note}</Text> : null}
              </Pressable>
            ))}
          </View>
        ) : null}
        <PasteField onPaste={onPaste}>
          <TextInput
            value={draft}
            onChangeText={(text) => {
              setDraft(text)
              // typing at the end: the selection event can come after the change
              if (cursor === null || cursor >= draft.length) setCursor(text.length)
            }}
            onSelectionChange={(event) => setCursor(event.nativeEvent.selection.end)}
            onFocus={() => setFocused(true)}
            onBlur={() => setFocused(false)}
            placeholder={placeholder}
            placeholderTextColor={colors.mutedForeground}
            selectionColor={colors.primary}
            keyboardAppearance={scheme}
            multiline
            numberOfLines={1}
            accessibilityLabel={placeholder}
            className="min-h-10 max-h-40 px-3 py-2 text-[16px] leading-[24px] font-noto text-foreground"
          />
        </PasteField>
        <View className="flex-row items-center gap-1 p-1.5">
          <IconButton icon="attach" label="Attach files" size={32} iconSize={20} disabled={mode.kind === 'edit'} onPress={() => setAttachOpen(true)} />
          <IconButton icon="smile" label="Emoji" size={32} iconSize={20} onPress={() => setEmojiOpen(true)} />
          {stickers.length > 0 ? <IconButton icon="sticker" label="Send a sticker" size={32} iconSize={20} disabled={mode.kind === 'edit'} onPress={() => setStickerOpen(true)} /> : null}
          <IconButton icon="at" label="Mention" size={32} iconSize={20} onPress={() => append('@')} />
          <IconButton icon="hashtag" label="Mention a channel or task" size={32} iconSize={20} onPress={() => append('#')} />
          <View className="flex-1" />
          <MicButton text={draft} setText={setDraft} />
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={mode.kind === 'edit' ? 'Save' : 'Send'}
            accessibilityState={{ disabled: !state.canSend }}
            disabled={!state.canSend}
            onPress={state.send}
            // like iMessage: hold Send for more ways to send
            onLongPress={state.schedulingHere && mode.kind === 'new' ? () => setSendMenu(true) : undefined}
            accessibilityHint={state.schedulingHere && mode.kind === 'new' ? 'Hold for Send later' : undefined}
            accessibilityActions={state.schedulingHere && mode.kind === 'new' ? [{ name: 'longpress', label: 'Send later' }] : undefined}
            onAccessibilityAction={(event) => {
              if (event.nativeEvent.actionName === 'longpress') setSendMenu(true)
            }}
            hitSlop={4}
            className={`size-8 rounded-lg items-center justify-center active:bg-muted ${state.canSend ? 'opacity-100' : 'opacity-50'}`}
          >
            <Icon name={mode.kind === 'edit' ? 'check' : 'send'} size={20} color={colors.foreground} />
          </Pressable>
        </View>
      </View>
      {alsoSendTo !== undefined ? (
        <Pressable
          accessibilityRole="checkbox"
          accessibilityState={{ checked: state.alsoInChannel }}
          onPress={() => state.setAlsoInChannel(!state.alsoInChannel)}
          className="flex-row items-center gap-2 px-1 py-0.5"
        >
          <View className={`size-4 rounded-[4px] border items-center justify-center ${state.alsoInChannel ? 'border-primary bg-primary' : 'border-input'}`}>
            {state.alsoInChannel ? <Icon name="check" size={11} color={colors.primaryForeground} /> : null}
          </View>
          <Text className="text-[12px] text-muted-foreground">Also send to {alsoSendTo}</Text>
        </Pressable>
      ) : null}
      <ActionSheet
        visible={sendMenu}
        items={[
          { key: 'later', label: 'Send later…', icon: 'clock', disabled: !state.canSchedule, onPress: () => setLaterOpen(true) },
          { key: 'list', label: 'Scheduled messages', icon: 'list', onPress: () => router.push('/chat/scheduled') },
        ]}
        onClose={() => setSendMenu(false)}
      />
      {stickerOpen ? (
        <StickerPicker
          stickers={stickers}
          onPick={(sticker) => {
            setStickerOpen(false)
            state.sendSticker(sticker.id)
          }}
          onClose={() => setStickerOpen(false)}
        />
      ) : null}
      {laterOpen ? <SendLaterSheet pending={state.schedulePending} onPick={(at) => state.scheduleAt(at, () => setLaterOpen(false))} onClose={() => setLaterOpen(false)} /> : null}
      <ActionSheet
        visible={attachOpen}
        title="Attach"
        items={[
          { key: 'library', label: 'Photo library', icon: 'display', onPress: () => void pickImages(false).catch((error) => showError('Could not open the photo library', error)) },
          ...(Platform.OS === 'web' ? [] : [{ key: 'camera', label: 'Take a photo', icon: 'display' as const, onPress: () => void pickImages(true).catch((error) => showError('Could not open the camera', error)) }]),
          { key: 'file', label: 'Choose a file', icon: 'documentDownload', onPress: () => void pickDocuments().catch((error) => showError('Could not open the files', error)) },
        ]}
        onClose={() => setAttachOpen(false)}
      />
      <EmojiPicker visible={emojiOpen} onPick={(emoji) => setDraft(`${draft}${emoji}`)} onClose={() => setEmojiOpen(false)} />
    </View>
  )
}

/**
 * `expo-paste-input` hands pasted images to JavaScript; React Native's TextInput drops them. It has native code, so
 * Expo Go lacks it: there the field stays a plain TextInput.
 */
const PasteInput: ComponentType<TextInputWrapperViewProps> | null =
  requireOptionalNativeModule('ExpoPasteInput') ? (require('expo-paste-input') as typeof import('expo-paste-input')).TextInputWrapper : null

function PasteField({ onPaste, children }: { onPaste: (payload: PasteEventPayload) => void; children: ReactNode }) {
  return PasteInput ? <PasteInput onPaste={onPaste}>{children}</PasteInput> : children
}
