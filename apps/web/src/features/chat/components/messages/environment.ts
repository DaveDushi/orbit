import { useEffect, useState, useSyncExternalStore } from 'react'

function matches(query: string): boolean {
  return typeof window.matchMedia === 'function' && window.matchMedia(query).matches
}

function useMediaQuery(query: string): boolean {
  return useSyncExternalStore(
    (listener) => {
      if (typeof window.matchMedia !== 'function') return () => {}
      const media = window.matchMedia(query)
      media.addEventListener('change', listener)
      return () => media.removeEventListener('change', listener)
    },
    () => matches(query),
  )
}

/** Below the app's phone breakpoint (the `max-[899px]:` classes). */
export function useIsPhone(): boolean {
  return useMediaQuery('(max-width: 899px)')
}

/** A touch screen is the main pointer. A touch laptop can say so too: for the keyboard, use `useTouchOnly`. */
export function isCoarsePointer(): boolean {
  return matches('(pointer: coarse)')
}

const TOUCH_PRIMARY = '(hover: none) and (pointer: coarse)'
const FINE_POINTER = '(any-pointer: fine)'

/**
 * A phone or a tablet: touch only, no mouse or trackpad, so most likely an on-screen keyboard. A Windows touch laptop
 * (Surface) can report its touch screen as the main pointer, but it also has a fine pointer and a real keyboard.
 */
export function touchOnly(match: (query: string) => boolean): boolean {
  return match(TOUCH_PRIMARY) && !match(FINE_POINTER)
}

/** On a touch-only device `Enter` makes a new line, and nothing takes focus on its own (it opens the keyboard). */
export function useTouchOnly(): boolean {
  const primary = useMediaQuery(TOUCH_PRIMARY)
  const fine = useMediaQuery(FINE_POINTER)
  return primary && !fine
}

export function isTouchOnly(): boolean {
  return touchOnly(matches)
}

function subscribeWindowFocus(listener: () => void): () => void {
  window.addEventListener('focus', listener)
  window.addEventListener('blur', listener)
  document.addEventListener('visibilitychange', listener)
  return () => {
    window.removeEventListener('focus', listener)
    window.removeEventListener('blur', listener)
    document.removeEventListener('visibilitychange', listener)
  }
}

/** The window has focus and is visible: one of the conditions for marking a conversation read. */
export function useWindowFocused(): boolean {
  return useSyncExternalStore(subscribeWindowFocus, () => document.hasFocus() && document.visibilityState === 'visible')
}

/** True once `active` has been true for `delay` ms: a first load stays blank for a moment before its spinner. */
export function useDelayed(active: boolean, delay: number): boolean {
  const [elapsed, setElapsed] = useState(false)
  useEffect(() => {
    if (!active) return
    const timer = setTimeout(() => setElapsed(true), delay)
    return () => {
      clearTimeout(timer)
      setElapsed(false)
    }
  }, [active, delay])
  return active && elapsed
}
