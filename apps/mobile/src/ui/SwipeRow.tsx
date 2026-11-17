import { type ReactNode, useMemo, useRef } from 'react'
import { Animated, PanResponder, View } from 'react-native'

import { useTheme } from '../theme'
import { Icon, type IconName } from './Icon'
import { Text } from './Text'

export interface SwipeAction {
  label: string
  icon: IconName
  /** `primary` (done, restore) or `destructive` (delete). */
  tone: 'primary' | 'destructive'
  onSwipe: () => void
}

/** How far a row must travel before its action runs on release. */
const TRIGGER = 96

/**
 * The iOS list swipe without a native module: drag a row right for `left`, left for `right`. The action shows under
 * the row and runs when the row is let go past `TRIGGER`; the row then springs back. Vertical scrolling wins over a
 * mostly vertical drag. VoiceOver users reach the same actions in the row's long-press menu.
 */
export function SwipeRow({ left, right, children }: { left?: SwipeAction; right?: SwipeAction; children: ReactNode }) {
  const offset = useRef(new Animated.Value(0)).current
  const actions = useRef({ left, right })
  actions.current = { left, right }
  const responder = useMemo(
    () =>
      PanResponder.create({
        onMoveShouldSetPanResponder: (_event, { dx, dy }) => Math.abs(dx) > 12 && Math.abs(dx) > Math.abs(dy) * 2 && (dx > 0 ? Boolean(actions.current.left) : Boolean(actions.current.right)),
        onPanResponderMove: (_event, { dx }) => offset.setValue(dx > 0 ? (actions.current.left ? dx : 0) : actions.current.right ? dx : 0),
        onPanResponderRelease: (_event, { dx }) => {
          const action = dx > TRIGGER ? actions.current.left : dx < -TRIGGER ? actions.current.right : undefined
          Animated.spring(offset, { toValue: 0, useNativeDriver: true, bounciness: 4 }).start()
          action?.onSwipe()
        },
        onPanResponderTerminate: () => Animated.spring(offset, { toValue: 0, useNativeDriver: true }).start(),
        onPanResponderTerminationRequest: () => false,
      }),
    [offset],
  )
  if (!left && !right) return <>{children}</>
  return (
    <View className="overflow-hidden">
      <View className="absolute inset-0 flex-row">
        <Under action={left} side="left" offset={offset} />
        <Under action={right} side="right" offset={offset} />
      </View>
      <Animated.View {...responder.panHandlers} className="bg-background" style={{ transform: [{ translateX: offset }] }}>
        {children}
      </Animated.View>
    </View>
  )
}

/** The action under the row: shown only on its own side, and full strength once a release would run it. */
function Under({ action, side, offset }: { action?: SwipeAction; side: 'left' | 'right'; offset: Animated.Value }) {
  const { colors } = useTheme()
  if (!action) return <View className="flex-1" />
  const range = side === 'left' ? [0, 1, TRIGGER] : [-TRIGGER, -1, 0]
  const opacity = offset.interpolate({ inputRange: range, outputRange: side === 'left' ? [0, 0.6, 1] : [1, 0.6, 0], extrapolate: 'clamp' })
  return (
    <Animated.View
      className={`flex-1 flex-row items-center gap-2 px-5 ${side === 'left' ? 'justify-start' : 'justify-end'} ${action.tone === 'destructive' ? 'bg-destructive' : 'bg-primary'}`}
      style={{ opacity }}
    >
      <Icon name={action.icon} size={18} color={colors.primaryForeground} />
      <Text className="font-noto-semibold text-[13px] text-primary-foreground">{action.label}</Text>
    </Animated.View>
  )
}
