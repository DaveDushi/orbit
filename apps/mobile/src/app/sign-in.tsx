import { useState } from 'react'
import { KeyboardAvoidingView, Linking, Platform, ScrollView, View } from 'react-native'

import { createClient, normalizeServer, signIn as signInWithPassword, signInSecondFactor } from '../data/orbit'
import { errorMessage, useSession } from '../session-context'
import { alpha, useTheme } from '../theme'
import { Button, Input, Text } from '../ui'

type Step = { name: 'credentials' } | { name: 'two_factor'; server: string; twoFactorToken: string }

const DEFAULT_SERVER = process.env.EXPO_PUBLIC_ORBIT_SERVER ?? ''

export default function SignInScreen() {
  const { state, signIn } = useSession()
  const { colors } = useTheme()
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

  const credentials = step.name === 'credentials'

  return (
    <KeyboardAvoidingView
      className="flex-1 bg-background"
      behavior={Platform.OS === 'ios' ? 'padding' : undefined}
      // the web sign-in's soft accent glow from the top edge
      style={{ backgroundImage: `radial-gradient(ellipse 60% 45% at 50% 0%, ${alpha(colors.primary, 0.14)}, transparent)` }}
    >
      <ScrollView contentContainerClassName="grow justify-center p-6" keyboardShouldPersistTaps="handled">
        {/* the web `AuthForm`: a header, then an inset panel with the fields and actions */}
        <View className="w-full max-w-115 self-center rounded-xl border border-border bg-card shadow-lg">
          <View className="gap-1 px-6 pt-6 pb-5">
            <Text className="font-noto-semibold text-[20px] leading-[28px]">{credentials ? 'Orbit' : 'Two-factor code'}</Text>
            <Text className="text-[13px] leading-[19.5px] text-muted-foreground">
              {credentials ? 'Sign in to your tasks, docs, mail and chat.' : 'Enter the 6-digit code from your authenticator app, or a recovery code.'}
            </Text>
          </View>
          <View className="-mx-px -mb-px gap-4 rounded-xl border border-border bg-background/40 p-6">
            {step.name === 'credentials' ? (
              <>
                <Input label="Server" required placeholder="orbit.example.com" autoCapitalize="none" autoCorrect={false} keyboardType="url" textContentType="URL" value={server} onChangeText={setServer} />
                <Input label="Email" required placeholder="you@example.com" autoCapitalize="none" autoComplete="email" keyboardType="email-address" textContentType="username" value={email} onChangeText={setEmail} />
                <Input
                  label="Password"
                  required
                  secureTextEntry
                  autoComplete="current-password"
                  textContentType="password"
                  value={password}
                  onChangeText={setPassword}
                  returnKeyType="go"
                  onSubmitEditing={() => server && email && password && submitCredentials()}
                  error={error}
                />
                {server.trim() ? (
                  <Text accessibilityRole="link" onPress={() => void Linking.openURL(`${normalizeServer(server)}/recovery`)} className="self-end text-[13px] text-muted-foreground">
                    Forgot password?
                  </Text>
                ) : null}
                <Button label="Sign in" size="lg" block loading={busy} disabled={!server || !email || !password} onPress={submitCredentials} />
              </>
            ) : (
              <>
                <Input
                  label="Code"
                  required
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
                <View className="gap-3">
                  <Button label="Verify" size="lg" block loading={busy} disabled={!code.trim()} onPress={() => submitCode(step)} />
                  <Button
                    label="Back"
                    variant="outline"
                    size="lg"
                    block
                    disabled={busy}
                    onPress={() => {
                      setStep({ name: 'credentials' })
                      setCode('')
                      setError(undefined)
                    }}
                  />
                </View>
              </>
            )}
          </View>
        </View>
      </ScrollView>
    </KeyboardAvoidingView>
  )
}
