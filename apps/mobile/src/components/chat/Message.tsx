import { File, Paths } from 'expo-file-system'
import { useState } from 'react'
import { Image, Modal, Platform, Pressable, Share, useWindowDimensions, View } from 'react-native'
import { useSafeAreaInsets } from 'react-native-safe-area-context'

import { fileSize, hasReaction, isImage } from '../../data/chat'
import type { ChatFileRecord } from '../../data/chat-api'
import { relativeAgo, timeOfDay } from '../../data/format'
import type { MessageRecord } from '../../data/orbit'
import { showError, useServerImage } from '../../hooks/chat'
import { useOrbit } from '../../orbit-context'
import { alpha, cssColors, type, useTheme } from '../../theme'
import { Avatar, Icon, IconButton, Spinner, Text } from '../../ui'
import { Emoji } from './EmojiPicker'

/** What a message row needs from the screen: names, the shown text of a body, and what a tap does. */
export interface MessageHandlers {
  nameOf: (userId: string) => string
  /** A stored body as shown: mentions as names, markdown as plain text. */
  text: (body: string) => string
  onLongPress: (message: MessageRecord) => void
  onReact: (message: MessageRecord, emoji: string) => void
  onAddReaction: (message: MessageRecord) => void
  /** Absent in a thread: its replies have no thread of their own. */
  onOpenThread?: (message: MessageRecord) => void
}

/**
 * One message at phone size, like the web: a 30px avatar, name and time when it starts a group, the text, files,
 * reactions and the thread summary. A long press opens the action sheet.
 */
