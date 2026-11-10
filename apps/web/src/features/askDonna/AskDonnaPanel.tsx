import { useNavigate } from 'react-router'
import { MagicStar, Xmark } from 'reicon-react'
import { PaneHeader, PaneTitle } from '@/components/common/Pane'
import { Tip } from '@/components/common/Tip'
import { Button } from '@/components/ui/button'
import { ChatHostContext } from '@/features/chat/chatHost'
import { conversationPath } from '@/features/chat/chatRoutes'
import { ConversationView } from '@/features/chat/components/ConversationView'
import { useChatPageHost } from '@/features/chat/pages/useChatPageHost'
import type { AskDonna } from './useAskDonna'

/**
 * The right panel of the Ask bar: the person's DM with the agent, with Chat's own list, cards, approvals and composer.
 * The person stays on their page; each message tells the agent which page that is.
 */
export default function AskDonnaPanel({ ask, conversationId }: { ask: AskDonna; conversationId: string }) {
  const navigate = useNavigate()
  const host = useChatPageHost()
  return (
    <aside
      data-slot="ask-panel"
      aria-label={ask.name}
      className="flex w-[400px] shrink-0 flex-col border-l bg-background floating:overflow-hidden floating:rounded-lg floating:border"
      onKeyDown={(event) => {
        // menus, the emoji picker and the mention list handle their own Esc first
        if (event.key === 'Escape' && !event.defaultPrevented) ask.close()
      }}
    >
      <PaneHeader>
        <MagicStar className="size-5 text-primary" />
        <PaneTitle className="flex-1">{ask.name}</PaneTitle>
        <Button
          variant="ghost"
          size="sm"
          onClick={() => {
            ask.close()
            navigate(conversationPath(conversationId))
          }}
        >
          Open in Chat
        </Button>
        <Tip label="Close">
          <Button variant="ghost" size="icon" aria-label="Close" onClick={ask.close}>
            <Xmark className="size-5" />
          </Button>
        </Tip>
      </PaneHeader>
      <ChatHostContext value={host}>
        <ConversationView
          conversationId={conversationId}
          pageContext={() => ask.context}
          files={ask.panel.files}
          onFilesTaken={ask.filesTaken}
        />
      </ChatHostContext>
    </aside>
  )
}
