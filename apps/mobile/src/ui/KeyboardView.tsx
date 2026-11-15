import { type ComponentRef, type ReactNode, useEffect, useRef, useState } from 'react'
import { Keyboard, type KeyboardEvent, LayoutAnimation, Platform, View } from 'react-native'

/**
 * The keyboard-avoiding part of an iOS page sheet. React Native's KeyboardAvoidingView compares its place in its
 * parent with the keyboard's place on the screen, but a page sheet starts below the status bar: the bottom of the
 * sheet (the Ask Donna composer with its send button) stayed under the keyboard. This view measures itself in the
 * window each time the keyboard moves, and pads its bottom by the part that the keyboard covers.
 */
export function SheetKeyboardView({ className, children }: { className?: string; children: ReactNode }) {
  const ref = useRef<ComponentRef<typeof View>>(null)
  const [bottom, setBottom] = useState(0)
  useEffect(() => {
    if (Platform.OS !== 'ios') return
    const update = (event: KeyboardEvent) => {
      ref.current?.measureInWindow((_x, y, _width, height) => {
        // the padding is inside the measured height, so the view's bottom stays where the sheet ends
        const covered = Math.max(0, Math.round(y + height - event.endCoordinates.screenY))
        if (event.duration > 0) {
          const duration = Math.max(event.duration, 10)
          LayoutAnimation.configureNext({ duration, update: { duration, type: LayoutAnimation.Types[event.easing] ?? LayoutAnimation.Types.keyboard } })
        }
        setBottom(covered)
      })
    }
    const subscriptions = [Keyboard.addListener('keyboardWillChangeFrame', update), Keyboard.addListener('keyboardWillHide', update)]
    return () => subscriptions.forEach((subscription) => subscription.remove())
  }, [])
  return (
    <View ref={ref} className={className} style={{ paddingBottom: bottom }}>
      {children}
    </View>
  )
}
