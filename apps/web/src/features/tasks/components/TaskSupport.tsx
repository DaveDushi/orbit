import { useId, useRef, useState, type ReactNode } from 'react'
import { ArrowUp, Danger, DocumentText, Sms } from 'reicon-react'
import { cn } from 'cn'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Textarea } from '@/components/ui/textarea'
import { Tip } from '@/components/common/Tip'
import type { SupportMessage, SupportTicket } from '@/features/tasks/api/support'
import { useRetrySupportMessage, useSendSupportReply } from '@/features/tasks/api/support'
import type { TaskViewState } from '@/features/tasks/api/models'
import { agoLabel } from '@/features/tasks/tasksLib'
import { emailDocument, hasExternalImages } from '@/features/tasks/lib/supportEmail'
import { TaskStatusIcon } from './TaskStatusIcon'

/**
 * An email's HTML in a sandboxed frame: no scripts, forms or top navigation; links open in a new tab. `allow-same-origin`
 * (without `allow-scripts`) lets the frame load the ticket's inline images with the session and lets this page size it.
 */
function EmailFrame({ html, title }: { html: string; title: string }) {
  const [showImages, setShowImages] = useState(false)
  const [height, setHeight] = useState(120)
  const frame = useRef<HTMLIFrameElement>(null)
  const external = hasExternalImages(html)
  const fit = () => {
    const body = frame.current?.contentDocument?.documentElement
    if (body) setHeight(Math.min(Math.max(body.scrollHeight, 48), 2000))
  }
  return (
    <div className="grid gap-1.5">
      {external ? (
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          {showImages ? 'Images from other sites are shown.' : 'Images from other sites are blocked.'}
          {showImages ? null : <Button type="button" variant="link" size="xs" className="h-auto p-0" onClick={() => setShowImages(true)}>Show images</Button>}
        </div>
      ) : null}
      <iframe
        ref={frame}
        key={showImages ? 'images' : 'blocked'}
        title={title}
        sandbox="allow-same-origin allow-popups allow-popups-to-escape-sandbox"
        referrerPolicy="no-referrer"
        srcDoc={emailDocument(html, showImages)}
        onLoad={fit}
        style={{ height }}
        className="w-full rounded-md border bg-white"
      />
    </div>
  )
}

function SendState({ message, onRetry, retrying }: { message: SupportMessage; onRetry: () => void; retrying: boolean }) {
  if (message.send_state === 'sent') return <span className="text-xs text-muted-foreground">Sent</span>
  if (message.send_state === 'queued') {
    return <span className="text-xs text-muted-foreground" title={message.send_error ?? undefined}>{message.send_error ? 'Sending… (retrying)' : 'Sending…'}</span>
  }
  if (message.send_state === 'failed') {
    return (
      <span className="flex items-center gap-1.5 text-xs text-destructive" role="alert">
        <Danger className="size-3.5" aria-hidden="true" />
        <span title={message.send_error ?? undefined}>Not sent</span>
        <Button type="button" variant="outline" size="xs" disabled={retrying} onClick={onRetry}>Retry</Button>
      </span>
    )
  }
  return null
}

/** One email of a ticket in the activity feed: from the customer, or a reply that was emailed. */
export function SupportMessageCard({ message, workspaceId, users }: { message: SupportMessage; workspaceId: string; users: TaskViewState['users'] }) {
  const retry = useRetrySupportMessage(workspaceId, message.task_id)
  const inbound = message.direction === 'in'
  const author = users.find((user) => user.id === message.author_id)
  const files = message.attachments.filter((attachment) => !attachment.inline)
  const who = inbound ? (message.from_name || message.from_address) : (author?.name ?? message.from_name ?? 'Reply')
  return (
    <article
      data-slot="support-message"
      data-direction={message.direction}
      className="my-3 overflow-hidden rounded-[10px] border bg-card/50 data-[direction=out]:border-sky-500/40 data-[direction=out]:bg-sky-500/5"
    >
      <header className="flex flex-wrap items-center gap-x-2 gap-y-1 border-b px-3.5 py-2 text-xs text-muted-foreground">
        <Sms className={cn('size-3.5 shrink-0', inbound ? 'text-muted-foreground' : 'text-sky-600 dark:text-sky-400')} aria-hidden="true" />
        <span className="font-medium text-foreground">{who}</span>
        <span className="truncate">{inbound ? `<${message.from_address}>` : `to ${message.to.join(', ')}`}</span>
        <span>· {agoLabel(message.created_at)}</span>
        {inbound && message.unverified ? (
          <Tip label="The mail provider reported failed SPF, DKIM or DMARC checks: the sender may be forged.">
            <Badge variant="outline" className="border-amber-500/50 text-amber-700 dark:text-amber-400">Unverified sender</Badge>
          </Tip>
        ) : null}
        <span className="ml-auto">{inbound ? <span>Email</span> : <SendState message={message} retrying={retry.isPending} onRetry={() => retry.mutate(message.id)} />}</span>
      </header>
      <div className="grid gap-2 px-3.5 py-3 text-[13px]">
        {message.body_html
          ? <EmailFrame html={message.body_html} title={`Email from ${message.from_address}`} />
          : <p className="m-0 break-words whitespace-pre-wrap text-foreground">{message.body_text}</p>}
        {files.length > 0 ? (
          <ul className="m-0 flex list-none flex-wrap gap-1.5 p-0" aria-label="Email attachments">
            {files.map((file) => (
              <li key={file.id}>
                <a
                  className="inline-flex max-w-60 items-center gap-1.5 rounded-md border px-2 py-1 text-xs text-foreground hover:bg-muted"
                  href={`/api/v1/workspaces/${workspaceId}/tasks/${message.task_id}/attachments/${file.id}/download`}
                  download={file.name}
                  rel="noopener"
                >
                  <DocumentText className="size-3.5 shrink-0" aria-hidden="true" />
                  <span className="truncate">{file.name}</span>
                </a>
              </li>
            ))}
          </ul>
        ) : null}
        {message.skipped_attachments > 0 ? <p className="m-0 text-xs text-muted-foreground">{message.skipped_attachments} attachment(s) left out: over 25 MB or 20 files.</p> : null}
      </div>
    </article>
  )
}

