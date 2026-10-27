import { useState } from 'react'
import { View } from 'react-native'

import { ConfirmDialog, type ConfirmRequest } from '../../../../components/settings/Dialog'
import { AdminShell, FieldLabel, Note, OptionHint, OptionText, Segmented, SettingsCard, SettingsRow, Toggle } from '../../../../components/settings/Shell'
import { limitsBody, limitsDraft, type LimitsView, SMTP_DEFAULT_PORT, SMTP_SECURITY_OPTIONS, smtpBody, smtpDraft, type SmtpView } from '../../../../data/admin-api'
import { useInstanceSettings, useRemoveSmtp, useSaveLimits, useSaveSmtp, useSendTestEmail, useSetRegistration } from '../../../../hooks/admin'
import { useOrbit } from '../../../../orbit-context'
import { errorMessage } from '../../../../session-context'
import { Button, Input, Text } from '../../../../ui'

/**
 * The web's Admin → General: who can create an account, the mail server for invitations, sign-up and password reset,
 * and the limits per account and workspace.
 */
export default function AdminGeneralScreen() {
  const settings = useInstanceSettings()
  const smtp = settings.data?.smtp ?? null
  return (
    <AdminShell refreshing={settings.isRefetching} onRefresh={() => void settings.refetch()}>
      {settings.isPending ? <SettingsCard title="General"><Note>Loading settings…</Note></SettingsCard> : null}
      {settings.isError ? (
        <SettingsCard title="General">
          <View className="flex-row items-center gap-2">
            <Note error>Settings could not be loaded.</Note>
            <Button label="Retry" variant="ghost" onPress={() => void settings.refetch()} />
          </View>
        </SettingsCard>
      ) : null}
      {settings.data ? (
        <>
          <RegistrationCard open={settings.data.registration_open} emailReady={smtp !== null} />
          <EmailCard smtp={smtp} />
          <LimitsCard limits={settings.data.limits} />
        </>
      ) : null}
    </AdminShell>
  )
}

function RegistrationCard({ open, emailReady }: { open: boolean; emailReady: boolean }) {
  const setRegistration = useSetRegistration()
  return (
    <SettingsCard title="Registration" description="Who can create an account on this Orbit." flush>
      <SettingsRow first>
        <OptionText title="Open registration">
          <OptionHint>
            {emailReady
              ? 'On: anyone can create an account with a link sent to their email. Off: people join only by invitation.'
              : 'Save a mail server below first: the sign-up link goes out by email.'}
          </OptionHint>
          {setRegistration.isError ? <OptionHint error>{errorMessage(setRegistration.error)}</OptionHint> : null}
        </OptionText>
        <Toggle label="Open registration" value={open} disabled={(!emailReady && !open) || setRegistration.isPending} onChange={(checked) => setRegistration.mutate(checked)} />
      </SettingsRow>
    </SettingsCard>
  )
}

