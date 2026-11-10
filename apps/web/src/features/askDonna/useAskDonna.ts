import { useReducer } from 'react'
import { useLocation } from 'react-router'
import { toast } from 'sonner'
import { useAgentStatus } from '@/features/agent/api'
import { useOpenDm, useSendMessage } from '@/features/chat/api/mutations'
import { useWorkspace } from '@/features/workspaces/workspaceContext'
import { useIsGuest } from '@/features/workspaces/permissions'
import { pageContextFor, usePageItem, type PageContext } from './pageContext'
import { closedPanel, panelIn, panelReducer, type PanelState } from './panelState'

export interface AskDonna {
  /** The agent's name, which the bar and the panel show. */
  name: string
  /** Where the person is now; `null` in Chat, where the bar and the panel do not show. */
  context: PageContext | null
  panel: PanelState
  /** Sends `text` (if any) to the DM with the agent and opens the panel; the files go to the panel's composer. */
  ask: (text: string, files?: File[]) => Promise<boolean>
  close: () => void
  filesTaken: () => void
  busy: boolean
}

/**
 * The Ask bar and its panel, for members of a workspace where the agent is on; `null` for a guest (who cannot use the
 * agent) and without the agent. The conversation is the person's DM with the agent, so it is the same one as in Chat.
 */
export function useAskDonna(): AskDonna | null {
  const { workspace } = useWorkspace()
  const guest = useIsGuest()
  const status = useAgentStatus(!guest).data
  const { pathname } = useLocation()
  const item = usePageItem()
  const [state, dispatch] = useReducer(panelReducer, closedPanel)
  const openDm = useOpenDm()
  const { send } = useSendMessage()
  const panel = panelIn(state, workspace.id)
  const context = pageContextFor(pathname, item)

  if (guest || !status?.enabled || !status.bot_user_id) return null
  const botId = status.bot_user_id

  async function conversation(): Promise<string | null> {
    if (panel.conversationId) return panel.conversationId
    try {
      return (await openDm.mutateAsync([botId])).id
    } catch {
      toast.error(`Could not open your conversation with ${status?.enabled ? status.name : 'the agent'}. Try again.`)
      return null
    }
  }

  return {
    name: status.name,
    context,
    panel,
    busy: openDm.isPending,
    ask: async (text, files = []) => {
      const id = await conversation()
      if (!id) return false
      const body = text.trim()
      if (body) send({ conversationId: id, body, pageContext: context })
      dispatch({ type: 'open', workspaceId: workspace.id, conversationId: id, files })
      return true
    },
    close: () => dispatch({ type: 'close' }),
    filesTaken: () => dispatch({ type: 'filesTaken' }),
  }
}
