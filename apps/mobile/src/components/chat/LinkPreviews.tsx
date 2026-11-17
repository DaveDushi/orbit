import { useQuery } from '@tanstack/react-query'
import { Image, Linking, Pressable, View } from 'react-native'

import { linkCardUrls } from '../../data/chat'
import { type LinkPreview, linkPreview } from '../../data/chat-api'
import { useOrbit } from '../../orbit-context'
import { Text } from '../../ui'

/**
 * The web's link cards under a message: for each of another site's URLs, its preview (site, title, description and
 * image; a bare image shows alone). Nothing shows while the server reads the page, nor for a URL without a preview. A
 * tap opens the URL.
 */
export function LinkPreviews({ body }: { body: string }) {
  const { session } = useOrbit()
  const urls = linkCardUrls(body, session.server)
  if (urls.length === 0) return null
  return (
    <View className="mt-1.5 gap-1.5">
      {urls.map((url) => (
        <LinkCard key={url} url={url} />
      ))}
    </View>
  )
}

function LinkCard({ url }: { url: string }) {
  const { client, workspaceId } = useOrbit()
  const preview = useQuery({ queryKey: ['chat-link-preview', workspaceId, url], queryFn: () => linkPreview(client, workspaceId, url), staleTime: 60 * 60 * 1000, retry: false })
  const data = preview.data
  if (!data) return null
  const open = () => void Linking.openURL(data.url).catch(() => {})
  if (data.kind === 'image') {
    return data.image_url ? (
      <Pressable accessibilityRole="link" accessibilityLabel={data.url} onPress={open}>
        <Image source={{ uri: data.image_url }} className="w-full h-48 rounded-lg bg-muted" resizeMode="cover" />
      </Pressable>
    ) : null
  }
  return (
    <Pressable accessibilityRole="link" accessibilityLabel={data.title ?? data.url} onPress={open} className="max-w-md overflow-hidden rounded-lg border border-border bg-card active:bg-muted/50">
      {data.image_url && (data.large_image || data.kind !== 'link') ? <Image source={{ uri: data.image_url }} className="w-full aspect-[1.91] border-b border-border bg-muted" resizeMode="cover" /> : null}
      <View className="flex-row items-start gap-3 p-2.5">
        <View className="flex-1 min-w-0 gap-0.5">
          <Text numberOfLines={1} className="text-[12px] text-muted-foreground">{siteOf(data)}</Text>
          {data.title ? <Text numberOfLines={2} className="font-noto-medium text-[13px]">{data.title}</Text> : null}
          {data.description ? <Text numberOfLines={2} className="text-[12px] leading-[17px] text-muted-foreground">{data.description}</Text> : null}
        </View>
        {data.image_url && !data.large_image && data.kind === 'link' ? <Image source={{ uri: data.image_url }} className="size-16 rounded-md bg-muted" resizeMode="cover" /> : null}
      </View>
    </Pressable>
  )
}

const siteOf = (preview: LinkPreview) =>
  preview.kind === 'youtube' ? ['YouTube', preview.author_name].filter(Boolean).join(' · ') : preview.kind === 'x' ? ['X', preview.author_name].filter(Boolean).join(' · ') : (preview.site_name ?? new URL(preview.url).host)
