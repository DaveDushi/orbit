import { useState, type FormEvent } from 'react'
import { Modal } from '@/components/common/Modal'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'

interface NameDialogProps {
  title: string
  label: string
  initial?: string
  submitLabel: string
  /** Resolves when the name is saved; a thrown error keeps the dialog open with its message. */
  onSubmit: (name: string) => Promise<unknown>
  onClose: () => void
}

/** Asks for a file or folder name (new folder, rename). The extension is not selected, so typing keeps it. */
export function NameDialog({ title, label, initial = '', submitLabel, onSubmit, onClose }: NameDialogProps) {
  const [name, setName] = useState(initial)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const submit = async (event: FormEvent) => {
    event.preventDefault()
    if (name.trim() === '') return
    setBusy(true)
    setError(null)
    try {
      await onSubmit(name.trim())
      onClose()
    } catch (failure) {
      setError(failure instanceof Error ? failure.message : 'Could not save the name.')
      setBusy(false)
    }
  }

  return (
    <Modal title={title} onClose={onClose} className="sm:max-w-md" dismissible={!busy}>
      <form className="flex flex-col gap-3" onSubmit={(event) => void submit(event)}>
        <Input
          aria-label={label}
          value={name}
          autoFocus
          maxLength={255}
          onFocus={(event) => {
            const dot = initial.lastIndexOf('.')
            event.currentTarget.setSelectionRange(0, dot > 0 ? dot : initial.length)
          }}
          onChange={(event) => setName(event.target.value)}
        />
        {error ? <p className="text-sm text-destructive" role="alert">{error}</p> : null}
        <div className="flex justify-end gap-2">
          <Button type="button" variant="outline" onClick={onClose} disabled={busy}>Cancel</Button>
          <Button type="submit" disabled={busy || name.trim() === ''}>{submitLabel}</Button>
        </div>
      </form>
    </Modal>
  )
}