type Mode = 'note' | 'reply'

/**
 * The composer of a support ticket: Internal note (a comment, the default) or Reply to customer (an email). The reply
 * field has its own colour and names the recipient, so a note is not emailed by mistake.
 */
export function TicketComposer({ ticket, workspaceId, taskId, note }: { ticket: NonNullable<SupportTicket['ticket']>; workspaceId: string; taskId: string; note: ReactNode }) {
  const [mode, setMode] = useState<Mode>('note')
  const [draft, setDraft] = useState('')
  const send = useSendSupportReply(workspaceId, taskId)
  const fieldId = useId()
  const submit = async () => {
    if (!draft.trim() || send.isPending) return
    try {
      await send.mutateAsync(draft)
      setDraft('')
      // back to the default: the next thing typed is a note, not another email
      setMode('note')
    } catch {
      // shown below; the draft stays
    }
  }
  const tab = (value: Mode, label: string, disabled = false) => (
    <button
      type="button"
      role="tab"
      aria-selected={mode === value}
      disabled={disabled}
      data-mode={value}
      onClick={() => setMode(value)}
      className="rounded-md px-2.5 py-1 text-xs font-medium text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50 disabled:opacity-50 aria-selected:bg-muted aria-selected:text-foreground data-[mode=reply]:aria-selected:bg-sky-500/15 data-[mode=reply]:aria-selected:text-sky-700 dark:data-[mode=reply]:aria-selected:text-sky-300"
    >
      {label}
    </button>
  )
  return (
    <div className="grid gap-2">
      <div role="tablist" aria-label="Write" className="flex items-center gap-1">
        {tab('note', 'Internal note')}
        {ticket.can_reply ? tab('reply', 'Reply to customer') : (
          <Tip label="Add an SMTP server in Project settings → Support email to answer by email.">
            <span>{tab('reply', 'Reply to customer', true)}</span>
          </Tip>
        )}
      </div>
      {mode === 'note' ? note : (
        <div data-slot="support-reply" className="rounded-[10px] border border-sky-500/60 bg-sky-500/5 transition-colors focus-within:border-sky-500">
          <p className="m-0 flex items-center gap-1.5 border-b border-sky-500/30 px-3 py-1.5 text-xs font-medium text-sky-700 dark:text-sky-300">
            <Sms className="size-3.5" aria-hidden="true" />
            Sends email to {ticket.contact.email}
          </p>
          <label htmlFor={fieldId} className="sr-only">Reply to customer</label>
          <Textarea
            id={fieldId}
            value={draft}
            disabled={send.isPending}
            placeholder="Write a reply…"
            className="min-h-24 resize-y border-0 bg-transparent px-3 text-[13px] shadow-none focus-visible:ring-0 dark:bg-transparent"
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
                event.preventDefault()
                void submit()
              }
            }}
          />
          {send.isError ? <p role="alert" className="m-0 px-3 pb-1 text-xs text-destructive">The reply could not be queued. The draft is kept.</p> : null}
          <div className="flex items-center justify-end gap-2 p-1.5">
            <span className="text-xs text-muted-foreground">Plain text email from {ticket.mailbox_address}</span>
            <Button type="button" size="sm" className="bg-sky-600 text-white hover:bg-sky-600/90" disabled={!draft.trim() || send.isPending} onClick={() => void submit()}>
              <ArrowUp data-icon="inline-start" />
              Send email
            </Button>
          </div>
        </div>
      )}
    </div>
  )
}

/** The ticket's customer in the side panel: name, email and their other tickets. */
export function CustomerProperty({ ticket, statuses, onOpenTask }: { ticket: NonNullable<SupportTicket['ticket']>; statuses: TaskViewState['statuses']; onOpenTask?: (taskId: string) => void }) {
  const { contact, other_tickets: others } = ticket
  return (
    <div data-slot="support-customer" className="flex w-full min-w-0 flex-col gap-1 text-[13px]">
      {contact.name ? <span className="truncate font-medium text-foreground">{contact.name}</span> : null}
      <a className="truncate text-muted-foreground hover:text-foreground" href={`mailto:${contact.email}`}>{contact.email}</a>
      {others.length > 0 ? (
        <div className="mt-1.5 flex w-full flex-col gap-0.5">
          <span className="text-xs text-muted-foreground/70">Other tickets</span>
          {others.map((other) => {
            const status = statuses.find((item) => item.id === other.status_id)
            return (
              <Button
                key={other.task_id}
                type="button"
                variant="ghost"
                size="sm"
                className="-ml-2 h-7 w-full justify-start gap-1.5 px-2 text-xs font-normal"
                disabled={!onOpenTask}
                onClick={() => onOpenTask?.(other.task_id)}
              >
                {status ? <TaskStatusIcon status={status} size={12} /> : null}
                <span className="shrink-0 text-muted-foreground">{other.identifier}</span>
                <span className="truncate">{other.title}</span>
              </Button>
            )
          })}
        </div>
      ) : null}
    </div>
  )
}
