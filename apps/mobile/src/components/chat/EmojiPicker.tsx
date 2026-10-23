import { useMemo, useState } from 'react'
import { Image, Modal, Platform, Pressable, SectionList, Text as NativeText, TextInput, useWindowDimensions, View } from 'react-native'
import { SafeAreaView } from 'react-native-safe-area-context'

import { customEmojiName, type EmojiEntry, searchEmoji } from '../../data/chat'
import { useCustomEmoji, useEmojiGroups, useServerImage } from '../../hooks/chat'
import { type, useTheme } from '../../theme'
import { Button, Icon, Text } from '../../ui'

const CELL = 44

/** One emoji: a Unicode character, or a workspace emoji (`:name:`) drawn from its image. */
export function Emoji({ value, size }: { value: string; size: number }) {
  const custom = useCustomEmoji()
  const image = useServerImage()
  const name = customEmojiName(value)
  const path = name ? custom.get(name) : undefined
  if (name && path) return <Image accessibilityLabel={value} source={image(path)} style={{ width: size, height: size }} resizeMode="contain" />
  return <NativeText style={{ fontSize: name ? size * 0.5 : size * 0.85, lineHeight: size }}>{value}</NativeText>
}

type Row = { key: string; cells: { key: string; value: string; label: string }[] }

const rowsOf = (entries: { key: string; value: string; label: string }[], columns: number, prefix: string): Row[] =>
  Array.from({ length: Math.ceil(entries.length / columns) }, (_, index) => ({ key: `${prefix}-${index}`, cells: entries.slice(index * columns, index * columns + columns) }))

/**
 * The web `EmojiPicker` as a sheet: a search field, the workspace's custom emoji first (for reactions and the
 * composer), then the emojibase groups the server serves.
 */
export function EmojiPicker({ visible, onPick, onClose }: { visible: boolean; onPick: (emoji: string) => void; onClose: () => void }) {
  const { colors, scheme } = useTheme()
  const groups = useEmojiGroups()
  const custom = useCustomEmoji()
  const { width } = useWindowDimensions()
  const [query, setQuery] = useState('')
  const columns = Math.max(6, Math.floor((Math.min(width, 520) - 16) / CELL))
  const text = query.trim().toLowerCase().replace(/^:/, '')

  const sections = useMemo(() => {
    const cell = (entry: EmojiEntry) => ({ key: entry.emoji, value: entry.emoji, label: entry.names[0] ?? entry.emoji })
    const customCells = [...custom.keys()].filter((name) => name.includes(text)).map((name) => ({ key: `:${name}:`, value: `:${name}:`, label: `:${name}:` }))
    const list = text
      ? [{ title: 'Results', data: rowsOf([...customCells, ...searchEmoji(groups, text).map(cell)], columns, 'results') }]
      : [...(customCells.length > 0 ? [{ title: 'Custom', data: rowsOf(customCells, columns, 'custom') }] : []), ...groups.map((group) => ({ title: group.title, data: rowsOf(group.emojis.map(cell), columns, group.key) }))]
    return list.filter((section) => section.data.length > 0)
  }, [columns, custom, groups, text])

  const close = () => {
    setQuery('')
    onClose()
  }

  return (
    <Modal visible={visible} animationType="slide" presentationStyle="pageSheet" onRequestClose={close}>
      <SafeAreaView edges={Platform.OS === 'ios' ? ['bottom', 'left', 'right'] : ['top', 'bottom', 'left', 'right']} style={{ flex: 1, backgroundColor: colors.popover }}>
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8, padding: 8 }}>
          <View style={{ flex: 1, height: 36, flexDirection: 'row', alignItems: 'center', gap: 8, paddingHorizontal: 10, borderRadius: 8, backgroundColor: colors.secondary }}>
            <Icon name="search" size={14} color={colors.mutedForeground} />
            <SearchField value={query} onChange={setQuery} scheme={scheme} />
          </View>
          <Button label="Close" variant="ghost" onPress={close} />
        </View>
        <SectionList
          sections={sections}
          keyExtractor={(row) => row.key}
          keyboardShouldPersistTaps="handled"
          stickySectionHeadersEnabled
          initialNumToRender={12}
          contentContainerStyle={{ paddingHorizontal: 8, paddingBottom: 16 }}
          ListEmptyComponent={<Text style={[type(14), { padding: 16, textAlign: 'center', color: colors.mutedForeground }]}>No emoji found.</Text>}
          renderSectionHeader={({ section }) => (
            <Text style={[type(12, 'medium'), { paddingHorizontal: 4, paddingTop: 12, paddingBottom: 6, color: colors.mutedForeground, backgroundColor: colors.popover }]}>{section.title}</Text>
          )}
          renderItem={({ item }) => (
            <View style={{ flexDirection: 'row' }}>
              {item.cells.map((cell) => (
                <Pressable
                  key={cell.key}
                  accessibilityRole="button"
                  accessibilityLabel={cell.label}
                  onPress={() => {
                    onPick(cell.value)
                    close()
                  }}
                  style={({ pressed }) => ({ width: CELL, height: CELL, alignItems: 'center', justifyContent: 'center', borderRadius: 8, backgroundColor: pressed ? colors.muted : 'transparent' })}
                >
                  <Emoji value={cell.value} size={30} />
                </Pressable>
              ))}
            </View>
          )}
        />
      </SafeAreaView>
    </Modal>
  )
}

function SearchField({ value, onChange, scheme }: { value: string; onChange: (value: string) => void; scheme: 'light' | 'dark' }) {
  const { colors } = useTheme()
  return (
    <TextInput
      value={value}
      onChangeText={onChange}
      placeholder="Search emoji"
      accessibilityLabel="Search emoji"
      placeholderTextColor={colors.mutedForeground}
      selectionColor={colors.primary}
      keyboardAppearance={scheme}
      autoCapitalize="none"
      autoCorrect={false}
      style={[type(16), { flex: 1, paddingVertical: 0, color: colors.foreground }]}
    />
  )
}