export function MessageItem({ message, groupStart, highlighted = false, handlers }: { message: MessageRecord; groupStart: boolean; highlighted?: boolean; handlers: MessageHandlers }) {
  const { colors } = useTheme()
  const { user } = useOrbit()
  const author = handlers.nameOf(message.author_id)

  if (message.kind !== 'message') {
    const what = message.kind === 'join' ? 'joined' : message.kind === 'leave' ? 'left' : 'pinned a message'
    return (
      <Text style={[type(12), { lineHeight: 16, paddingHorizontal: 12, paddingVertical: 4, color: colors.mutedForeground }]}>
        {author} {what} · {timeOfDay(message.created_at)}
      </Text>
    )
  }

  const body = message.deleted ? 'Message deleted.' : handlers.text(message.body)
  return (
    <Pressable
      onLongPress={() => handlers.onLongPress(message)}
      delayLongPress={350}
      accessibilityHint="Long press for actions"
      style={({ pressed }) => ({ backgroundColor: highlighted ? alpha(colors.primary, 0.12) : pressed ? alpha(colors.muted, 0.5) : 'transparent', marginTop: groupStart ? 8 : 0 })}
    >
      {message.reply_to ? <Quote author={handlers.nameOf(message.reply_to.author_id)} body={message.reply_to.sticker && !message.reply_to.body ? 'Sticker' : handlers.text(message.reply_to.body)} /> : null}
      <View style={{ flexDirection: 'row', gap: 8, paddingHorizontal: 12, paddingTop: groupStart ? 4 : 2, paddingBottom: 2 }}>
        <View style={{ width: 30, paddingTop: 2 }}>{groupStart ? <Avatar id={message.author_id} name={author} size={30} /> : null}</View>
        <View style={{ flex: 1, minWidth: 0 }}>
          {groupStart ? (
            <View style={{ flexDirection: 'row', alignItems: 'baseline', gap: 8 }}>
              <Text numberOfLines={1} style={[type(14, 'medium'), { lineHeight: 20, flexShrink: 1 }]}>
                {author}
              </Text>
              <Text style={[type(12), { lineHeight: 16, color: colors.mutedForeground }]}>{timeOfDay(message.created_at)}</Text>
              {message.pinned ? <Icon name="pinTack" size={12} color={colors.mutedForeground} /> : null}
            </View>
          ) : null}
          {message.forwarded ? (
            <View style={{ flexDirection: 'row', alignItems: 'center', gap: 4 }}>
              <Icon name="forward" size={12} color={colors.mutedForeground} />
              <Text style={[type(12), { lineHeight: 16, color: colors.mutedForeground }]}>Forwarded from {handlers.nameOf(message.forwarded.author_id)}</Text>
            </View>
          ) : null}
          {body ? (
            <Text style={[type(14), { lineHeight: 21, color: message.deleted ? colors.mutedForeground : alpha(colors.foreground, 0.85) }]}>
              {body}
              {message.edited_at && !message.deleted ? <Text style={[type(11), { color: colors.mutedForeground }]}> (edited)</Text> : null}
            </Text>
          ) : null}
          {message.sticker ? <Sticker url={message.sticker.url} name={message.sticker.name} /> : message.sticker_id && !message.deleted ? <Text style={[type(12), { color: colors.mutedForeground }]}>Sticker was deleted</Text> : null}
          {!message.deleted && message.attachments.length > 0 ? <Attachments files={message.attachments} hasText={body !== ''} /> : null}
          {!message.deleted && message.reactions.length > 0 ? (
            <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 4, marginTop: 4 }}>
              {message.reactions.map((reaction) => {
                const own = hasReaction(message, reaction.emoji, user.id)
                return (
                  <Pressable
                    key={reaction.emoji}
                    accessibilityRole="button"
                    accessibilityState={{ selected: own }}
                    accessibilityLabel={`${reaction.emoji} ${reaction.user_ids.length}, ${reaction.user_ids.map(handlers.nameOf).join(', ')} reacted`}
                    onPress={() => handlers.onReact(message, reaction.emoji)}
                    onLongPress={() => handlers.onLongPress(message)}
                    style={({ pressed }) => ({
                      height: 26,
                      minWidth: 44,
                      flexDirection: 'row',
                      alignItems: 'center',
                      justifyContent: 'center',
                      gap: 4,
                      paddingHorizontal: 8,
                      borderRadius: 999,
                      borderWidth: 1,
                      borderColor: own ? alpha(colors.primary, 0.4) : colors.border,
                      backgroundColor: own ? alpha(colors.primary, 0.1) : alpha(colors.muted, 0.5),
                      transform: [{ scale: pressed ? 0.97 : 1 }],
                    })}
                  >
                    <Emoji value={reaction.emoji} size={16} />
                    <Text style={[type(12, 'medium'), { lineHeight: 16, color: own ? colors.foreground : colors.mutedForeground }]}>{reaction.user_ids.length}</Text>
                  </Pressable>
                )
              })}
              <Pressable
                accessibilityRole="button"
                accessibilityLabel="Add reaction"
                onPress={() => handlers.onAddReaction(message)}
                style={({ pressed }) => ({ height: 26, width: 34, alignItems: 'center', justifyContent: 'center', borderRadius: 999, borderWidth: 1, borderColor: colors.border, backgroundColor: pressed ? colors.muted : 'transparent' })}
              >
                <Icon name="smile" size={14} color={colors.mutedForeground} />
              </Pressable>
            </View>
          ) : null}
          {message.reply_count > 0 && handlers.onOpenThread ? <ReplySummary message={message} handlers={handlers} /> : null}
        </View>
      </View>
    </Pressable>
  )
}

/** The web `MessageQuote`: a bent guide from the avatar column to the quoted author and the start of their text. */
function Quote({ author, body }: { author: string; body: string }) {
  const { colors } = useTheme()
  return (
    <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8, paddingHorizontal: 12, paddingTop: 4 }}>
      <View style={{ width: 30, alignItems: 'flex-end', alignSelf: 'flex-end' }}>
        <View style={{ width: 19, height: 10, marginRight: -4, borderTopLeftRadius: 6, borderTopWidth: 2, borderLeftWidth: 2, borderColor: colors.border }} />
      </View>
      <Text numberOfLines={1} style={[type(12), { flex: 1, lineHeight: 20, color: colors.mutedForeground }]}>
        <Text style={[type(12, 'medium'), { color: alpha(colors.foreground, 0.9) }]}>{author} </Text>
        {body}
      </Text>
    </View>
  )
}

