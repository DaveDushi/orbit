import { listChatStickers } from '@orbit/api/generated/sdk.gen'
import type { CustomStickerRecord } from '@orbit/api/generated/types.gen'
import { useQuery } from '@tanstack/react-query'
import { useState } from 'react'
import { Image, Pressable, useWindowDimensions, View } from 'react-native'

import { useServerImage } from '../../hooks/chat'
import { useOrbit } from '../../orbit-context'
import { Input, Text } from '../../ui'
import { FormSheet } from './Sheet'

/** The workspace's stickers (Settings → Stickers on the web). The list changes rarely: an hour before it asks again. */
export function useStickers(enabled = true) {
  const { client, workspaceId } = useOrbit()
  return useQuery({
    queryKey: ['chat-stickers', workspaceId],
    queryFn: async () => (await listChatStickers({ client, path: { workspace_id: workspaceId }, throwOnError: true })).data,
    staleTime: 60 * 60_000,
    enabled,
  })
}

/** The web's sticker popover as a sheet: a search field and a grid of four. A tap sends the sticker at once. */
export function StickerPicker({ stickers, onPick, onClose }: { stickers: readonly CustomStickerRecord[]; onPick: (sticker: CustomStickerRecord) => void; onClose: () => void }) {
  const image = useServerImage()
  const { width } = useWindowDimensions()
  const [query, setQuery] = useState('')
  const text = query.trim().toLowerCase()
  const shown = stickers.filter((sticker) => !text || sticker.name.toLowerCase().includes(text))
  const cell = Math.floor((Math.min(width, 560) - 32 - 3 * 4) / 4)
  return (
    <FormSheet visible title="Stickers" onClose={onClose}>
      <Input placeholder="Search stickers" value={query} onChangeText={setQuery} autoCapitalize="none" autoCorrect={false} accessibilityLabel="Search stickers" />
      {shown.length === 0 ? <Text className="text-[14px] text-muted-foreground">No stickers match</Text> : null}
      <View className="flex-row flex-wrap gap-1">
        {shown.map((sticker) => (
          <Pressable
            key={sticker.id}
            accessibilityRole="button"
            accessibilityLabel={`Send sticker ${sticker.name}`}
            onPress={() => onPick(sticker)}
            className="items-center justify-center rounded-md p-1.5 active:bg-muted"
            style={{ width: cell, height: cell }}
          >
            <Image source={image(sticker.url)} className="size-full" resizeMode="contain" />
          </Pressable>
        ))}
      </View>
    </FormSheet>
  )
}
