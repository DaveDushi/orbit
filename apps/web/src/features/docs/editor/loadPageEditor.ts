// BlockNote + ProseMirror are ~1 MB: load them only in Docs.
export const loadPageEditor = () => import('./PageEditor')

/** Starts the editor and Yjs downloads before a page (and its socket) is open. A failure surfaces again when they load. */
export function preloadPageEditor() {
  Promise.all([loadPageEditor(), import('yjs'), import('y-websocket')]).catch(() => {})
}
