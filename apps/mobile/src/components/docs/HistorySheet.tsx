import { useQuery } from '@tanstack/react-query'
import { useEffect, useState } from 'react'
import { ScrollView, View } from 'react-native'

import { blockLines } from '../../data/docs'
import { pageVersion, pageVersions, type PageVersionSummary } from '../../data/docs-api'
import { keys } from '../../data/keys'
import { useOrbit } from '../../orbit-context'
import { errorMessage } from '../../session-context'
import { Button, Spinner, Text } from '../../ui'
import { Sheet, SheetList, SheetNote, SheetRow } from '../tasks/Sheet'

const KIND: Record<PageVersionSummary['kind'], string> = { auto: 'Edited', restore: 'Before a restore', import: 'Imported' }

const when = (iso: string) => new Date(iso).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })

/**
 * The web's Version history: the page's saved versions, newest first. A tap shows the version's text; "Restore" puts it
 * back on the page (the page as it is now is kept as a version too). Hidden while `pageId` is `null`.
 */
export function HistorySheet({ pageId, locked, onRestore, onClose }: { pageId: string | null; locked: boolean; onRestore: (version: PageVersionSummary) => Promise<unknown>; onClose: () => void }) {
  const { client, workspaceId } = useOrbit()
  const [chosen, setChosen] = useState<PageVersionSummary | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    setChosen(null)
    setError(null)
    setBusy(false)
  }, [pageId])
  const base = [...keys.page(workspaceId, pageId ?? ''), 'versions'] as const
  const list = useQuery({ queryKey: base, queryFn: () => pageVersions(client, workspaceId, pageId ?? ''), enabled: pageId !== null })
  // versions never change
  const detail = useQuery({ queryKey: [...base, chosen?.id ?? ''], queryFn: () => pageVersion(client, workspaceId, pageId ?? '', chosen?.id ?? ''), enabled: chosen !== null, staleTime: Infinity })
  const restore = async () => {
    if (!chosen) return
    setBusy(true)
    setError(null)
    try {
      await onRestore(chosen)
      onClose()
    } catch (reason) {
      setError(errorMessage(reason))
      setBusy(false)
    }
  }
  const lines = detail.data ? blockLines(detail.data.content) : []
  return (
    <Sheet visible={pageId !== null} title={chosen ? `${detail.data?.title || chosen.title || 'Untitled'} · ${when(chosen.created_at)}` : 'Version history'} onClose={onClose}>
      {chosen ? (
        <>
          <ScrollView className="shrink px-4" contentContainerClassName="pb-3 gap-1">
            {!detail.data ? (
              detail.error ? <Text className="text-[13px] text-destructive">{errorMessage(detail.error)}</Text> : <View className="py-4 items-center"><Spinner /></View>
            ) : lines.length === 0 ? (
              <Text className="text-[13px] text-muted-foreground">This version has no text.</Text>
            ) : (
              lines.map((line, index) => (
                <Text key={index} className="text-[14px] leading-[20px]">
                  {line}
                </Text>
              ))
            )}
          </ScrollView>
          {error ? <SheetNote tone="destructive">{error}</SheetNote> : null}
          {locked ? <SheetNote>Unlock the page to restore a version.</SheetNote> : null}
          <View className="flex-row justify-end gap-2 px-4 pt-2">
            <Button label="Back" variant="outline" onPress={() => setChosen(null)} />
            <Button label="Restore this version" icon="rotateLeft" loading={busy} disabled={busy || locked || !detail.data} onPress={() => void restore()} />
          </View>
        </>
      ) : (
        <SheetList>
          {!list.data ? (
            list.error ? <SheetNote tone="destructive">{errorMessage(list.error)}</SheetNote> : <View className="py-4 items-center"><Spinner /></View>
          ) : list.data.length === 0 ? (
            <SheetNote>No versions yet. Orbit keeps one as the page changes.</SheetNote>
          ) : (
            list.data.map((version) => (
              <SheetRow
                key={version.id}
                label={when(version.created_at)}
                subtitle={[KIND[version.kind], version.created_by?.display_name].filter(Boolean).join(' · ')}
                icon="history"
                onPress={() => setChosen(version)}
              />
            ))
          )}
        </SheetList>
      )}
    </Sheet>
  )
}
