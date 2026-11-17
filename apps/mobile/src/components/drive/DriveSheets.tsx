/**
 * The web's Drive Move and Share dialogs at phone size. Move: walk the folders and "Move here". Share: make a public
 * link (expiry, optional password), hand it to the iOS share sheet at once (Orbit shows the address only once), and
 * turn old links off.
 */
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect, useState } from 'react'
import { Pressable, Share, View } from 'react-native'

import { keys } from '../../data/keys'
import { driveChildren, type DriveNode, moveNode, moveTargets, newShare, revokeShare, shareExpiry, shares, shareState } from '../../data/drive'
import { relativeAgo } from '../../data/format'
import { useOrbit } from '../../orbit-context'
import { errorMessage } from '../../session-context'
import { useTheme } from '../../theme'
import { Button, Icon, Input, Spinner, Text } from '../../ui'
import { Sheet, SheetList, SheetNote, SheetRow } from '../tasks/Sheet'

type Crumb = { id: string | null; name: string }

/** Moves `node` into the folder the user walks to. `node: null` hides the sheet. */
export function MoveSheet({ node, onClose, onMoved }: { node: DriveNode | null; onClose: () => void; onMoved: (target: string) => void }) {
  const { colors } = useTheme()
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  const [path, setPath] = useState<Crumb[]>([{ id: null, name: 'Drive' }])
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  useEffect(() => {
    setPath([{ id: null, name: 'Drive' }])
    setError(null)
    setBusy(false)
  }, [node])
  const here = path[path.length - 1]
  const folders = useQuery({
    queryKey: keys.driveChildren(workspaceId, here.id),
    queryFn: () => driveChildren(client, workspaceId, here.id),
    enabled: node !== null,
  })
  const same = node !== null && (node.parent_id ?? null) === here.id
  const move = async () => {
    if (!node || busy) return
    setBusy(true)
    setError(null)
    try {
      await moveNode(client, workspaceId, node.id, here.id)
      void queryClient.invalidateQueries({ queryKey: keys.drive(workspaceId) })
      onMoved(here.name)
      onClose()
    } catch (reason) {
      setError(errorMessage(reason))
      setBusy(false)
    }
  }
  return (
    <Sheet visible={node !== null} title={node ? `Move “${node.name}”` : ''} onClose={onClose}>
      <View className="flex-row items-center gap-1 px-4 pb-2 flex-wrap">
        {path.map((crumb, index) => (
          <Pressable key={crumb.id ?? 'top'} accessibilityRole="button" disabled={index === path.length - 1} onPress={() => setPath(path.slice(0, index + 1))} className="flex-row items-center gap-1">
            {index > 0 ? <Icon name="chevronRight" size={12} color={colors.mutedForeground} /> : null}
            <Text className={`text-[13px] ${index === path.length - 1 ? 'font-noto-semibold text-foreground' : 'text-muted-foreground'}`}>{crumb.name}</Text>
          </Pressable>
        ))}
      </View>
      <SheetList>
        {!folders.data ? (
          folders.error ? <SheetNote tone="destructive">{errorMessage(folders.error)}</SheetNote> : <View className="py-4 items-center"><Spinner /></View>
        ) : node && moveTargets(folders.data, node).length === 0 ? (
          <SheetNote>No folders here.</SheetNote>
        ) : (
          node && moveTargets(folders.data, node).map((folder) => <SheetRow key={folder.id} label={folder.name} icon="folder" onPress={() => setPath([...path, { id: folder.id, name: folder.name }])} />)
        )}
      </SheetList>
      {error ? <SheetNote tone="destructive">{error}</SheetNote> : null}
      <View className="flex-row justify-end gap-2 px-4 pt-2">
        <Button label="Cancel" variant="outline" onPress={onClose} />
        <Button label={same ? 'Already here' : `Move to ${here.name}`} disabled={same || busy} loading={busy} onPress={() => void move()} />
      </View>
    </Sheet>
  )
}

const EXPIRY: { label: string; days: number | null }[] = [
  { label: 'Never', days: null },
  { label: '1 day', days: 1 },
  { label: '7 days', days: 7 },
  { label: '30 days', days: 30 },
]

