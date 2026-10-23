import { StyleSheet, View } from 'react-native'

import { fonts, useTheme } from '../theme'
import { Text } from './Text'

/** Mid-tone hues that read on both the light and dark muted surface. */
const HUES = ['#f2458f', '#f97316', '#eab308', '#22c55e', '#14b8a6', '#3b82f6', '#8b5cf6', '#ec4899'] as const

function hueFor(id: string) {
  let hash = 0
  for (let index = 0; index < id.length; index++) hash = (hash * 31 + id.charCodeAt(index)) | 0
  return HUES[Math.abs(hash) % HUES.length]
}

const initialsOf = (name: string) =>
  name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((part) => part[0]!.toUpperCase())
    .join('') || '?'

export interface AvatarProps {
  id: string
  name: string
  size?: number
  /** The member colour from the server, when known. Otherwise one is derived from `id`. */
  color?: string
}

/** Initials on a tint of the person's colour over the muted surface, like the web `UserAvatar`. */
export function Avatar({ id, name, size = 32, color }: AvatarProps) {
  const { colors } = useTheme()
  const hue = color ?? hueFor(id)
  return (
    <View accessibilityLabel={name} style={[styles.circle, { width: size, height: size, borderRadius: size / 2, backgroundColor: colors.muted }]}>
      <View style={[StyleSheet.absoluteFill, { backgroundColor: hue, opacity: 0.26 }]} />
      <Text style={{ color: hue, fontFamily: fonts.semibold, fontSize: Math.max(9, Math.round(size * 0.38)), lineHeight: size }}>{initialsOf(name)}</Text>
    </View>
  )
}

const styles = StyleSheet.create({
  circle: { alignItems: 'center', justifyContent: 'center', overflow: 'hidden' },
})
