import { useState } from 'react'
import { Image, Linking, Share, View } from 'react-native'

import { Dialog, PromptDialog, type PromptRequest } from '../../../components/settings/Dialog'
import { ACCOUNT_NAV, Badge, Note, RowIcon, RowText, SettingsCard, SettingsRow, SettingsShell } from '../../../components/settings/Shell'
import { type TotpSetup, twoFactorSummary } from '../../../data/account-api'
import { useTwoFactor, useTwoFactorWrites } from '../../../hooks/account'
import { errorMessage } from '../../../session-context'
import { Button, Input, Text } from '../../../ui'

/**
 * The web's Security page: the authenticator app (set up, new recovery codes, turn off), each step confirmed with the
 * password. Passkeys need the browser's WebAuthn prompt, which Expo Go does not offer, so they stay on the web.
 */
export default function SecurityScreen() {
  const status = useTwoFactor()
  const writes = useTwoFactorWrites()
  const [prompt, setPrompt] = useState<PromptRequest | null>(null)
  const [setup, setSetup] = useState<TotpSetup | null>(null)
  const [codes, setCodes] = useState<{ title: string; codes: string[] } | null>(null)
  const enabled = status.data?.totp_enabled === true

  const askPassword = (title: string, description: string, confirmLabel: string, then: (password: string) => Promise<unknown>) =>
    setPrompt({ title, description, label: 'Current password', confirmLabel, secure: true, onSubmit: then })

  return (
    <SettingsShell title="Account settings" nav={ACCOUNT_NAV} refreshing={status.isRefetching} onRefresh={() => void status.refetch()}>
      <SettingsCard
        title="Authenticator app"
        description="Sign-in asks for a code from an app such as 1Password, Authy or Google Authenticator after your password."
        actions={
          status.data ? (
            enabled ? (
              <>
                <Button
                  label="New recovery codes"
                  variant="outline"
                  onPress={() =>
                    askPassword('New recovery codes', 'Your current recovery codes stop working.', 'Make new codes', async (password) =>
                      setCodes({ title: 'New recovery codes', codes: await writes.regenerate.mutateAsync(password) }),
                    )
                  }
                />
                <Button
                  label="Turn off"
                  variant="destructive"
                  onPress={() =>
                    askPassword('Turn off two-factor sign-in?', 'Sign-in asks only for your password again, and your recovery codes stop working.', 'Turn off', (password) =>
                      writes.disable.mutateAsync(password),
                    )
                  }
                />
              </>
            ) : (
              <Button
                label="Set up"
                variant="outline"
                onPress={() => askPassword('Set up an authenticator app', 'Confirm your password to start.', 'Continue', async (password) => setSetup(await writes.setup.mutateAsync(password)))}
              />
            )
          ) : undefined
        }
        flush
      >
        {status.isPending ? <View className="p-4"><Note>Loading…</Note></View> : null}
        {status.isError ? <View className="p-4"><Note error>Two-factor sign-in could not be loaded.</Note></View> : null}
        {status.data ? (
          <SettingsRow first>
            <RowIcon icon="mobile" />
            <RowText title="Two-factor sign-in" subtitle={twoFactorSummary(status.data)} badges={<Badge label={enabled ? 'On' : 'Off'} tone={enabled ? 'primary' : 'secondary'} />} />
          </SettingsRow>
        ) : null}
      </SettingsCard>

      <SettingsCard title="Passkeys" description="Sign in with your fingerprint, face or device screen lock instead of a password.">
        <Note>Add and remove passkeys in Orbit in a browser: creating one needs the browser's passkey prompt.</Note>
      </SettingsCard>

      <PromptDialog request={prompt} onClose={() => setPrompt(null)} />
      <SetupDialog
        setup={setup}
        onEnabled={(list) => {
          setSetup(null)
          setCodes({ title: 'Two-factor sign-in is on', codes: list })
        }}
        onClose={() => setSetup(null)}
      />
      <RecoveryCodesDialog codes={codes} onClose={() => setCodes(null)} />
    </SettingsShell>
  )
}

/** Step two of the setup: the QR code (for another device), the key, the authenticator app link, then the code. */
function SetupDialog({ setup, onEnabled, onClose }: { setup: TotpSetup | null; onEnabled: (codes: string[]) => void; onClose: () => void }) {
  const { enable } = useTwoFactorWrites()
  const [code, setCode] = useState('')
  const submit = () => {
    if (!code.trim()) return
    enable.mutate(code.trim(), {
      onSuccess: (codes) => {
        setCode('')
        onEnabled(codes)
      },
    })
  }
  return (
    <Dialog
      visible={setup !== null}
      title="Add Orbit to your authenticator app"
      description="Open the setup link in an authenticator app on this phone, or scan the QR code from another device, or enter the key by hand. Then enter the code the app shows."
      onClose={onClose}
      footer={
        <>
          <Button label="Cancel" variant="outline" onPress={onClose} />
          <Button label={enable.isPending ? 'Turning on…' : 'Turn on'} disabled={enable.isPending || !code.trim()} onPress={submit} />
        </>
      }
    >
      {setup ? (
        <>
          <Image accessibilityLabel="QR code for your authenticator app" source={{ uri: setup.qr_code }} className="size-42 self-center rounded-md bg-white" />
          <Button label="Open in authenticator app" icon="key" variant="outline" onPress={() => void Linking.openURL(setup.otpauth_url).catch(() => undefined)} />
          <CopyValue label="Setup key" value={setup.secret} />
          <Input label="Code from the app" value={code} onChangeText={setCode} keyboardType="number-pad" autoComplete="one-time-code" textContentType="oneTimeCode" returnKeyType="done" onSubmitEditing={submit} error={enable.isError ? errorMessage(enable.error) : undefined} />
        </>
      ) : null}
    </Dialog>
  )
}

function RecoveryCodesDialog({ codes, onClose }: { codes: { title: string; codes: string[] } | null; onClose: () => void }) {
  const text = codes ? `Orbit recovery codes\nEach code signs in once in place of an authenticator app code.\n\n${codes.codes.join('\n')}\n` : ''
  return (
    <Dialog
      visible={codes !== null}
      title={codes?.title ?? ''}
      description="Save these recovery codes somewhere safe. Each one signs you in once if you lose your authenticator app. They are shown only now."
      onClose={onClose}
      footer={
        <>
          <Button label="Share" icon="send" variant="outline" onPress={() => void Share.share({ message: text })} />
          <Button label="Done" onPress={onClose} />
        </>
      }
    >
      <View accessibilityLabel="Recovery codes" className="flex-row flex-wrap gap-y-1.5 p-4 rounded-md bg-muted">
        {codes?.codes.map((code) => (
          <Text key={code} selectable className="w-[50%] font-mono text-[13px] text-foreground leading-[21px]">
            {code}
          </Text>
        ))}
      </View>
    </Dialog>
  )
}

/** A read-only value to select or share (the app has no clipboard module). */
function CopyValue({ label, value }: { label: string; value: string }) {
  return (
    <View className="gap-2">
      <Text className="font-noto-medium text-[14px]">{label}</Text>
      <View className="flex-row items-center gap-2">
        <Text selectable className="flex-1 font-mono text-[13px] text-foreground p-2 rounded-md border border-input leading-[21px]">
          {value}
        </Text>
        <Button label="Share" variant="outline" size="sm" onPress={() => void Share.share({ message: value })} />
      </View>
    </View>
  )
}
