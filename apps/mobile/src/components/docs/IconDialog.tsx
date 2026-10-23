import { useEffect, useState } from 'react'
import { Pressable, View } from 'react-native'

import { iconOf, PAGE_ICONS } from '../../data/docs'
import { useTheme } from '../../theme'
import { Button, Input, Text } from '../../ui'
import { Dialog } from '../settings/Dialog'

/**
 * Picks a page or teamspace icon: a few common emoji, or any one typed with the emoji keyboard. The web's full emoji
 * picker has no React Native version.
 */
export function IconDialog({ visible, title, current, onPick, onClose }: { visible: boolean; title: string; current: string | null; onPick: (icon: string | null) => void; onClose: () => void }) {
  const { colors } = useTheme()
  const [typed, setTyped] = useState('')
  useEffect(() => setTyped(''), [visible])
  const pick = (icon: string | null) => {
    onClose()
    onPick(icon)
  }
  return (
    <Dialog
      visible={visible}
      title={title}
      onClose={onClose}
      footer={
        <>
          {current ? <Button label="Remove icon" variant="ghost" onPress={() => pick(null)} /> : null}
          <Button label="Cancel" variant="outline" onPress={onClose} />
          <Button label="Use typed emoji" disabled={!iconOf(typed)} onPress={() => pick(iconOf(typed))} />
        </>
      }
    >
      <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 4 }}>
        {PAGE_ICONS.map((icon) => (
          <Pressable
            key={icon}
            accessibilityRole="button"
            accessibilityLabel={`Icon ${icon}`}
            accessibilityState={{ selected: icon === current }}
            onPress={() => pick(icon)}
            style={({ pressed }) => ({ width: 40, height: 40, alignItems: 'center', justifyContent: 'center', borderRadius: 8, backgroundColor: icon === current ? colors.sidebarAccent : pressed ? colors.muted : 'transparent' })}
          >
            <Text style={{ fontSize: 22, lineHeight: 28 }}>{icon}</Text>
          </Pressable>
        ))}
      </View>
      <Input label="Or type any emoji" value={typed} onChangeText={setTyped} placeholder="😀" autoCorrect={false} returnKeyType="done" onSubmitEditing={() => iconOf(typed) && pick(iconOf(typed))} />
    </Dialog>
  )
}
