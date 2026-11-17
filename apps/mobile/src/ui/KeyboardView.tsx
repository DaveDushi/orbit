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
  const rootTop = useRef(0)
  rootTop.current = useContext(RootTop)
  const [bottom, setBottom] = useState(0)
  useEffect(() => {
    if (Platform.OS !== 'ios') return
    const animate = (event: KeyboardEvent) => {
      if (event.duration > 0) {
        const duration = Math.max(event.duration, 10)
        LayoutAnimation.configureNext({ duration, update: { duration, type: LayoutAnimation.Types[event.easing] ?? LayoutAnimation.Types.keyboard } })
      }
    }
    const update = (event: KeyboardEvent) => {
      ref.current?.measureInWindow((_x, y, _width, height) => {
        // the padding is inside the measured height, so the view's bottom stays where its parent puts it
        const covered = keyboardCover({ rootTop: rootTop.current, y, height, keyboardTop: event.endCoordinates.screenY, screenHeight: Dimensions.get('screen').height })
        animate(event)
        setBottom(covered)
      })
    }
    // a hidden keyboard covers nothing: no measure, so a wrong one cannot keep a padding
    const hide = (event: KeyboardEvent) => {
      animate(event)
      setBottom(0)
    }
    const subscriptions = [Keyboard.addListener('keyboardWillChangeFrame', update), Keyboard.addListener('keyboardWillHide', hide)]
    return () => subscriptions.forEach((subscription) => subscription.remove())
  }, [])
  return (
    <View ref={ref} className={className} pointerEvents={pointerEvents} style={[style, { paddingBottom: bottom }]}>
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
