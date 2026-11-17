import { useSyncExternalStore } from 'react'

/** The Orbit desktop app's state, from its bridge script (`apps/desktop/crates/orbit-desktop/assets/bridge.js`, 0.1.8+). */
export interface DesktopAppState {
  app: string
  os: string
  autostart: boolean
  /** "Start with Windows", "Open at login" or "Start at login". */
  autostartLabel: string
  /** A newer version, or null. */
  update: string | null
  /** The update is downloading (Windows). */
  updating: boolean
  /** The app installs updates itself (Windows); else it opens the download. */
  install: boolean
}

export type DesktopAction = 'autostart' | 'update' | 'check' | 'server' | 'diagnostics' | 'quit'

type Bridge = { getState?: () => DesktopAppState | null; action?: (name: DesktopAction, on?: boolean) => void }

/** In the desktop app (any version). */
export const inDesktopApp = typeof window !== 'undefined' && '__orbitDesktop' in window

function bridge(): Bridge | null {
  return inDesktopApp ? ((window as { __orbitDesktop?: Bridge }).__orbitDesktop ?? null) : null
}

function subscribe(onChange: () => void) {
  window.addEventListener('orbit-desktop', onChange)
  return () => window.removeEventListener('orbit-desktop', onChange)
}

/** The app's state; null in a browser and in apps before 0.1.8 (they have the tray menu only). */
export function useDesktopApp(): DesktopAppState | null {
  return useSyncExternalStore(subscribe, () => bridge()?.getState?.() ?? null, () => null)
}

export function desktopAction(name: DesktopAction, on?: boolean) {
  bridge()?.action?.(name, on)
}
