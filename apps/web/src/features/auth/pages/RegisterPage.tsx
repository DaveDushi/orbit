import { useState } from 'react'
import { Link, useNavigate } from 'react-router'
import { useAuthOptions, useCompleteRegistration, useRequestRegistration } from '@/features/auth/api'
import { AuthForm, AuthInput } from '@/features/auth/components/AuthForm'
import { AuthMessage } from '@/features/auth/components/AuthMessage'
import { useConsumedToken } from '@/features/auth/useConsumedToken'

/** Open registration: first the email (a link goes there), then from that link the name and password. */
export function RegisterPage() {
  const token = useConsumedToken()
  return token ? <CompleteRegistration token={token} /> : <RequestRegistration />
}

function RequestRegistration() {
  const options = useAuthOptions()
  const mutation = useRequestRegistration()
  const [email, setEmail] = useState('')
  if (mutation.isSuccess) return <AuthMessage title="Check your email" detail={`${mutation.data.detail} The link expires in 60 minutes.`} />
  if (options.data && !options.data.registration_open) {
    return <AuthMessage title="Registration is closed" detail="This Orbit does not accept new accounts. Ask an administrator for an invitation." />
  }
  return (
    <AuthForm title="Create your account" error={mutation.error} pending={mutation.isPending} submitLabel="Send sign-up link" onSubmit={() => mutation.mutateAsync({ email: email.trim() })} footer={<Link to="/login">Already have an account? Sign in</Link>}>
      <AuthInput label="Email" type="email" value={email} onChange={setEmail} />
    </AuthForm>
  )
}

function CompleteRegistration({ token }: { token: string }) {
  const navigate = useNavigate()
  const mutation = useCompleteRegistration()
  const [form, setForm] = useState({ display_name: '', password: '' })
  return (
    <AuthForm title="Finish creating your account" error={mutation.error} pending={mutation.isPending} submitLabel="Create account" onSubmit={async () => {
      await mutation.mutateAsync({ token, ...form })
      navigate('/onboarding', { replace: true })
    }}>
      <AuthInput label="Your name" value={form.display_name} onChange={(display_name) => setForm({ ...form, display_name })} />
      <AuthInput label="Password" type="password" value={form.password} onChange={(password) => setForm({ ...form, password })} />
    </AuthForm>
  )
}