/** "2 replies · last reply 5m ago" with the last reply's preview, behind a 2px guide line. Opens the thread. */
function ReplySummary({ message, handlers }: { message: MessageRecord; handlers: MessageHandlers }) {
  const { colors } = useTheme()
  const reply = message.last_reply
  const replies = `${message.reply_count} ${message.reply_count === 1 ? 'reply' : 'replies'}`
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`Open thread, ${replies}`}
      onPress={() => handlers.onOpenThread?.(message)}
      style={({ pressed }) => ({ marginTop: 6, marginLeft: -4, paddingVertical: 4, paddingLeft: 4, paddingRight: 8, borderRadius: 8, backgroundColor: pressed ? colors.muted : 'transparent' })}
    >
      <View style={{ borderLeftWidth: 2, borderLeftColor: colors.border, paddingLeft: 10, gap: 4 }}>
        {reply ? (
          <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6, minWidth: 0 }}>
            <Avatar id={reply.author_id} name={handlers.nameOf(reply.author_id)} size={16} />
            <Text numberOfLines={1} style={[type(12, 'medium'), { lineHeight: 16, flexShrink: 0, color: alpha(colors.foreground, 0.9) }]}>
              {handlers.nameOf(reply.author_id)}
            </Text>
            <Text numberOfLines={1} style={[type(13), { lineHeight: 17, flexShrink: 1, color: colors.mutedForeground }]}>
              {reply.sticker ? 'Sticker' : handlers.text(reply.body)}
            </Text>
          </View>
        ) : null}
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}>
          <Text style={[type(12, 'medium'), { lineHeight: 16, color: alpha(colors.foreground, 0.9) }]}>{replies}</Text>
          {message.last_reply_at ? <Text style={[type(12), { lineHeight: 16, color: colors.mutedForeground }]}>last reply {relativeAgo(message.last_reply_at)}</Text> : null}
        </View>
      </View>
    </Pressable>
  )
}

function Sticker({ url, name }: { url: string; name: string }) {
  const image = useServerImage()
  return <Image accessibilityLabel={`Sticker ${name}`} source={image(url)} style={{ width: 120, height: 120, marginTop: 4 }} resizeMode="contain" />
}

/** The web `Attachments` at phone size: images in a grid (one alone up to 300px high), other files as cards. */
export function Attachments({ files, hasText }: { files: readonly ChatFileRecord[]; hasText: boolean }) {
  const { colors } = useTheme()
  const image = useServerImage()
  const { width: screen } = useWindowDimensions()
  const [viewing, setViewing] = useState<ChatFileRecord | null>(null)
  const images = files.filter(isImage)
  const others = files.filter((file) => !isImage(file))
  const maxWidth = Math.min(screen - 12 - 30 - 8 - 12, 512)
  const single = images.length === 1 ? images[0] : undefined
  const singleSize = single?.width && single.height ? fit(single.width, single.height, maxWidth, 300) : { width: maxWidth, height: 200 }
  const columns = images.length === 3 ? 3 : 2
  const cell = (maxWidth - (columns - 1) * 4) / columns

  return (
    <View style={{ gap: 8, marginTop: hasText ? 8 : 4 }}>
      {images.length > 0 ? (
        <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 4, maxWidth }}>
          {images.slice(0, 4).map((file, index) => (
            <Pressable key={file.id} accessibilityRole="imagebutton" accessibilityLabel={`Open image ${file.file_name}`} onPress={() => setViewing(file)}>
              <Image
                source={image(file.url)}
                style={[single ? singleSize : { width: cell, height: cell }, { borderRadius: 8, backgroundColor: colors.muted }]}
                resizeMode={single ? 'contain' : 'cover'}
              />
              {index === 3 && images.length > 4 ? (
                <View style={{ position: 'absolute', inset: 0, borderRadius: 8, alignItems: 'center', justifyContent: 'center', backgroundColor: alpha(cssColors.black, 0.6) }}>
                  <Text style={[type(18, 'semibold'), { color: cssColors.white }]}>+{images.length - 4}</Text>
                </View>
              ) : null}
            </Pressable>
          ))}
        </View>
      ) : null}
      {others.map((file) => (
        <FileCard key={file.id} file={file} />
      ))}
      <ImageViewer file={viewing} onClose={() => setViewing(null)} />
    </View>
  )
}

