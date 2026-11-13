import { useQuery, useQueryClient } from '@tanstack/react-query'
import { router, useFocusEffect } from 'expo-router'
import { useCallback, useState } from 'react'
import { RefreshControl, ScrollView, View } from 'react-native'

import { useAskDonnaAgent } from '../../components/AskDonna'
import { MenuButton } from '../../components/Drawer'
import { HomeCardBody, HomeCardFrame } from '../../components/home/HomeCards'
import { addableCards, addCard, CARD_LIMITS, cardLimit, cardTitle, HOME_CARDS, removeCard, setCardLimit, stepCard, visibleCards } from '../../data/home'
import { keys } from '../../data/keys'
import * as api from '../../data/orbit'
import { useHomeLayout, useSaveHomeLayout } from '../../hooks/home'
import { useIsGuest, useOrbit } from '../../orbit-context'
import { useUnreadNotifications } from '../../queries'
import { useTheme } from '../../theme'
import { Button, HeaderTitle, IconButton, PaneHeader, PickerSheet, SafeAreaView, Text } from '../../ui'

function greetingFor(hour: number): string {
  if (hour < 12) return 'Good morning'
  if (hour < 18) return 'Good afternoon'
  return 'Good evening'
}

/** Tasks due this week and unread notifications, under the greeting (members only: a guest has neither). */
function MemberSummary() {
  const { client, workspaceId } = useOrbit()
  const week = useQuery({
    queryKey: keys.taskView(workspaceId, 'home-week'),
    queryFn: () => api.taskPage(client, workspaceId, { view: 'my_week', sort: 'due_date', order: 'asc', limit: 7 }),
  })
  const unread = useUnreadNotifications().data ?? []
  const weekCount = `${week.data?.items.length ?? 0}${week.data?.more ? '+' : ''}`
  return (
    <Text className="text-[13px] text-muted-foreground">
      <Text onPress={() => router.navigate('/tasks?view=my_week')} className="text-[13px] text-muted-foreground">
        {weekCount} {weekCount === '1' ? 'task' : 'tasks'} due this week
      </Text>
      {' · '}
      <Text onPress={() => router.navigate('/activity')} className="text-[13px] text-muted-foreground">
        {unread.length} unread {unread.length === 1 ? 'notification' : 'notifications'}
      </Text>
    </Text>
  )
}

type Sheet = { kind: 'add' } | { kind: 'rows'; id: string } | null

/**
 * Home: the person's cards, one layout shared with the web and saved on the server. Customize adds, removes, moves
 * and sizes cards; every change saves at once.
 */
export default function HomeScreen() {
  const { user } = useOrbit()
  const { colors } = useTheme()
  const guest = useIsGuest()
  const agent = useAskDonnaAgent()
  const layout = useHomeLayout()
  const save = useSaveHomeLayout()
  const queryClient = useQueryClient()
  const [customizing, setCustomizing] = useState(false)
  const [sheet, setSheet] = useState<Sheet>(null)
  const [refreshing, setRefreshing] = useState(false)
  // the web may have changed the layout meanwhile
  useFocusEffect(
    useCallback(() => {
      void layout.refetch()
    }, [layout.refetch]),
  )

  const now = new Date()
  const firstName = user.display_name.split(' ')[0] ?? user.display_name
  // edits change the whole saved list; cards the phone hides keep their place in it
  const saved = layout.data?.cards ?? []
  const cards = layout.data ? visibleCards(layout.data, agent !== null) : []
  const addable = layout.data ? addableCards(layout.data, agent !== null) : []
  const rowsCard = sheet?.kind === 'rows' ? saved.find((card) => card.id === sheet.id) : undefined

  const refresh = async () => {
    setRefreshing(true)
    // the layout and the data of the cards on screen
    await queryClient.refetchQueries({ type: 'active' })
    setRefreshing(false)
  }

  return (
    <SafeAreaView edges={['top', 'left', 'right']} className="flex-1 bg-background">
      <PaneHeader>
        <MenuButton />
        <HeaderTitle size={14}>Home</HeaderTitle>
        <View className="flex-1" />
        {guest ? null : <IconButton icon="search" label="Search" onPress={() => router.navigate('/tasks?search=1')} />}
      </PaneHeader>
      <ScrollView
        contentContainerClassName="px-4 pt-8 pb-6 gap-6"
        refreshControl={<RefreshControl refreshing={refreshing} onRefresh={() => void refresh()} tintColor={colors.mutedForeground} />}
      >
        <View className="flex-row items-start gap-3">
          <View className="flex-1">
            <Text className="font-noto-semibold text-[24px] leading-[28px] tracking-[-0.6px] mb-1.5">
              {greetingFor(now.getHours())}, {firstName}
            </Text>
            {guest ? null : <MemberSummary />}
          </View>
          {layout.data && !customizing ? <IconButton icon="edit" label="Customize Home" size={32} color={colors.mutedForeground} onPress={() => setCustomizing(true)} /> : null}
        </View>

        {customizing ? (
          <View className="flex-row items-center gap-2">
            <Button label="Add card" variant="outline" icon="plus" disabled={addable.length === 0} onPress={() => setSheet({ kind: 'add' })} />
            <View className="flex-1" />
            <Button label="Done" onPress={() => setCustomizing(false)} />
          </View>
        ) : null}

        {layout.isError ? <Text className="text-[13px] text-muted-foreground">Home could not load. Pull down to try again.</Text> : null}
        {layout.data && cards.length === 0 ? (
          <View className="rounded-lg border border-dashed border-border p-8">
            <Text className="text-center text-[13px] text-muted-foreground">{customizing ? 'Add a card to start.' : 'Your Home has no cards. Tap the pencil to add some.'}</Text>
          </View>
        ) : null}

        <View className="gap-4">
          {cards.map((card, index) => (
            <HomeCardFrame
              key={card.id}
              title={cardTitle(card.id, agent?.name ?? null)}
              href={HOME_CARDS[card.id]?.href}
              controls={
                customizing
                  ? {
                      first: index === 0,
                      last: index === cards.length - 1,
                      limit: card.id === 'ask_donna' ? null : cardLimit(card),
                      onLimit: () => setSheet({ kind: 'rows', id: card.id }),
                      onMove: (step) => void save(stepCard(saved, cards, card.id, step)),
                      onRemove: () => void save(removeCard(saved, card.id)),
                    }
                  : null
              }
            >
              <HomeCardBody id={card.id} limit={cardLimit(card)} agentName={agent?.name ?? null} />
            </HomeCardFrame>
          ))}
        </View>
      </ScrollView>

      <PickerSheet
        visible={sheet?.kind === 'add'}
        title="Add card"
        options={addable.map((id) => ({ key: id, label: cardTitle(id, agent?.name ?? null) }))}
        onSelect={(id) => {
          setSheet(null)
          void save(addCard(saved, id))
        }}
        onClose={() => setSheet(null)}
      />
      <PickerSheet
        visible={rowsCard !== undefined}
        title={rowsCard ? `Rows in ${cardTitle(rowsCard.id, agent?.name ?? null)}` : 'Rows'}
        options={CARD_LIMITS.map((limit) => ({ key: String(limit), label: `${limit} rows`, selected: rowsCard ? cardLimit(rowsCard) === limit : false }))}
        onSelect={(limit) => {
          setSheet(null)
          if (rowsCard) void save(setCardLimit(saved, rowsCard.id, Number(limit)))
        }}
        onClose={() => setSheet(null)}
      />
    </SafeAreaView>
  )
}
