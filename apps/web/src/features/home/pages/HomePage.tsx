import { useState, type DragEvent, type ReactNode } from 'react'
import { Link } from 'react-router'
import { toast } from 'sonner'
import { Add, ArrowDown2, ArrowUp2, Edit2, Menu, Trash } from 'reicon-react'
import { Button, buttonVariants } from '@/components/ui/button'
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Pane } from '@/components/common/Pane'
import { Tip } from '@/components/common/Tip'
import { useCurrentUser } from '@/features/auth/api'
import { useWorkspace } from '@/features/workspaces/workspaceContext'
import { useIsGuest } from '@/features/workspaces/permissions'
import { useNotifications } from '@/features/inbox/api'
import { useTasks } from '@/features/tasks/api/tasks'
import { chatEnabled, docsHidden } from '@/app/shell/productNavigation'
import { useHomeLayout, useSaveHomeLayout } from '../api'
import { HomeCardBody } from '../components/HomeCards'
import { addableCards, addCard, HOME_CARDS, CARD_LIMITS, cardLimit, dropCard, removeCard, setCardLimit, type HomeCardEntry } from '../layout'

function greetingFor(hour: number): string {
  if (hour < 12) return 'Good morning'
  if (hour < 18) return 'Good afternoon'
  return 'Good evening'
}

/** A card this build can show: known to the web, and its product is not turned off. */
function shown(id: string) {
  return id in HOME_CARDS && !(docsHidden && id === 'recent_docs') && !(!chatEnabled && id === 'unread_chat')
}

/** Tasks due this week and unread notifications, under the greeting (members only: a guest has neither). */
function MemberSummary({ workspaceId }: { workspaceId: string }) {
  const thisWeek = useTasks(workspaceId, { view: 'my_week', sort: 'due_date', order: 'asc', limit: 7 })
  const unread = useNotifications(workspaceId, true).data ?? []
  // a full first page with a next page means there are more tasks than it holds
  const page = thisWeek.data?.pages[0]
  const weekCount = `${page?.items.length ?? 0}${page?.next_cursor ? '+' : ''}`
  return (
    <p className="text-[13px] text-muted-foreground [&_a]:text-muted-foreground [&_a]:no-underline [&_a:hover]:text-foreground [&_a:hover]:underline">
      <Link to="/tasks?view=my_week">
        {weekCount} {weekCount === '1' ? 'task' : 'tasks'} due this week
      </Link>
      {' · '}
      <Link to="/inbox">
        {unread.length} unread {unread.length === 1 ? 'notification' : 'notifications'}
      </Link>
    </p>
  )
}

interface CardControls {
  index: number
  count: number
  limit: number
  onMove: (step: -1 | 1) => void
  onLimit: (limit: number) => void
  onRemove: () => void
}

function HomeCard({ id, title, viewAllTo, controls, children }: { id: string; title: string; viewAllTo?: string; controls: CardControls | null; children: ReactNode }) {
  return (
    <>
      <header className="flex min-h-12 items-center gap-1 py-2 pr-2 pl-4 text-sm font-medium text-muted-foreground">
        {controls ? <Menu aria-hidden="true" className="-ml-1 mr-1 size-4 shrink-0 cursor-grab text-muted-foreground/70" /> : null}
        <span className="min-w-0 flex-1 truncate">{title}</span>
        {controls ? (
          <>
            {id === 'ask_donna' ? null : (
              <Select items={CARD_LIMITS.map((value) => ({ value: String(value), label: `${value} rows` }))} value={String(controls.limit)} onValueChange={(value) => controls.onLimit(Number(value))}>
                <SelectTrigger size="sm" aria-label={`Rows in ${title}`} className="w-auto">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {CARD_LIMITS.map((value) => <SelectItem key={value} value={String(value)}>{value} rows</SelectItem>)}
                </SelectContent>
              </Select>
            )}
            <Tip label="Move up">
              <Button variant="ghost" size="icon-sm" aria-label={`Move ${title} up`} disabled={controls.index === 0} onClick={() => controls.onMove(-1)}>
                <ArrowUp2 className="size-4" />
              </Button>
            </Tip>
            <Tip label="Move down">
              <Button variant="ghost" size="icon-sm" aria-label={`Move ${title} down`} disabled={controls.index === controls.count - 1} onClick={() => controls.onMove(1)}>
                <ArrowDown2 className="size-4" />
              </Button>
            </Tip>
            <Tip label="Remove">
              <Button variant="ghost" size="icon-sm" aria-label={`Remove ${title}`} onClick={controls.onRemove}>
                <Trash className="size-4" />
              </Button>
            </Tip>
          </>
        ) : viewAllTo ? (
          <Link to={viewAllTo} className={buttonVariants({ variant: 'ghost' })}>
            View all
          </Link>
        ) : null}
      </header>
      {/* while customizing, the rows are a preview: links and fields do not take clicks */}
      <div inert={controls !== null} className="relative min-w-0 overflow-hidden rounded-lg bg-background shadow-[0_0_0_1px_var(--muted)]">
        {children}
      </div>
    </>
  )
}

