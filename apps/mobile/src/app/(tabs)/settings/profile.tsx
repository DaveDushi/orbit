import { useMutation } from '@tanstack/react-query'
import { useState } from 'react'
import { View } from 'react-native'

import { ProfilePicture } from '../../../components/settings/ProfilePicture'
import { ACCOUNT_NAV, Note, SettingsCard, SettingsShell } from '../../../components/settings/Shell'
import { passwordProblem } from '../../../data/account-api'
import * as api from '../../../data/orbit'
import { phoneValid, profileChanges, profileFormOf, type ProfileForm } from '../../../data/profile'
import { pickImage, useChangePassword, useSetProfilePicture } from '../../../hooks/account'
import { useOrbit } from '../../../orbit-context'
import { errorMessage } from '../../../session-context'
import { type, useTheme } from '../../../theme'
import { Button, Input, Text } from '../../../ui'

/** The server refuses a bigger picture. */
const MAX_PICTURE_BYTES = 512 * 1024
const BIO_LIMIT = 500
/** The bio counter shows from this many characters before the limit. */
const BIO_COUNTER_FROM = 100

/** The web's Profile page: the picture, the details other members see, and the password. */
export default function ProfileScreen() {
  const { client, user, setUser } = useOrbit()
  const { colors } = useTheme()
  const saved = profileFormOf(user)
  const [form, setForm] = useState<ProfileForm>(saved)
  const set = (part: Partial<ProfileForm>) => setForm((current) => ({ ...current, ...part }))
  const changes = profileChanges(user, form)
  const phoneInvalid = form.phone.trim() !== '' && !phoneValid(form.phone.trim())
  const bioLeft = BIO_LIMIT - form.bio.length

  const save = useMutation({
    mutationFn: (body: api.UpdateMeBody) => api.updateProfile(client, body),
    onSuccess: (updated) => {
      setUser(updated)
      setForm(profileFormOf(updated))
    },
  })

  return (
    <SettingsShell title="Account settings" nav={ACCOUNT_NAV}>
      <SettingsCard title="Profile details" description="Your picture, name and what the people in your workspaces see of you.">
        <PictureField />
        <View style={{ marginTop: 16, gap: 16 }}>
          <Input label="Name" required value={form.name} onChangeText={(name) => set({ name })} autoComplete="name" />
          <Input label="Email" value={user.email} editable={false} />
          <Input label="Title" value={form.title} onChangeText={(title) => set({ title })} placeholder="Product designer" maxLength={100} />
          <Input label="Pronouns" value={form.pronouns} onChangeText={(pronouns) => set({ pronouns })} placeholder="she/her" maxLength={40} />
          <Input
            label="Phone number"
            value={form.phone}
            onChangeText={(phone) => set({ phone })}
            placeholder="+1 555 010 0000"
            keyboardType="phone-pad"
            autoComplete="tel"
            description="The people in your workspaces can see it."
            error={phoneInvalid ? 'Use digits, spaces and + - ( ) . only.' : undefined}
          />
          <Input label="Time zone" value={form.timezone} onChangeText={(timezone) => set({ timezone })} placeholder="Europe/Budapest" autoCapitalize="none" autoCorrect={false} />
          <View style={{ gap: 4 }}>
            <Input label="Bio" value={form.bio} onChangeText={(bio) => set({ bio })} multiline maxLength={BIO_LIMIT} />
            {bioLeft <= BIO_COUNTER_FROM ? <Text style={[type(12), { textAlign: 'right', color: colors.mutedForeground }]}>{`${form.bio.length} / ${BIO_LIMIT}`}</Text> : null}
          </View>
        </View>
        {save.isError ? <View style={{ marginTop: 12 }}><Note error>{errorMessage(save.error)}</Note></View> : null}
        {changes ? (
          <View style={{ marginTop: 16, flexDirection: 'row', gap: 8, justifyContent: 'flex-end' }}>
            <Button label="Discard" variant="outline" onPress={() => setForm(saved)} />
            <Button label="Save" loading={save.isPending} disabled={phoneInvalid} onPress={() => save.mutate(changes)} />
          </View>
        ) : null}
      </SettingsCard>

      <PasswordCard />
    </SettingsShell>
  )
}

function PictureField() {
  const { user } = useOrbit()
  const setPicture = useSetProfilePicture()
  const [pickError, setPickError] = useState<string | null>(null)
  const choose = () => {
    setPickError(null)
    setPicture.reset()
    pickImage({ square: true, maxBytes: MAX_PICTURE_BYTES }).then(
      (file) => file && setPicture.mutate(file),
      (error: unknown) => setPickError(errorMessage(error)),
    )
  }
  const error = pickError ?? (setPicture.isError ? errorMessage(setPicture.error) : null)
  return (
    <View style={{ flexDirection: 'row', alignItems: 'center', gap: 16, flexWrap: 'wrap' }}>
      <ProfilePicture id={user.id} name={user.display_name} url={user.avatar_url} size={64} />
      <View style={{ flex: 1, minWidth: 180, gap: 8 }}>
        <View style={{ flexDirection: 'row', gap: 8 }}>
          <Button label={setPicture.isPending ? 'Saving…' : user.avatar_url ? 'Change picture' : 'Upload picture'} variant="outline" disabled={setPicture.isPending} onPress={choose} />
          {user.avatar_url ? <Button label="Remove" variant="ghost" disabled={setPicture.isPending} onPress={() => setPicture.mutate(null)} /> : null}
        </View>
        <Note error={error !== null}>{error ?? 'Cropped to a square. People in your workspaces see it.'}</Note>
      </View>
    </View>
  )
}

function PasswordCard() {
  const change = useChangePassword()
  const [current, setCurrent] = useState('')
  const [next, setNext] = useState('')
  const [confirm, setConfirm] = useState('')
  const [problem, setProblem] = useState<string | null>(null)
  const [done, setDone] = useState(false)
  const submit = () => {
    const found = passwordProblem(current, next, confirm)
    setProblem(found)
    setDone(false)
    if (found) return
    change.mutate(
      { current, next },
      {
        onSuccess: () => {
          setCurrent('')
          setNext('')
          setConfirm('')
          setDone(true)
        },
      },
    )
  }
  const alert = problem ?? (change.isError ? errorMessage(change.error) : null)
  return (
    <SettingsCard title="Password" description="Other devices will be signed out." actions={<Button label={change.isPending ? 'Changing…' : 'Change password'} variant="outline" disabled={change.isPending} onPress={submit} />}>
      <View style={{ gap: 16 }}>
        <Input label="Current password" required value={current} onChangeText={setCurrent} secureTextEntry autoComplete="current-password" autoCapitalize="none" />
        <Input label="New password" required value={next} onChangeText={setNext} secureTextEntry autoComplete="new-password" autoCapitalize="none" />
        <Input label="Confirm new password" required value={confirm} onChangeText={setConfirm} secureTextEntry autoComplete="new-password" autoCapitalize="none" returnKeyType="done" onSubmitEditing={submit} />
        {alert ? <Note error>{alert}</Note> : done ? <Note>Your password is changed. Your other devices are signed out.</Note> : null}
      </View>
    </SettingsCard>
  )
}
