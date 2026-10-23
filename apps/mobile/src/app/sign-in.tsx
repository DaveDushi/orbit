import { useState } from 'react'
import { KeyboardAvoidingView, Linking, Platform, ScrollView, View } from 'react-native'
import { SvgXml } from 'react-native-svg'

import { createClient, normalizeServer, signIn as signInWithPassword, signInSecondFactor } from '../data/orbit'
import { errorMessage, useSession } from '../session-context'
import { alpha, cssColors, type, useTheme } from '../theme'
import { Button, Input, Text } from '../ui'
import { logo } from '../ui/icons.gen'

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
    <KeyboardAvoidingView style={{ flex: 1, backgroundColor: colors.background }} behavior={Platform.OS === 'ios' ? 'padding' : undefined}>
      <ScrollView contentContainerStyle={{ flexGrow: 1, justifyContent: 'center', padding: 24 }} keyboardShouldPersistTaps="handled">
        <View
          style={{
            width: '100%',
            maxWidth: 420,
            alignSelf: 'center',
            padding: 32,
            gap: 18,
            borderRadius: 14,
            borderWidth: 1,
            borderColor: colors.border,
            backgroundColor: colors.card,
            boxShadow: `0 10px 15px -3px ${alpha(cssColors.black, 0.1)}, 0 4px 6px -4px ${alpha(cssColors.black, 0.1)}`,
          }}
        >
          <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
            <SvgXml xml={logo} width={22} height={22} />
            <Text style={[type(13, 'bold'), { letterSpacing: 1.04, color: colors.primary }]}>ORBIT</Text>
          </View>
          <View style={{ marginTop: 16, marginBottom: 8, gap: 4 }}>
            <Text style={{ ...type(24, 'semibold'), lineHeight: 28, letterSpacing: -0.6 }}>{credentials ? 'Sign in to Orbit' : 'Two-factor code'}</Text>
            {credentials ? null : <Text style={[type(14), { color: colors.mutedForeground }]}>Enter the 6-digit code from your authenticator app, or a recovery code.</Text>}
          </View>
          {step.name === 'credentials' ? (
            <>
              <View style={{ gap: 14 }}>
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
                <Text accessibilityRole="link" onPress={() => void Linking.openURL(`${normalizeServer(server)}/recovery`)} style={[type(13), { textAlign: 'center' }]}>
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