function EmailCard({ smtp }: { smtp: SmtpView | null }) {
  const { user } = useOrbit()
  const save = useSaveSmtp()
  const remove = useRemoveSmtp()
  const test = useSendTestEmail()
  const [draft, setDraft] = useState(() => smtpDraft(smtp))
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null)
  const edit = (change: Partial<typeof draft>) => {
    save.reset()
    setDraft((current) => ({ ...current, ...change }))
  }
  const set = (field: keyof typeof draft) => (value: string) => edit({ [field]: value })
  // after a save or a removal the form shows what the server stored
  const stored = { onSuccess: (settings: { smtp?: SmtpView | null }) => setDraft(smtpDraft(settings.smtp ?? null)) }
  const ready = draft.host.trim() !== '' && draft.port.trim() !== '' && draft.from_address.trim() !== ''

  return (
    <SettingsCard
      title="Email"
      description="The SMTP server Orbit sends invitations, sign-up links and password reset links through. Nothing else is sent."
      actions={
        smtp ? (
          <>
            <Button label={test.isPending ? 'Sending…' : 'Send test email'} variant="outline" size="sm" disabled={test.isPending} onPress={() => test.mutate()} />
            <Button
              label="Remove"
              variant="destructive"
              size="sm"
              disabled={remove.isPending}
              onPress={() =>
                setConfirm({
                  title: 'Remove the mail server?',
                  description: 'Orbit stops sending invitations and password reset links by email, and registration closes.',
                  confirmLabel: 'Remove',
                  danger: true,
                  onConfirm: () => remove.mutate(undefined, stored),
                })
              }
            />
          </>
        ) : undefined
      }
    >
      <View className="gap-4">
        <Input label="Host" required placeholder="smtp.example.com" value={draft.host} onChangeText={set('host')} autoCapitalize="none" autoCorrect={false} keyboardType="url" />
        <View className="gap-2">
          <FieldLabel>Encryption</FieldLabel>
          <Segmented
            label="Encryption"
            value={draft.security}
            options={SMTP_SECURITY_OPTIONS}
            onChange={(security) => edit({ security, port: String(SMTP_DEFAULT_PORT[security]) })}
          />
          <Text className="text-[12px] text-muted-foreground">
            {draft.security === 'none' ? 'Only for a mail relay on the same network.' : `Usually port ${SMTP_DEFAULT_PORT[draft.security]}.`}
          </Text>
        </View>
        <Input label="Port" required keyboardType="number-pad" value={draft.port} onChangeText={set('port')} />
        <Input label="Username" value={draft.username} onChangeText={set('username')} autoCapitalize="none" autoCorrect={false} autoComplete="off" />
        <Input
          label="Password"
          secureTextEntry
          autoComplete="new-password"
          placeholder={smtp?.password_set ? 'Saved. Leave empty to keep it.' : undefined}
          value={draft.password}
          onChangeText={set('password')}
        />
        <Input label="From address" required placeholder="orbit@example.com" keyboardType="email-address" autoCapitalize="none" autoCorrect={false} value={draft.from_address} onChangeText={set('from_address')} />
        <Input label="From name" placeholder="Orbit" value={draft.from_name} onChangeText={set('from_name')} />
        {save.isError ? <Note error>{errorMessage(save.error)}</Note> : null}
        {remove.isError ? <Note error>{errorMessage(remove.error)}</Note> : null}
        {test.isError ? <Note error>{errorMessage(test.error)}</Note> : null}
        {test.isSuccess ? <Note>Test email sent to {user.email}.</Note> : null}
        {save.isSuccess ? <Note>Mail settings saved.</Note> : null}
        <View className="flex-row justify-end">
          <Button label={save.isPending ? 'Saving…' : 'Save mail settings'} disabled={save.isPending || !ready} onPress={() => save.mutate(smtpBody(draft), stored)} />
        </View>
      </View>
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </SettingsCard>
  )
}

function LimitsCard({ limits }: { limits: LimitsView }) {
  const save = useSaveLimits()
  const [draft, setDraft] = useState(() => limitsDraft(limits))
  const [invalid, setInvalid] = useState(false)
  const edit = (change: Partial<typeof draft>) => {
    save.reset()
    setDraft((current) => ({ ...current, ...change }))
  }

  const submit = () => {
    const body = limitsBody(draft)
    setInvalid(body === null)
    if (body) save.mutate(body, { onSuccess: (settings) => setDraft(limitsDraft(settings.limits)) })
  }

  return (
    <SettingsCard title="Limits" description="What one account and one workspace may use. Empty or 0 means no limit. A lower limit removes nothing that exists.">
      <View className="gap-4">
        <Input
          label="Workspaces per account"
          keyboardType="number-pad"
          placeholder="No limit"
          value={draft.workspaces}
          onChangeText={(workspaces) => edit({ workspaces })}
          description="Workspaces one account may own, not counting the trash. Admins have no limit."
        />
        <Input
          label="Storage per workspace (GB)"
          keyboardType="decimal-pad"
          placeholder="No limit"
          value={draft.storageGb}
          onChangeText={(storageGb) => edit({ storageGb })}
          description="Files, emoji and stickers of one workspace, each file counted once."
        />
        {invalid ? <Note error>Enter a number, or leave the field empty for no limit.</Note> : null}
        {save.isError ? <Note error>{errorMessage(save.error)}</Note> : null}
        {save.isSuccess ? <Note>Limits saved.</Note> : null}
        <View className="flex-row justify-end">
          <Button label={save.isPending ? 'Saving…' : 'Save limits'} disabled={save.isPending} onPress={submit} />
        </View>
      </View>
    </SettingsCard>
  )
}
