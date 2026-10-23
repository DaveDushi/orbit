import { useState } from 'react'
import { KeyboardAvoidingView, Platform, StyleSheet, View } from 'react-native'

import { createClient, normalizeServer, signIn as signInWithPassword, signInSecondFactor } from '../data/orbit'
import { errorMessage, useSession } from '../session-context'
import { useTheme } from '../theme'
import { Button, Input, Screen, Text } from '../ui'

type Step = { name: 'credentials' } | { name: 'two_factor'; server: string; twoFactorToken: string }

const DEFAULT_SERVER = process.env.EXPO_PUBLIC_ORBIT_SERVER ?? ''

export default function SignInScreen() {
  const { state, signIn } = useSession()
  const { spacing } = useTheme()
  const [step, setStep] = useState<Step>({ name: 'credentials' })
  const [server, setServer] = useState(DEFAULT_SERVER)
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [code, setCode] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState(state.status === 'signed_out' ? state.error : undefined)

  const run = async (action: () => Promise<void>) => {
    setBusy(true)
    setError(undefined)
    try {
      await action()
    } catch (cause) {
      setError(errorMessage(cause))
    } finally {
      setBusy(false)
    }
  }

  const submitCredentials = () =>
    run(async () => {
      const normalized = normalizeServer(server)
      const result = await signInWithPassword(createClient(normalized, () => null), email.trim(), password)
      if (result.kind === 'two_factor') setStep({ name: 'two_factor', server: normalized, twoFactorToken: result.twoFactorToken })
      else await signIn({ server: normalized, token: result.token }, result.user)
    })

  const submitCode = (current: Extract<Step, { name: 'two_factor' }>) =>
    run(async () => {
      const result = await signInSecondFactor(createClient(current.server, () => null), current.twoFactorToken, code.trim())
      if (result.kind === 'signed_in') await signIn({ server: current.server, token: result.token }, result.user)
    })

  return (
    <KeyboardAvoidingView style={styles.fill} behavior={Platform.OS === 'ios' ? 'padding' : undefined}>
      <Screen scroll centered>
        <View style={{ gap: spacing[1], marginBottom: spacing[3] }}>
          <Text variant="title">{step.name === 'credentials' ? 'Sign in to Orbit' : 'Two-factor code'}</Text>
          <Text color="muted">
            {step.name === 'credentials' ? 'Use the address of your Orbit server.' : 'Enter the 6-digit code from your authenticator app, or a recovery code.'}
          </Text>
        </View>
        {step.name === 'credentials' ? (
          <>
            <Input label="Server" placeholder="orbit.example.com" autoCapitalize="none" autoCorrect={false} keyboardType="url" textContentType="URL" value={server} onChangeText={setServer} />
            <Input label="Email" placeholder="you@example.com" autoCapitalize="none" autoComplete="email" keyboardType="email-address" textContentType="username" value={email} onChangeText={setEmail} />
            <Input
              label="Password"
              secureTextEntry
              autoComplete="current-password"
              textContentType="password"
              value={password}
              onChangeText={setPassword}
              returnKeyType="go"
              onSubmitEditing={() => server && email && password && submitCredentials()}
              error={error}
            />
            <Button label="Sign in" loading={busy} disabled={!server || !email || !password} onPress={submitCredentials} />
          </>
        ) : (
          <>
            <Input
              label="Code"
              placeholder="123456"
              autoCapitalize="none"
              autoComplete="one-time-code"
              textContentType="oneTimeCode"
              autoFocus
              value={code}
              onChangeText={setCode}
              returnKeyType="go"
              onSubmitEditing={() => code && submitCode(step)}
              error={error}
            />
            <Button label="Verify" loading={busy} disabled={!code.trim()} onPress={() => submitCode(step)} />
            <Button
              label="Back"
              variant="ghost"
              disabled={busy}
              onPress={() => {
                setStep({ name: 'credentials' })
                setCode('')
                setError(undefined)
              }}
            />
          </>
        )}
      </Screen>
    </KeyboardAvoidingView>
  )
}

const styles = StyleSheet.create({ fill: { flex: 1 } })
