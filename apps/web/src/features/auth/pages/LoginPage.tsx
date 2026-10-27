import { useId, useState } from 'react'
import { Link, useNavigate, useSearchParams } from 'react-router'
import { ApiProblem } from '@orbit/api/problem'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Field, FieldDescription, FieldLabel } from '@/components/ui/field'
import { isTwoFactorChallenge, useAuthOptions, useLogin, useLoginSecondFactor, usePasskeyLogin } from '@/features/auth/api'
import { initialLoginValues } from '@/features/auth/authState'
import { AuthForm, AuthInput } from '@/features/auth/components/AuthForm'
import { isPasskeyCancel, passkeysSupported } from '@/features/auth/passkey'

import { oauthLoginDestination } from '@/features/auth/oauth'

export function LoginPage() {
  const [params] = useSearchParams()
  const navigate = useNavigate()
  const mutation = useLogin()
  const secondFactor = useLoginSecondFactor()
  const passkey = usePasskeyLogin()
  const options = useAuthOptions()
  const [email, setEmail] = useState(() => initialLoginValues(import.meta.env.DEV).email)
  const [password, setPassword] = useState(() => initialLoginValues(import.meta.env.DEV).password)
  // Set while the password step is done and the account asks for a code.
  const [twoFactorToken, setTwoFactorToken] = useState<string | null>(null)
  const [code, setCode] = useState('')
  const codeId = useId()
  // Why the code step ended (expired or used-up challenge), shown on the password step.
  const [returnError, setReturnError] = useState<Error | null>(null)
  const signedIn = () => navigate(oauthLoginDestination(params.get('return_to')), { replace: true })

  const backToPassword = (error: Error | null = null) => {
    setTwoFactorToken(null)
    setCode('')
    secondFactor.reset()
    setReturnError(error)
  }

  if (twoFactorToken) {
    return (
      <AuthForm title="Two-factor sign-in" error={secondFactor.error} pending={secondFactor.isPending} submitLabel="Verify" onSubmit={async () => {
        try {
          await secondFactor.mutateAsync({ two_factor_token: twoFactorToken, code: code.trim() })
        } catch (error) {
          if (error instanceof ApiProblem && error.code === 'invalid_two_factor_token') backToPassword(error)
          throw error
        }
        void signedIn()
      }} footer={<Button type="button" variant="link" className="h-auto p-0" onClick={() => backToPassword()}>Back</Button>}>
        <Field className="gap-1.5">
          <FieldLabel htmlFor={codeId}>Authentication code</FieldLabel>
          <Input id={codeId} required autoFocus autoComplete="one-time-code" spellCheck={false} aria-describedby={`${codeId}-hint`} value={code} onChange={(event) => setCode(event.target.value)} />
          <FieldDescription id={`${codeId}-hint`}>Enter the 6-digit code from your authenticator app, or a recovery code.</FieldDescription>
        </Field>
      </AuthForm>
    )
  }

  const showPasskey = options.data?.passkeys_enabled === true && passkeysSupported()
  const passkeyError = passkey.error && !isPasskeyCancel(passkey.error) ? passkey.error : null
  return (
    <AuthForm title="Orbit" description="Sign in to your tasks, docs, mail and chat." error={mutation.error ?? passkeyError ?? returnError} pending={mutation.isPending || passkey.isPending} submitLabel="Sign in" onSubmit={async () => {
      setReturnError(null)
      passkey.reset()
      const response = await mutation.mutateAsync({ email, password })
      if (isTwoFactorChallenge(response)) {
        setCode('')
        setTwoFactorToken(response.two_factor_token)
        return
      }
      void signedIn()
    }} actions={showPasskey ? (
      <Button type="button" variant="outline" size="lg" className="border-primary/30 bg-primary/10 hover:bg-primary/15 dark:border-primary/30 dark:bg-primary/10 dark:hover:bg-primary/15" disabled={mutation.isPending || passkey.isPending} onClick={() => {
        setReturnError(null)
        mutation.reset()
        passkey.mutate(undefined, { onSuccess: () => void signedIn() })
      }}>
        Sign in with a passkey
      </Button>
    ) : null} footer={options.data?.registration_open ? <>New to Orbit? <Link to="/register" className="font-medium text-foreground">Create an account</Link></> : undefined}>
      <AuthInput label="Email" type="email" value={email} onChange={setEmail} />
      <AuthInput label="Password" type="password" value={password} onChange={setPassword} />
      <Link to="/recovery" className="justify-self-end text-[13px] text-muted-foreground hover:text-foreground">Forgot password?</Link>
    </AuthForm>
  )
}
