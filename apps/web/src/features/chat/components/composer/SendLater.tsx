import { useState } from 'react'
import { ChevronDown, Clock } from 'reicon-react'
import { toast } from 'sonner'
import type { ScheduledMessage } from '@orbit/api/scheduled'
import { Modal } from '@/components/common/Modal'
import { Tip } from '@/components/common/Tip'
import { Button } from '@/components/ui/button'
import { DialogClose, DialogFooter } from '@/components/ui/dialog'
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import { problemDetail } from '@/features/scheduled/problem'
import { useDeleteScheduled, useScheduledMessages, useUpdateScheduled } from '@/features/scheduled/api'
import { fromLocalInput, localInputValue, sendLaterPresets, whenLabel } from '@/features/scheduled/times'
import { useChatContext } from '../../api/chatContext'
import { decodeMentions, encodeMentions } from '../../lib/mentionTokens'
import { useChatPeople } from '../messages/people'

/** The arrow next to Send: Send later… and the list of the person's scheduled messages. */
export function SendLaterMenu({ canSchedule, onSendLater, onShowScheduled }: { canSchedule: boolean; onSendLater: () => void; onShowScheduled: () => void }) {
  return (
    <DropdownMenu>
      <Tip label="Send later">
        <DropdownMenuTrigger render={<Button variant="ghost" size="icon" aria-label="More send options" className="-ml-1 w-6" />}>
          <ChevronDown className="size-4" />
        </DropdownMenuTrigger>
      </Tip>
      <DropdownMenuContent side="top" align="end" className="w-auto min-w-48">
        <DropdownMenuItem disabled={!canSchedule} onClick={onSendLater}>
          <Clock className="size-4" />
          Send later…
        </DropdownMenuItem>
        <DropdownMenuItem onClick={onShowScheduled}>Scheduled messages</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

/** Picks the time of a message: a quick choice or a date and time (local). */
export function SendLaterDialog({ title = 'Send later', initial, pending = false, onPick, onClose }: { title?: string; initial?: Date; pending?: boolean; onPick: (at: Date) => void; onClose: () => void }) {
  const [now] = useState(() => new Date())
  // A time that has passed (a failed message) starts an hour from now.
  const [value, setValue] = useState(() => localInputValue(initial && initial > now ? initial : new Date(now.getTime() + 60 * 60 * 1000)))
  const custom = fromLocalInput(value)
  const valid = custom !== null && custom > now
  return (
    <Modal title={title} description="The message is posted as you at this time." onClose={onClose} className="sm:max-w-sm">
      <div className="flex flex-col gap-1">
        {sendLaterPresets(now).map((preset) => (
          <Button key={preset.label} type="button" variant="ghost" className="justify-between font-normal" disabled={pending} onClick={() => onPick(preset.at)}>
            {preset.label}
            <span className="text-xs text-muted-foreground">{whenLabel(preset.at, now)}</span>
          </Button>
        ))}
      </div>
      <label className="flex flex-col gap-1.5 text-sm">
        <span className="text-muted-foreground">Date and time</span>
        <Input type="datetime-local" value={value} min={localInputValue(now)} onChange={(event) => setValue(event.target.value)} aria-label="Date and time" />
      </label>
      <DialogFooter>
        <DialogClose render={<Button type="button" variant="outline" />}>Cancel</DialogClose>
        <Button type="button" disabled={!valid || pending} onClick={() => custom && onPick(custom)}>
          Schedule
        </Button>
      </DialogFooter>
    </Modal>
  )
}

/** Above the composer: how many of the person's messages wait for this conversation, with the list. */
export function ScheduledBar({ conversationId, onOpen }: { conversationId: string; onOpen: () => void }) {
  const { workspaceId } = useChatContext()
  const here = (useScheduledMessages(workspaceId).data ?? []).filter((message) => message.kind === 'message' && message.conversation_id === conversationId)
  if (here.length === 0) return null
  const failed = here.filter((message) => message.status === 'failed').length
  return (
    <button
      type="button"
      data-slot="scheduled-bar"
      onClick={onOpen}
      className="mb-1.5 flex w-full items-center gap-2 rounded-md px-2 py-1 text-left text-xs text-muted-foreground transition-[color,background-color] hover:bg-muted/60 hover:text-foreground"
    >
      <Clock className="size-4 shrink-0" />
      <span className="min-w-0 flex-1 truncate">
        {here.length === 1 ? '1 message scheduled here' : `${here.length} messages scheduled here`}
        {failed > 0 ? <span className="text-destructive"> · {failed} not sent</span> : ` · next ${whenLabel(here[0].send_at)}`}
      </span>
      <span className="font-medium text-foreground">View</span>
    </button>
  )
}

function where(message: ScheduledMessage): string {
  if (message.kind === 'reminder') return 'Reminder'
  if (message.conversation_kind === 'dm') return 'Direct message'
  return message.conversation_name ? `#${message.conversation_name}` : 'Deleted conversation'
}

/** The person's scheduled messages and reminders, soonest first: change the text or the time, or cancel. */
export function ScheduledMessagesDialog({ onClose }: { onClose: () => void }) {
  const { workspaceId } = useChatContext()
  const people = useChatPeople()
  const scheduled = useScheduledMessages(workspaceId)
  const update = useUpdateScheduled(workspaceId)
  const cancel = useDeleteScheduled(workspaceId)
  const [editing, setEditing] = useState<{ message: ScheduledMessage; text: string } | null>(null)
  const [retiming, setRetiming] = useState<ScheduledMessage | null>(null)
  const items = scheduled.data ?? []
  const fail = (error: unknown) => toast.error(problemDetail(error, 'Could not change the message.'))

  return (
    <Modal title="Scheduled messages" description="Your messages and reminders that wait to be sent." onClose={onClose} className="sm:max-w-lg">
      {scheduled.isPending ? null : items.length === 0 ? (
        <p className="text-sm text-muted-foreground">Nothing is scheduled.</p>
      ) : (
        <ul className="-mx-2 flex max-h-[60vh] flex-col overflow-y-auto">
          {items.map((message) => (
            <li key={message.id} data-slot="scheduled-message" className="flex flex-col gap-1.5 rounded-md px-2 py-2 hover:bg-muted/40">
              <div className="flex items-center gap-2 text-xs text-muted-foreground">
                <span className="font-medium text-foreground">{where(message)}</span>
                <span>·</span>
                <span>{whenLabel(message.send_at)}</span>
                {message.status === 'failed' ? <span className="text-destructive">· {message.error ?? 'Not sent.'}</span> : null}
              </div>
              {editing?.message.id === message.id ? (
                <div className="flex flex-col gap-2">
                  <Textarea value={editing.text} onChange={(event) => setEditing({ message, text: event.target.value })} aria-label="Message" className="min-h-20" />
                  <div className="flex justify-end gap-2">
                    <Button type="button" size="sm" variant="outline" onClick={() => setEditing(null)}>
                      Cancel
                    </Button>
                    <Button
                      type="button"
                      size="sm"
                      disabled={editing.text.trim() === '' || update.isPending}
                      onClick={() =>
                        update.mutate(
                          { id: message.id, body: encodeMentions(editing.text.trim(), people.members, people.channels) },
                          { onSuccess: () => setEditing(null), onError: fail },
                        )
                      }
                    >
                      Save
                    </Button>
                  </div>
                </div>
              ) : (
                <p className="line-clamp-3 text-sm break-words whitespace-pre-wrap">{decodeMentions(message.body, people.members, people.channels)}</p>
              )}
              {editing?.message.id === message.id ? null : (
                <div className="flex gap-1">
                  <Button type="button" size="sm" variant="ghost" onClick={() => setEditing({ message, text: decodeMentions(message.body, people.members, people.channels) })}>
                    Edit
                  </Button>
                  <Button type="button" size="sm" variant="ghost" onClick={() => setRetiming(message)}>
                    {message.status === 'failed' ? 'Try again at…' : 'Change time'}
                  </Button>
                  <Button type="button" size="sm" variant="ghost" className="text-destructive" onClick={() => cancel.mutate(message.id, { onError: fail })}>
                    {message.kind === 'reminder' ? 'Cancel reminder' : 'Cancel message'}
                  </Button>
                </div>
              )}
            </li>
          ))}
        </ul>
      )}
      {retiming ? (
        <SendLaterDialog
          title="Change time"
          initial={new Date(retiming.send_at)}
          pending={update.isPending}
          onClose={() => setRetiming(null)}
          onPick={(at) => update.mutate({ id: retiming.id, sendAt: at }, { onSuccess: () => setRetiming(null), onError: fail })}
        />
      ) : null}
    </Modal>
  )
}
