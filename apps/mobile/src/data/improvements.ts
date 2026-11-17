/** The page context of "Suggest a change", in the form the server accepts (`apps/server/src/improvements.rs`, `checked_context`). */
import type { ImprovementContext } from '@orbit/api/improvements'

/** Path: from `/`, no white space; viewport `WIDTHxHEIGHT`; revision: letters, digits, `.`, `-` and `_` only (at most 64). */
export function improvementContext(pathname: string, width: number, height: number, appVersion: string | undefined): ImprovementContext {
  const path = `/${pathname.replace(/^\/+/, '')}`.replace(/[\s\u0000-\u001f\u007f]/g, '').slice(0, 200)
  const side = (value: number) => String(Math.min(99_999, Math.max(1, Math.round(value))))
  const revision = `ios-app-${appVersion ?? 'development'}`.replace(/[^\w.-]/g, '-').slice(0, 64)
  return { path, viewport: `${side(width)}x${side(height)}`, revision }
}
