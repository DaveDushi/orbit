import { useId, useState } from 'react'
import { useNavigate } from 'react-router'
import { toast } from 'sonner'
import { ApiProblem } from '@orbit/api/problem'
import { Modal } from '@/components/common/Modal'
import { Button } from '@/components/ui/button'
import { DialogClose, DialogFooter } from '@/components/ui/dialog'
import { Field, FieldDescription, FieldLabel } from '@/components/ui/field'
import { Textarea } from '@/components/ui/textarea'
import { currentContext, useSendImprovement } from './api'

declare const __ORBIT_BUILD_REVISION__: string

const MAX_TEXT_CHARS = 4000

const PLACEHOLDER = 'For example: “Show the due date on the cards of the board view.” or “Make the chat Send button easier to see in dark mode.”'

/** An installation admin asks for a change; an agent on this server builds it. Callers mount it while it is open. */
export function SuggestChangeDialog({ onClose }: { onClose: () => void }) {
  const send = useSendImprovement()
  const navigate = useNavigate()
  const [text, setText] = useState('')
  const textId = useId()
  const empty = text.trim() === ''

  const submit = (event: React.FormEvent) => {
    event.preventDefault()
    if (empty || send.isPending) return
    const revision = typeof __ORBIT_BUILD_REVISION__ === 'string' ? __ORBIT_BUILD_REVISION__ : 'development'
    send.mutate(
      { text, context: currentContext(revision) },
      {
        onSuccess: () => {
          onClose()
          toast('Request sent', { action: { label: 'Feedback & fixes', onClick: () => navigate('/admin/feedback') } })
        },
      },
    )
  }

  return (
    <Modal title="Suggest a change" onClose={onClose} className="sm:max-w-lg" dismissible={!send.isPending}>
      <form className="grid gap-4" onSubmit={submit}>
        <Field>
          <FieldLabel htmlFor={textId}>What should change?</FieldLabel>
          <Textarea
            id={textId}
            autoFocus
            rows={6}
            className="max-h-[50vh] min-h-32"
            maxLength={MAX_TEXT_CHARS}
            placeholder={PLACEHOLDER}
            value={text}
            onChange={(event) => setText(event.target.value)}
          />
          <FieldDescription className="flex justify-between gap-3">
            <span>An agent builds it on this server, sends screenshots to the owner for review, and reports back under Feedback &amp; fixes.</span>
            <span className="shrink-0 tabular-nums">{text.length} / {MAX_TEXT_CHARS}</span>
          </FieldDescription>
        </Field>
        {send.isError ? (
          <p role="alert" className="text-xs text-destructive">
            {send.error instanceof ApiProblem ? send.error.detail : 'The request could not be sent. Try again.'}
          </p>
        ) : null}
        <DialogFooter>
          <DialogClose render={<Button type="button" variant="outline" disabled={send.isPending} />}>Cancel</DialogClose>
          <Button type="submit" disabled={empty || send.isPending}>{send.isPending ? 'Sending…' : 'Send'}</Button>
        </DialogFooter>
      </form>
    </Modal>
  )
}
