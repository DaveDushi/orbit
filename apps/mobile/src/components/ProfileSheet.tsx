import { visibleEmail } from '@orbit/api/members'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import * as Clipboard from 'expo-clipboard'
import { router } from 'expo-router'
import { createContext, type ReactNode, use, useMemo, useRef, useState } from 'react'
import { Linking, Pressable, View } from 'react-native'

import * as chat from '../data/chat-api'
import { localTimeLabel, memberProfile, noteToSave, phoneHref, profileKey, roleLabel, saveMemberNote } from '../data/profiles'
import { useChatWrite, useIsBlocked, useSetBlocked } from '../hooks/chat'
import { useOrbit } from '../orbit-context'
import { useMembers } from '../queries'
import { useTheme } from '../theme'
import { Button, Icon, type IconName, Text } from '../ui'
import { TextInput } from '../ui/scaled'
import { ProfilePicture } from './settings/ProfilePicture'
import { Sheet } from './tasks/Sheet'

const ProfileContext = createContext<(userId: string, name: string) => void>(() => {})

/** Opens a member's profile card: `open(userId, name)`. */
export const useOpenProfile = () => use(ProfileContext)

/**
 * The web's profile popover at phone size: one sheet for the whole signed-in app, opened from a chat author, a chat
 * member or a workspace member. It shows the member's title, local time, email, phone, role and bio; others get
 * Message and the private note, you get Edit profile.
 */
export function ProfileSheetProvider({ children }: { children: ReactNode }) {
  const [target, setTarget] = useState<{ userId: string; name: string } | null>(null)
  const open = useMemo(() => (userId: string, name: string) => setTarget({ userId, name }), [])
  return (
    <ProfileContext value={open}>
      {children}
      <Sheet visible={target !== null} title={target?.name ?? ''} onClose={() => setTarget(null)}>
        {target ? <ProfileCard key={target.userId} userId={target.userId} name={target.name} onClose={() => setTarget(null)} /> : null}
      </Sheet>
    </ProfileContext>
  )
}

function ProfileCard({ userId, name, onClose }: { userId: string; name: string; onClose: () => void }) {
  const { client, workspaceId, user } = useOrbit()
  const members = useMembers()
  const member = members.data?.find((candidate) => candidate.user_id === userId)
  const own = userId === user.id
  const profile = useQuery({ queryKey: profileKey(workspaceId, userId), queryFn: () => memberProfile(client, workspaceId, userId), enabled: Boolean(member) })
  const run = useChatWrite()
  const isBlocked = useIsBlocked()
  const setBlocked = useSetBlocked()
  const [copied, setCopied] = useState(false)

  if (members.isPending) return <View className="h-24" />
  if (!member || profile.data === null) {
    return (
      <View className="px-4 pb-4">
        <Text className="text-[13px] text-muted-foreground">No longer in this workspace</Text>
      </View>
    )
  }
  const shown = own ? { title: user.title, phone: user.phone, timezone: user.timezone, bio: user.bio } : profile.data
  const localTime = localTimeLabel(shown?.timezone)
  // an agent's address is a placeholder: never shown
  const email = visibleEmail(member.email)
  const message = async () => {
    const dm = await run(`Could not open the conversation with ${member.display_name}`, (client, workspaceId) => chat.openDm(client, workspaceId, [userId]))
    if (!dm) return
    onClose()
    router.push(`/chat/${dm.id}`)
  }

  return (
    <View className="px-4 pb-4 gap-3">
      <View className="flex-row items-start gap-3">
        <ProfilePicture id={userId} name={member.display_name || name} url={member.avatar_url} size={64} />
        <View className="flex-1 min-w-0 gap-0.5 pt-1">
          {shown?.title ? <Text className="text-[13px] text-muted-foreground">{shown.title}</Text> : null}
          {profile.isError ? <Text className="text-[13px] text-destructive">The profile could not be loaded.</Text> : null}
        </View>
      </View>
      <View className="gap-1.5">
        {localTime ? <Detail icon="clock">{localTime}</Detail> : null}
        {/* a guest does not see the addresses of the people in its channels: the server sends none */}
        {email ? (
          <Detail
            icon="mail"
            label={`Copy email address ${email}`}
            onPress={() => void Clipboard.setStringAsync(email).then(() => setCopied(true))}
          >
            {copied ? 'Email copied' : email}
          </Detail>
        ) : null}
        {shown?.phone ? (
          <Detail icon="phone" label={`Call ${shown.phone}`} onPress={() => void Linking.openURL(phoneHref(shown.phone!)).catch(() => {})}>
            {shown.phone}
          </Detail>
        ) : null}
        <Detail icon="shield">{roleLabel(member.role)}</Detail>
      </View>
      {shown?.bio ? <Text className="text-[13px] leading-[19px] text-foreground/85">{shown.bio}</Text> : null}
      {own ? (
        <Button
          label="Edit profile"
          variant="outline"
          onPress={() => {
            onClose()
            router.push('/settings/profile')
          }}
        />
      ) : (
        <Button label="Message" onPress={() => void message()} />
      )}
      {/* blocking hides their chat messages from you, and they cannot write you a DM */}
      {own ? null : isBlocked(userId) ? (
        <Button label="Unblock" variant="outline" onPress={() => void setBlocked({ id: userId, name: member.display_name || name }, false)} />
      ) : (
        <Button label="Block" variant="outline" onPress={() => void setBlocked({ id: userId, name: member.display_name || name }, true)} />
      )}
      {!own && profile.data ? <Note userId={userId} name={member.display_name} saved={profile.data.note ?? ''} /> : null}
    </View>
  )
}

