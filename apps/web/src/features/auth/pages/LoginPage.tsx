import { useState } from 'react'
import { Link, useNavigate, useSearchParams } from 'react-router'
import { useAuthOptions, useLogin } from '@/features/auth/api'
import { initialLoginValues } from '@/features/auth/authState'
import { AuthForm, AuthInput } from '@/features/auth/components/AuthForm'

import { oauthLoginDestination } from '@/features/auth/oauth'

export function LoginPage() {
  const [params] = useSearchParams()
  const navigate = useNavigate()
  const mutation = useLogin()
  const options = useAuthOptions()
  const [email, setEmail] = useState(() => initialLoginValues(import.meta.env.DEV).email)
  const [password, setPassword] = useState(() => initialLoginValues(import.meta.env.DEV).password)
  return (
    <AuthForm title="Sign in to Orbit" error={mutation.error} pending={mutation.isPending} submitLabel="Sign in" onSubmit={async () => {
      await mutation.mutateAsync({ email, password })
      navigate(oauthLoginDestination(params.get('return_to')), { replace: true })
    }} footer={(
      <span className="flex flex-wrap justify-center gap-x-4 gap-y-1">
        <Link to="/recovery">Forgot your password?</Link>
        {options.data?.registration_open ? <Link to="/register">Create an account</Link> : null}
      </span>
    )}>
      <AuthInput label="Email" type="email" value={email} onChange={setEmail} />
      <AuthInput label="Password" type="password" value={password} onChange={setPassword} />
    </AuthForm>
  )
}