/** The public links of `node`. `node: null` hides the sheet. */
export function ShareSheet({ node, onClose }: { node: DriveNode | null; onClose: () => void }) {
  const { colors } = useTheme()
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  const sharesKey = [...keys.drive(workspaceId), 'shares', node?.id ?? ''] as const
  const list = useQuery({ queryKey: sharesKey, queryFn: () => shares(client, workspaceId, node?.id ?? ''), enabled: node !== null })
  const [days, setDays] = useState<number | null>(null)
  const [password, setPassword] = useState('')
  const [created, setCreated] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  useEffect(() => {
    setDays(null)
    setPassword('')
    setCreated(null)
    setError(null)
    setBusy(false)
  }, [node])
  const send = (url: string) => void Share.share({ message: url, url }).catch(() => undefined)
  const create = async () => {
    if (!node || busy) return
    if (password && password.length < 12) {
      setError('A password needs 12 or more characters.')
      return
    }
    setBusy(true)
    setError(null)
    try {
      const result = await newShare(client, workspaceId, node.id, { expires_at: shareExpiry(days), password: password || null })
      setCreated(result.url)
      setPassword('')
      void queryClient.invalidateQueries({ queryKey: sharesKey })
      send(result.url)
    } catch (reason) {
      setError(errorMessage(reason))
    } finally {
      setBusy(false)
    }
  }
  const revoke = async (shareId: string) => {
    try {
      await revokeShare(client, workspaceId, shareId)
    } catch (reason) {
      setError(errorMessage(reason))
    }
    void queryClient.invalidateQueries({ queryKey: sharesKey })
  }
  return (
    <Sheet visible={node !== null} title={node ? `Share “${node.name}”` : ''} onClose={onClose}>
      <SheetList>
        <Text className="text-[13px] px-4 pb-3 text-muted-foreground">Anyone with the link can open it, without an Orbit account.</Text>
        {created ? (
          <View className="mx-4 mb-3 p-3 gap-2 rounded-md border border-primary/40 bg-primary/5">
            <Text className="font-noto-medium text-[13px]">New link: share or copy it now. Orbit shows it only once.</Text>
            <Text selectable numberOfLines={2} className="text-[12px] text-muted-foreground">
              {created}
            </Text>
            <Button label="Share link" icon="link2" variant="outline" onPress={() => send(created)} />
          </View>
        ) : null}
        <View className="px-4 gap-2">
          <Text className="font-noto-medium text-[13px] text-muted-foreground">Expires</Text>
          <View className="flex-row flex-wrap gap-1.5">
            {EXPIRY.map((option) => (
              <Pressable
                key={option.label}
                accessibilityRole="radio"
                accessibilityState={{ checked: days === option.days }}
                onPress={() => setDays(option.days)}
                className={`h-8 px-3 rounded-full justify-center border ${days === option.days ? 'border-primary bg-primary/10' : 'border-border'}`}
              >
                <Text className="text-[13px]">{option.label}</Text>
              </Pressable>
            ))}
          </View>
          <Input label="Password (optional, 12+ characters)" value={password} onChangeText={setPassword} secureTextEntry autoCapitalize="none" autoComplete="new-password" />
          {error ? <Text className="text-[13px] text-destructive">{error}</Text> : null}
          <View className="flex-row justify-end">
            <Button label="Create link" icon="link2" loading={busy} disabled={busy} onPress={() => void create()} />
          </View>
        </View>
        <Text className="font-noto-medium text-[12px] px-4 pt-4 pb-1 text-muted-foreground">Links</Text>
        {!list.data ? (
          list.error ? <SheetNote tone="destructive">{errorMessage(list.error)}</SheetNote> : <View className="py-3 items-center"><Spinner /></View>
        ) : list.data.length === 0 ? (
          <SheetNote>No links yet.</SheetNote>
        ) : (
          list.data.map((share) => {
            const state = shareState(share)
            const detail = [
              `made ${relativeAgo(share.created_at)}`,
              share.expires_at ? `expires ${new Date(share.expires_at).toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}` : null,
              share.has_password ? 'password' : null,
              `${share.download_count} ${share.download_count === 1 ? 'download' : 'downloads'}`,
            ]
              .filter(Boolean)
              .join(' · ')
            return (
              <View key={share.id} className="flex-row items-center gap-3 px-4 py-2 border-t border-border">
                <Icon name={state.active ? 'link2' : 'linkBroken'} size={16} color={state.active ? colors.primary : colors.mutedForeground} />
                <View className="flex-1 min-w-0">
                  <Text className="font-noto-medium text-[13px]">{state.label}</Text>
                  <Text numberOfLines={1} className="text-[12px] text-muted-foreground">
                    {detail}
                  </Text>
                </View>
                {state.active && share.can_revoke ? <Button label="Turn off" size="sm" variant="outline" onPress={() => void revoke(share.id)} /> : null}
              </View>
            )
          })
        )}
      </SheetList>
    </Sheet>
  )
}