function Detail({ icon, label, onPress, children }: { icon: IconName; label?: string; onPress?: () => void; children: ReactNode }) {
  const { colors } = useTheme()
  const row = (
    <View className="flex-row items-center gap-2 min-h-6">
      <Icon name={icon} size={15} color={colors.mutedForeground} />
      <Text numberOfLines={1} className="flex-1 text-[13px]">
        {children}
      </Text>
    </View>
  )
  return onPress ? (
    <Pressable accessibilityRole="button" accessibilityLabel={label} onPress={onPress} className="rounded-md active:bg-muted">
      {row}
    </Pressable>
  ) : (
    row
  )
}

/** The private note about a member: only the writer sees it. It saves when the field loses the focus. */
function Note({ userId, name, saved }: { userId: string; name: string; saved: string }) {
  const { client, workspaceId } = useOrbit()
  const { colors } = useTheme()
  const queryClient = useQueryClient()
  const [draft, setDraft] = useState(saved)
  const [error, setError] = useState<string | null>(null)
  const saving = useRef(false)
  const save = () => {
    const body = noteToSave(saved, draft)
    if (body === null || saving.current) return
    saving.current = true
    setError(null)
    saveMemberNote(client, workspaceId, userId, body).then(
      () => void queryClient.invalidateQueries({ queryKey: profileKey(workspaceId, userId) }),
      () => setError('The note was not saved. Try again.'),
    ).finally(() => (saving.current = false))
  }
  return (
    <View className="gap-1">
      <Text className="font-noto-medium text-[12px] text-muted-foreground">Note (only you see it)</Text>
      <TextInput
        accessibilityLabel={`Private note about ${name}`}
        value={draft}
        onChangeText={setDraft}
        onBlur={save}
        placeholder="Add a note"
        placeholderTextColor={colors.mutedForeground}
        multiline
        maxLength={1000}
        className="font-noto text-[14px] leading-[20px] min-h-16 max-h-32 px-2.5 py-2 rounded-md border border-input text-foreground"
      />
      {error ? <Text className="text-[12px] text-destructive">{error}</Text> : null}
    </View>
  )
}
