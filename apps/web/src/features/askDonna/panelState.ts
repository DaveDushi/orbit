/**
 * The Ask panel: open or closed, and the conversation it shows (the person's DM with the agent in one workspace).
 * Closing keeps the conversation, so the panel opens again where it was; the messages stay in the chat cache.
 */
export interface PanelState {
  open: boolean
  /** The workspace of `conversationId`: another workspace starts closed and without it (`panelIn`). */
  workspaceId: string | null
  /** Known after the first open of the DM. */
  conversationId: string | null
  /** Files from the bar's +, for the panel's composer to take. */
  files: File[]
}

export type PanelAction =
  | { type: 'open'; workspaceId: string; conversationId: string; files?: File[] }
  | { type: 'close' }
  | { type: 'filesTaken' }

export const closedPanel: PanelState = { open: false, workspaceId: null, conversationId: null, files: [] }

export function panelReducer(state: PanelState, action: PanelAction): PanelState {
  switch (action.type) {
    case 'open': {
      const same = state.workspaceId === action.workspaceId
      return {
        open: true,
        workspaceId: action.workspaceId,
        conversationId: action.conversationId,
        files: action.files?.length ? action.files : same ? state.files : [],
      }
    }
    case 'close':
      return { ...state, open: false, files: [] }
    case 'filesTaken':
      return state.files.length ? { ...state, files: [] } : state
  }
}

/** The panel as the given workspace sees it. */
export function panelIn(state: PanelState, workspaceId: string): PanelState {
  return state.workspaceId === workspaceId ? state : closedPanel
}
