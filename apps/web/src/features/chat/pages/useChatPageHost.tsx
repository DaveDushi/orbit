import { useChatContext } from '@/features/chat/api/chatContext'
import { useConversations } from '@/features/chat/api/queries'
import type { Message as ChatMessage } from '@/features/chat/api/types'
import type { ChatHost } from '@/features/chat/chatHost'
import { messagePath } from '@/features/chat/chatRoutes'
import { decodeMentions } from '@/features/chat/lib/mentionTokens'
import { useProjects } from '@/features/tasks/api/projects'
import { useOpenNewTask } from '@/features/tasks/newTask'
import { useMembers } from '@/features/workspaces/api'
import { useIsGuest } from '@/features/workspaces/permissions'
import { ChatLinkCard, ChatLinkChip } from './ChatLinkCard'
import { orbitLinkTarget, taskTitleFromMessage } from './chatPageLib'

/** What chat messages need from the other features, for the chat page and the Ask panel. */
export function useChatPageHost(): ChatHost {
  const { workspaceId } = useChatContext()
  const conversations = useConversations()
  const people = useMembers(workspaceId).data
  const openNewTask = useOpenNewTask()
  // a guest has no tasks or docs: no task keys, no Orbit link cards, no "Create task"
  const guest = useIsGuest()
  const projects = useProjects(workspaceId, !guest).data
  return {
    renderLinkCard: (url) => {
      const target = guest ? null : orbitLinkTarget(url, window.location.origin)
      return target ? <ChatLinkCard kind={target.kind} id={target.id} /> : null
    },
    renderLink: (url, plain, hidden) => {
      const target = guest ? null : orbitLinkTarget(url, window.location.origin)
      return target ? <ChatLinkChip kind={target.kind} id={target.id} hidden={hidden}>{plain}</ChatLinkChip> : plain
    },
    taskKeys: (projects ?? []).map((project) => project.key),
    createTask: guest ? undefined : (message: ChatMessage) => {
      const source = conversations.data?.find((candidate) => candidate.id === message.conversationId)
      const text = decodeMentions(message.body, people ?? [], conversations.data ?? [])
      const where = source && source.kind !== 'dm' ? `a message in #${source.name}` : 'a direct message'
      openNewTask({
        title: taskTitleFromMessage(text),
        description: `Created from [${where}](${window.location.origin}${messagePath(message)}).`,
      })
    },
  }
}
