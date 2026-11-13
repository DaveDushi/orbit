import * as DocumentPicker from 'expo-document-picker'
import { useState } from 'react'
import { Image, View } from 'react-native'

import { Emoji } from '../../../components/chat/EmojiPicker'
import { ConfirmDialog, type ConfirmRequest } from '../../../components/settings/Dialog'
import { Note, RowText, SettingsCard, SettingsRow, SettingsShell, WORKSPACE_NAV } from '../../../components/settings/Shell'
import { EMOJI_IMAGE_TYPES, EMOJI_LIMIT, emojiNameFromFile, emojiProblem, emojiServerProblem } from '../../../data/emoji'
import { shortDate } from '../../../data/format'
import { useEmojiGroups } from '../../../hooks/chat'
import { useCustomEmojiList, useEmojiWrites } from '../../../hooks/settings'
import { useOrbit } from '../../../orbit-context'
import { useMembers } from '../../../queries'
import { uploadPart } from '../../../upload-part'
import { Button, Input, Text } from '../../../ui'

interface Picked {
  uri: string
  name: string
  type: string
  size: number | null
}

/**
 * The web's Settings → Emoji: the workspace's custom emoji, to add one from an image or delete one. Owners and admins
 * only (`chat.manage`); the nav hides it from the others and the server refuses them anyway.
 */
export default function EmojiScreen() {
  const { workspace } = useOrbit()
  // a standard emoji keeps its name (`:joy:`)
  const standard = useEmojiGroups()
  const canManage = workspace.permissions.includes('chat.manage')
  const emoji = useCustomEmojiList(canManage)
  const members = useMembers()
  const { create, remove } = useEmojiWrites()
  const [file, setFile] = useState<Picked | null>(null)
  const [name, setName] = useState('')
  const [problem, setProblem] = useState<string | null>(null)
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null)
  const memberNames = new Map((members.data ?? []).map((member) => [member.user_id, member.display_name]))
  const list = emoji.data ?? []

  const choose = async () => {
    const result = await DocumentPicker.getDocumentAsync({ type: EMOJI_IMAGE_TYPES, copyToCacheDirectory: true })
    const asset = result.canceled ? undefined : result.assets[0]
    if (!asset) return
    setFile({ uri: asset.uri, name: asset.name, type: asset.mimeType ?? '', size: asset.size ?? null })
    setName(emojiNameFromFile(asset.name))
    setProblem(null)
  }

  const submit = async () => {
    if (!file || create.isPending) return
    const refused = emojiProblem(file, name, (candidate) => list.some((entry) => entry.name === candidate) || standard.some((group) => group.emojis.some((item) => item.names.includes(candidate))))
    setProblem(refused)
    if (refused) return
    try {
      await create.mutateAsync({ name, file: await uploadPart(file.uri, file.name) })
      setFile(null)
      setName('')
    } catch (error) {
      setProblem(emojiServerProblem(error, 'The emoji could not be added.'))
    }
  }

  return (
    <SettingsShell title="Settings" nav={WORKSPACE_NAV} refreshing={emoji.isRefetching} onRefresh={canManage ? () => void emoji.refetch() : undefined}>
      {!canManage ? (
        <SettingsCard title="Emoji">
          <Note>Only workspace owners and administrators can manage custom emoji.</Note>
        </SettingsCard>
      ) : (
        <>
          <SettingsCard title="Add emoji" description="Custom emoji are for everyone in this workspace: in chat messages and as reactions.">
            <View className="gap-4">
              <View className="gap-2">
                <Text className="font-noto-medium text-[14px]">Image</Text>
                <View className="flex-row items-center gap-3">
                  {file ? <Image accessibilityLabel="Chosen image" source={{ uri: file.uri }} style={{ width: 32, height: 32 }} resizeMode="contain" /> : null}
                  <Button label={file ? 'Choose another' : 'Choose image'} variant="outline" size="sm" onPress={() => void choose()} />
                </View>
                <Text className="text-[12px] text-muted-foreground">PNG, JPEG, WebP or GIF, at most 256 KB.</Text>
              </View>
              <Input
                label="Name"
                required
                value={name}
                maxLength={32}
                placeholder="party_parrot"
                autoCapitalize="none"
                autoCorrect={false}
                onChangeText={(text) => {
                  setName(text.toLowerCase())
                  setProblem(null)
                }}
                description={`Lower case letters, digits and underscores. In a message, type :${name || 'name'}:`}
              />
              <View className="flex-row flex-wrap items-center gap-3">
                <Button label="Add emoji" loading={create.isPending} disabled={!file || !name} onPress={() => void submit()} />
              </View>
              {problem ? <Note error>{problem}</Note> : null}
            </View>
          </SettingsCard>

          <SettingsCard title="Emoji" description={emoji.data ? `${list.length} of ${EMOJI_LIMIT} in this workspace.` : undefined} flush>
            {emoji.isPending ? <View className="p-4"><Note loading>Loading emoji…</Note></View> : null}
            {emoji.isError ? <View className="p-4"><Note error>Emoji could not be loaded.</Note></View> : null}
            {emoji.isSuccess && list.length === 0 ? <View className="p-4"><Note>No custom emoji yet. Add an image above. Everyone in the workspace can then use it in chat.</Note></View> : null}
            {list.map((entry, index) => (
              <SettingsRow key={entry.id} first={index === 0}>
                <View className="size-9 rounded-md bg-muted items-center justify-center">
                  <Emoji value={`:${entry.name}:`} size={24} />
                </View>
                <RowText title={`:${entry.name}:`} subtitle={`Added by ${memberNames.get(entry.created_by) ?? 'a former member'} · ${shortDate(entry.created_at)}`} />
                <Button
                  label="Delete"
                  variant="ghost"
                  size="sm"
                  disabled={remove.isPending}
                  onPress={() =>
                    setConfirm({
                      title: `Delete :${entry.name}:?`,
                      description: 'Messages and reactions that use it show its name as text. You cannot undo this.',
                      confirmLabel: 'Delete',
                      danger: true,
                      onConfirm: () => remove.mutate(entry.id),
                    })
                  }
                />
              </SettingsRow>
            ))}
            {remove.isError ? <View className="p-4"><Note error>{emojiServerProblem(remove.error, 'The emoji could not be deleted.')}</Note></View> : null}
          </SettingsCard>
          <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
        </>
      )}
    </SettingsShell>
  )
}
