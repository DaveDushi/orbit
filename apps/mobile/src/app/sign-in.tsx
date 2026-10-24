import { useState } from 'react'
import { KeyboardAvoidingView, Linking, Platform, ScrollView, View } from 'react-native'
import { SvgXml } from 'react-native-svg'

import { createClient, normalizeServer, signIn as signInWithPassword, signInSecondFactor } from '../data/orbit'
import { errorMessage, useSession } from '../session-context'
import { Button, Input, Text } from '../ui'
import { logo } from '../ui/icons.gen'

type Step = { name: 'credentials' } | { name: 'two_factor'; server: string; twoFactorToken: string }

const DEFAULT_SERVER = process.env.EXPO_PUBLIC_ORBIT_SERVER ?? ''

export default function SignInScreen() {
  const { state, signIn } = useSession()
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
    <KeyboardAvoidingView className="flex-1 bg-background" behavior={Platform.OS === 'ios' ? 'padding' : undefined}>
      <ScrollView contentContainerClassName="grow justify-center p-6" keyboardShouldPersistTaps="handled">
        <View
          className="w-full max-w-105 self-center gap-4.5 rounded-xl border border-border bg-card p-8 shadow-lg"
        >
          <View className="flex-row items-center gap-2">
            <SvgXml xml={logo} width={22} height={22} />
            <Text className="font-noto-bold text-[13px] tracking-[1.04px] text-primary">ORBIT</Text>
          </View>
          <View className="mt-4 mb-2 gap-1">
            <Text className="font-noto-semibold text-[24px] leading-[28px] tracking-[-0.6px]">{credentials ? 'Sign in to Orbit' : 'Two-factor code'}</Text>
            {credentials ? null : <Text className="text-[14px] text-muted-foreground">Enter the 6-digit code from your authenticator app, or a recovery code.</Text>}
          </View>
          {step.name === 'credentials' ? (
            <>
              <View className="gap-3.5">
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
              </View>
              <Button label="Sign in" block loading={busy} disabled={!server || !email || !password} onPress={submitCredentials} />
              {server.trim() ? (
                <Text accessibilityRole="link" onPress={() => void Linking.openURL(`${normalizeServer(server)}/recovery`)} className="text-[13px] text-center">
                  Forgot your password?
                </Text>
              ) : null}
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
              <Button label="Verify" block loading={busy} disabled={!code.trim()} onPress={() => submitCode(step)} />
              <Button
                label="Back"
                variant="ghost"
                block
                disabled={busy}
                onPress={() => {
                  setStep({ name: 'credentials' })
                  setCode('')
                  setError(undefined)
                }}
              />
            </>
          )}
        </View>
      </ScrollView>
    </KeyboardAvoidingView>
  )
}