const fit = (width: number, height: number, maxWidth: number, maxHeight: number) => {
  const scale = Math.min(1, maxWidth / width, maxHeight / height)
  return { width: Math.round(width * scale), height: Math.round(height * scale) }
}

/**
 * Downloads a chat file with the session's token into the cache and hands it to the system preview (Quick Look on
 * iOS, the app for its type on Android), where it can be saved or shared.
 */
function useOpenFile() {
  const { session } = useOrbit()
  const image = useServerImage()
  const [opening, setOpening] = useState<string | null>(null)
  const open = async (file: ChatFileRecord) => {
    if (Platform.OS === 'web') return void window.open(image(file.url).uri, '_blank')
    setOpening(file.id)
    try {
      const target = new File(Paths.cache, `chat-${file.id}-${file.file_name.replace(/[/\\]/g, '_')}`)
      const local = target.exists ? target : await File.downloadFileAsync(image(file.url).uri, target, { headers: { Authorization: `Bearer ${session.token}` }, idempotent: true })
      if (await local.canPreview()) await local.preview()
      else await Share.share({ url: local.uri, title: file.file_name })
    } catch (error) {
      showError(`Could not open ${file.file_name}`, error)
    } finally {
      setOpening(null)
    }
  }
  return { open, opening }
}

function FileCard({ file }: { file: ChatFileRecord }) {
  const { colors } = useTheme()
  const { open, opening } = useOpenFile()
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`Open ${file.file_name}, ${fileSize(file.size_bytes)}`}
      onPress={() => void open(file)}
      style={({ pressed }) => ({
        maxWidth: 448,
        flexDirection: 'row',
        alignItems: 'center',
        gap: 12,
        padding: 12,
        borderRadius: 10,
        borderWidth: 1,
        borderColor: colors.border,
        backgroundColor: pressed ? colors.muted : colors.background,
      })}
    >
      <Icon name="attach" size={28} color={colors.mutedForeground} />
      <View style={{ flex: 1, minWidth: 0 }}>
        <Text numberOfLines={1} style={[type(14, 'medium'), { lineHeight: 20 }]}>
          {file.file_name}
        </Text>
        <Text style={[type(12), { lineHeight: 16, color: colors.mutedForeground }]}>{fileSize(file.size_bytes)}</Text>
      </View>
      {opening === file.id ? <Spinner /> : <Icon name="download" size={20} color={colors.mutedForeground} />}
    </Pressable>
  )
}

/** An image of a message over the whole screen, with its name, Close and Open (save or share). */
function ImageViewer({ file, onClose }: { file: ChatFileRecord | null; onClose: () => void }) {
  const image = useServerImage()
  const { open, opening } = useOpenFile()
  // Read outside the Modal: a SafeAreaView inside a full-screen iOS Modal measures no insets, and the bar sat under the status bar.
  const insets = useSafeAreaInsets()
  return (
    <Modal visible={file !== null} animationType="fade" onRequestClose={onClose} supportedOrientations={['portrait', 'landscape']}>
      <View style={{ flex: 1, backgroundColor: cssColors.black, paddingTop: insets.top, paddingBottom: insets.bottom, paddingLeft: insets.left, paddingRight: insets.right }}>
        <View style={{ height: 48, flexDirection: 'row', alignItems: 'center', gap: 8, paddingHorizontal: 8 }}>
          <IconButton icon="x" label="Close" size={36} iconSize={22} color={cssColors.white} onPress={onClose} />
          <Text numberOfLines={1} style={[type(14, 'medium'), { flex: 1, color: cssColors.white }]}>
            {file?.file_name}
          </Text>
          {file ? opening ? <Spinner color={cssColors.white} /> : <IconButton icon="download" label="Open or save" size={36} iconSize={22} color={cssColors.white} onPress={() => void open(file)} /> : null}
        </View>
        {file ? <Image accessibilityLabel={file.file_name} source={image(file.url)} style={{ flex: 1 }} resizeMode="contain" /> : null}
      </View>
    </Modal>
  )
}
