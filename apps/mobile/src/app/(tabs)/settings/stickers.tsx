import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import * as DocumentPicker from 'expo-document-picker'
import { useState } from 'react'
import { Image, View } from 'react-native'

import { ConfirmDialog, type ConfirmRequest } from '../../../components/settings/Dialog'
import { Note, RowText, SettingsCard, SettingsRow, SettingsShell, WORKSPACE_NAV } from '../../../components/settings/Shell'
import { serverHeaders } from '../../../data/chat-api'
import { absoluteUrl } from '../../../data/docs'
import { shortDate } from '../../../data/format'
import { addSticker, removeSticker, STICKER_IMAGE_TYPES, STICKER_LIMIT, STICKER_NAME_MAX, stickerNameFromFile, stickerProblem, stickers as listStickers, stickerServerProblem } from '../../../data/stickers'
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
 * The web's Settings → Stickers: the workspace's stickers, to add one from an image or delete one. Owners and admins
 * only (`chat.manage`).
 */
export default function StickersScreen() {
  const { client, session, workspace, workspaceId } = useOrbit()
  const canManage = workspace.permissions.includes('chat.manage')
  const queryClient = useQueryClient()
  const key = ['custom-stickers', workspaceId]
  const list = useQuery({ queryKey: key, queryFn: () => listStickers(client, workspaceId), enabled: canManage })
  const refresh = () => void queryClient.invalidateQueries({ queryKey: key })
  const create = useMutation({ mutationFn: async ({ name, file }: { name: string; file: Picked }) => addSticker(client, workspaceId, name, await uploadPart(file.uri, file.name)), onSuccess: refresh })
  const remove = useMutation({ mutationFn: (id: string) => removeSticker(client, workspaceId, id), onSettled: refresh })
  const members = useMembers()
  const [file, setFile] = useState<Picked | null>(null)
  const [name, setName] = useState('')
  const [problem, setProblem] = useState<string | null>(null)
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null)
  const memberNames = new Map((members.data ?? []).map((member) => [member.user_id, member.display_name]))
  const items = list.data ?? []

  const choose = async () => {
    const result = await DocumentPicker.getDocumentAsync({ type: STICKER_IMAGE_TYPES, copyToCacheDirectory: true })
    const asset = result.canceled ? undefined : result.assets[0]
    if (!asset) return
    setFile({ uri: asset.uri, name: asset.name, type: asset.mimeType ?? '', size: asset.size ?? null })
    setName(stickerNameFromFile(asset.name))
    setProblem(null)
  }

  const submit = async () => {
    if (!file || create.isPending) return
    const refused = stickerProblem(file, name, items.map((entry) => entry.name))
    setProblem(refused)
    if (refused) return
    try {
      await create.mutateAsync({ name: name.trim(), file })
      setFile(null)
      setName('')
    } catch (error) {
      setProblem(stickerServerProblem(error, 'The sticker could not be added.'))
    }
  }

  return (
    <SettingsShell title="Settings" nav={WORKSPACE_NAV} refreshing={list.isRefetching} onRefresh={canManage ? () => void list.refetch() : undefined}>
      {!canManage ? (
        <SettingsCard title="Stickers">
          <Note>Only workspace owners and administrators can manage stickers.</Note>
        </SettingsCard>
      ) : (
        <>
          <SettingsCard title="Add sticker" description="Stickers are for everyone in this workspace: a large image that is sent as a chat message.">
            <View className="gap-4">
              <View className="gap-2">
                <Text className="font-noto-medium text-[14px]">Image</Text>
                <View className="flex-row items-center gap-3">
                  {file ? <Image accessibilityLabel="Chosen image" source={{ uri: file.uri }} style={{ width: 48, height: 48 }} resizeMode="contain" /> : null}
                  <Button label={file ? 'Choose another' : 'Choose image'} variant="outline" size="sm" onPress={() => void choose()} />
                </View>
                <Text className="text-[12px] text-muted-foreground">PNG, JPEG, WebP or GIF, at most 512 KB.</Text>
              </View>
              <Input
                label="Name"
                required
                value={name}
                maxLength={STICKER_NAME_MAX}
                placeholder="Party Parrot"
                onChangeText={(text) => {
                  setName(text)
                  setProblem(null)
                }}
                description="2 to 30 characters. People find the sticker by this name."
              />
              <View className="flex-row">
                <Button label="Add sticker" loading={create.isPending} disabled={!file || !name.trim()} onPress={() => void submit()} />
              </View>
              {problem ? <Note error>{problem}</Note> : null}
            </View>
          </SettingsCard>

          <SettingsCard title="Stickers" description={list.data ? `${items.length} of ${STICKER_LIMIT} in this workspace.` : undefined} flush>
            {list.isPending ? <View className="p-4"><Note loading>Loading stickers…</Note></View> : null}
            {list.isError ? <View className="p-4"><Note error>Stickers could not be loaded.</Note></View> : null}
            {list.isSuccess && items.length === 0 ? <View className="p-4"><Note>No stickers yet. Add an image above. Everyone in the workspace can then send it in chat.</Note></View> : null}
            {items.map((entry, index) => {
              const uri = absoluteUrl(session.server, entry.url)
              return (
                <SettingsRow key={entry.id} first={index === 0}>
                  <Image accessibilityIgnoresInvertColors source={{ uri, headers: serverHeaders(session.server, uri, session.token) }} style={{ width: 56, height: 56 }} resizeMode="contain" />
                  <RowText title={entry.name} subtitle={`Added by ${memberNames.get(entry.created_by) ?? 'a former member'} · ${shortDate(entry.created_at)}`} />
                  <Button
                    label="Delete"
                    variant="ghost"
                    size="sm"
                    disabled={remove.isPending}
                    onPress={() =>
                      setConfirm({ title: `Delete ${entry.name}?`, description: 'Messages that were sent with it say that the sticker was deleted. You cannot undo this.', confirmLabel: 'Delete', danger: true, onConfirm: () => remove.mutate(entry.id) })
                    }
                  />
                </SettingsRow>
              )
            })}
            {remove.isError ? <View className="p-4"><Note error>{stickerServerProblem(remove.error, 'The sticker could not be deleted.')}</Note></View> : null}
          </SettingsCard>
          <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
        </>
      )}
    </SettingsShell>
  )
}
