import { useState } from 'react'
import { Link, useNavigate } from 'react-router'
import { useAuthOptions, useCompleteRecovery, useRequestRecovery } from '@/features/auth/api'
import { recoveryRequestCopy } from '@/features/auth/authState'
import { AuthForm, AuthInput } from '@/features/auth/components/AuthForm'
import { AuthMessage } from '@/features/auth/components/AuthMessage'
import { useConsumedToken } from '@/features/auth/useConsumedToken'

export function RecoveryPage() {
  const token = useConsumedToken()
  const options = useAuthOptions()
  if (token) return <CompleteRecovery token={token} />
  // Without a mail server, only an administrator can create a recovery link.
  return options.data?.email_enabled ? <RequestRecovery /> : <AuthMessage {...recoveryRequestCopy} />
}

function RequestRecovery() {
  const mutation = useRequestRecovery()
  const [email, setEmail] = useState('')
  if (mutation.isSuccess) return <AuthMessage title="Check your email" detail={mutation.data.detail} />
  return (
    <AuthForm title="Reset your password" error={mutation.error} pending={mutation.isPending} submitLabel="Send reset link" onSubmit={() => mutation.mutateAsync({ email: email.trim() })} footer={<Link to="/login">Back to sign in</Link>}>
      <AuthInput label="Email" type="email" value={email} onChange={setEmail} />
    </AuthForm>
  )
}

function CompleteRecovery({ token }: { token: string }) {
  const navigate = useNavigate()
  const mutation = useCompleteRecovery()
  const [password, setPassword] = useState('')
  return (
    <AuthForm title="Choose a new password" error={mutation.error} pending={mutation.isPending} submitLabel="Reset password" onSubmit={async () => {
      await mutation.mutateAsync({ token, password })
      navigate('/login', { replace: true })
    }}>
      <AuthInput label="New password" type="password" value={password} onChange={setPassword} />
    </AuthForm>
  )
}
