import { type ComponentRef, createContext, type ReactNode, useContext, useEffect, useRef, useState } from 'react'
import { Dimensions, Keyboard, type KeyboardEvent, LayoutAnimation, Platform, type StyleProp, View, type ViewProps, type ViewStyle } from 'react-native'

import { keyboardCover, sheetTop } from '../data/keyboard'

/** Where the root of the views below starts on the screen (`keyboardCover`): 0, or the top of a page sheet. */
const RootTop = createContext(0)

/**
 * The part of a screen, sheet or dialog that stays above the iOS keyboard: every text composer of the app is in one.
 * Each time the keyboard moves, it measures itself and pads its bottom by the part that the keyboard covers.
 * React Native's KeyboardAvoidingView (and this view in build 14) measured from the root of the view and missed the
 * offset of a page sheet, so the composer of a sheet (Ask Donna) and its send button stayed under the keyboard. In a
 * page sheet, put the content in a `KeyboardSheetRoot`.
 */
export function KeyboardView({ className, style, pointerEvents, children }: { className?: string; style?: StyleProp<ViewStyle>; pointerEvents?: ViewProps['pointerEvents']; children: ReactNode }) {
  const ref = useRef<ComponentRef<typeof View>>(null)
  const rootTop = useContext(RootTop)
  const rootTopRef = useRef(rootTop)
  rootTopRef.current = rootTop
  // the keyboard's top on the screen from its last event; null while it is hidden
  const keyboardTop = useRef<number | null>(null)
  const [bottom, setBottom] = useState(0)
  const measure = useRef((animate?: KeyboardEvent) => {
    if (keyboardTop.current == null && !animate) return
    ref.current?.measureInWindow((_x, y, _width, height) => {
      // the padding is inside the measured height, so the view's bottom stays where its parent puts it
      const covered = keyboardCover({ rootTop: rootTopRef.current, y, height, keyboardTop: keyboardTop.current ?? Number.POSITIVE_INFINITY })
      if (animate && animate.duration > 0) {
        const duration = Math.max(animate.duration, 10)
        LayoutAnimation.configureNext({ duration, update: { duration, type: LayoutAnimation.Types[animate.easing] ?? LayoutAnimation.Types.keyboard } })
      }
      setBottom(covered)
    })
  }).current
  useEffect(() => {
    if (Platform.OS !== 'ios') return
    const update = (event: KeyboardEvent) => {
      // endCoordinates is the whole keyboard frame on the screen, QuickType bar included
      keyboardTop.current = event.endCoordinates.screenY < Dimensions.get('screen').height ? event.endCoordinates.screenY : null
      measure(event)
    }
    const subscriptions = [Keyboard.addListener('keyboardWillChangeFrame', update), Keyboard.addListener('keyboardWillHide', update)]
    return () => subscriptions.forEach((subscription) => subscription.remove())
  }, [measure])
  // the page sheet's top can arrive after the keyboard (the sheet lays out once it is presented): measure again
  useEffect(() => {
    if (Platform.OS === 'ios') measure()
  }, [rootTop, measure])
  return (
    <View ref={ref} className={className} pointerEvents={pointerEvents} style={[style, { paddingBottom: bottom }]} onLayout={Platform.OS === 'ios' ? () => measure() : undefined}>
      {children}
    </View>
  )
}

/** The root of a page sheet (`presentationStyle="pageSheet"`) whose content has a `KeyboardView`: it tells it where the sheet starts on the screen. */
export function KeyboardSheetRoot({ children }: { children: ReactNode }) {
  const [top, setTop] = useState(0)
  return (
    <View className="flex-1" onLayout={(event) => setTop(sheetTop(Dimensions.get('screen').height, event.nativeEvent.layout.height))}>
      <RootTop.Provider value={top}>{children}</RootTop.Provider>
    </View>
  )
}