export function HomePage() {
  const { workspace } = useWorkspace()
  const me = useCurrentUser()
  const guest = useIsGuest()
  const layout = useHomeLayout(workspace.id)
  const save = useSaveHomeLayout(workspace.id)
  const [customizing, setCustomizing] = useState(false)
  const [dragId, setDragId] = useState<string | null>(null)
  const [overId, setOverId] = useState<string | null>(null)
  const firstName = me.data?.display_name.split(' ')[0] ?? 'there'
  const now = new Date()

  // edits change the whole saved list; cards this build hides keep their place in it
  const saved = layout.data?.cards ?? []
  const cards = saved.filter((card) => shown(card.id))
  const addable = addableCards(saved, (layout.data?.available ?? []).filter(shown))
  const change = (next: HomeCardEntry[]) => save.mutate(next, { onError: () => toast.error('Could not save your Home. Try again.') })

  const dragProps = (id: string) =>
    customizing
      ? {
          draggable: true,
          onDragStart: (event: DragEvent) => {
            event.dataTransfer.effectAllowed = 'move'
            event.dataTransfer.setData('text/plain', id)
            setDragId(id)
          },
          onDragEnd: () => {
            setDragId(null)
            setOverId(null)
          },
          onDragOver: (event: DragEvent) => {
            if (dragId === null) return
            event.preventDefault()
            event.dataTransfer.dropEffect = 'move'
            setOverId(id)
          },
          onDrop: (event: DragEvent) => {
            event.preventDefault()
            if (dragId !== null && dragId !== id) change(dropCard(saved, dragId, id))
            setDragId(null)
            setOverId(null)
          },
        }
      : {}

  return (
    <div className="flex min-h-0 min-w-0 flex-1 overflow-hidden bg-background">
      <Pane>
        <div className="min-h-0 flex-1 overflow-y-auto">
          <div className="mx-auto flex max-w-[960px] flex-col gap-6 px-6 pt-12 pb-8 max-[599px]:px-4 max-[599px]:pt-8 max-[599px]:pb-6">
            <div className="flex flex-wrap items-start gap-3">
              <div className="min-w-0 flex-1">
                <h1 className="mb-1.5">
                  {greetingFor(now.getHours())}, {firstName}
                </h1>
                {guest ? null : <MemberSummary workspaceId={workspace.id} />}
              </div>
              {layout.data ? (
                <div className="flex items-center gap-2">
                  {customizing ? (
                    <>
                      <DropdownMenu>
                        <DropdownMenuTrigger render={<Button variant="outline" disabled={addable.length === 0} />}>
                          <Add className="size-4" />
                          Add card
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end" className="w-auto min-w-48">
                          {addable.map((id) => (
                            <DropdownMenuItem key={id} onClick={() => change(addCard(saved, id))}>
                              {HOME_CARDS[id].title}
                            </DropdownMenuItem>
                          ))}
                        </DropdownMenuContent>
                      </DropdownMenu>
                      <Button onClick={() => setCustomizing(false)}>Done</Button>
                    </>
                  ) : (
                    <Button variant="ghost" onClick={() => setCustomizing(true)}>
                      <Edit2 className="size-4" />
                      Customize
                    </Button>
                  )}
                </div>
              ) : null}
            </div>
            {layout.isError ? <p className="text-[13px] text-muted-foreground">Home could not load. Reload the page to try again.</p> : null}
            {layout.data && cards.length === 0 ? (
              <div className="rounded-lg border border-dashed p-8 text-center text-[13px] text-muted-foreground">
                {customizing ? 'Add a card to start.' : 'Your Home has no cards. Choose Customize to add some.'}
              </div>
            ) : null}
            <div className="grid grid-cols-1 items-start gap-4 min-[900px]:grid-cols-2">
              {cards.map((card, index) => {
                const meta = HOME_CARDS[card.id]
                const move = (step: -1 | 1) => change(dropCard(saved, card.id, cards[index + step].id))
                return (
                  <section
                    key={card.id}
                    aria-label={meta.title}
                    data-dragging={dragId === card.id || undefined}
                    data-drop={(overId === card.id && dragId !== card.id) || undefined}
                    className="flex w-full min-w-0 flex-col rounded-lg bg-card shadow-[0_0_0_1px_var(--border)] transition-opacity data-dragging:opacity-40 data-drop:shadow-[0_0_0_2px_var(--primary)]"
                    {...dragProps(card.id)}
                  >
                    <HomeCard
                      id={card.id}
                      title={meta.title}
                      viewAllTo={meta.viewAllTo}
                      controls={
                        customizing
                          ? {
                              index,
                              count: cards.length,
                              limit: cardLimit(card),
                              onMove: move,
                              onLimit: (limit) => change(setCardLimit(saved, card.id, limit)),
                              onRemove: () => change(removeCard(saved, card.id)),
                            }
                          : null
                      }
                    >
                      <HomeCardBody id={card.id} limit={cardLimit(card)} />
                    </HomeCard>
                  </section>
                )
              })}
            </div>
          </div>
        </div>
      </Pane>
    </div>
  )
}
