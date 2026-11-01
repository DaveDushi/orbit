import { useId } from 'react'
import { ApiProblem } from '@orbit/api/problem'
import { Field, FieldError, FieldLabel } from '@/components/ui/field'
import { PasswordInput } from '@/components/ui/password-input'

/** The current password, asked again before a change to the account. */
export function CurrentPassword({ value, onChange, error, fallback, autoFocus = true }: { value: string; onChange: (value: string) => void; error: unknown; fallback: string; autoFocus?: boolean }) {
  const id = useId()
  const message = error ? (error instanceof ApiProblem ? error.detail : fallback) : null
  return (
    <Field data-invalid={message ? true : undefined}>
      <FieldLabel htmlFor={id}>Current password</FieldLabel>
      <PasswordInput id={id} required autoFocus={autoFocus} autoComplete="current-password" aria-invalid={message ? true : undefined} value={value} onChange={(event) => onChange(event.target.value)} />
      {message ? <FieldError>{message}</FieldError> : null}
    </Field>
  )
}
