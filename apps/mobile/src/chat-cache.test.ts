import { expect, test } from 'bun:test'
import { QueryClient } from '@tanstack/react-query'

import { applyChatEvents, chatQueryKeys } from './chat-cache'
import type { MessagePages } from './data/chat'
import type { MessageRecord } from './data/orbit'

const message = (id: string) => ({ id, conversation_id: 'c1', thread_root_id: null, also_in_channel: false }) as unknown as MessageRecord
const page = (items: MessageRecord[]) => ({ pages: [{ items, before: null, after: null }], pageParams: [null] }) as unknown as MessagePages

test('a message that arrives while its conversation loads is not lost when the older response lands', async () => {
  const queryClient = new QueryClient()
  let answer: (data: MessagePages) => void = () => {}
  const loading = queryClient.fetchQuery({ queryKey: ['messages', 'w', 'c1'], queryFn: () => new Promise<MessagePages>((resolve) => (answer = resolve)) })
  applyChatEvents(queryClient, 'w', [{ type: 'message.created', message: message('m2') }])
  // the server read the page before m2 was sent
  answer(page([message('m1')]))
  await loading
  await new Promise((resolve) => setTimeout(resolve, 0))
  expect(queryClient.getQueryData<MessagePages>(['messages', 'w', 'c1'])?.pages[0]?.items.map((item) => item.id)).toEqual(['m1', 'm2'])
})

test('a new message makes the conversation list and its Threads, Pins and Files lists stale', () => {
  const queryClient = new QueryClient()
  for (const kind of ['threads', 'pins', 'files'] as const) queryClient.setQueryData(chatQueryKeys.conversationList('w', 'c1', kind), [])
  queryClient.setQueryData(['conversations', 'w'], [])
  queryClient.setQueryData(['chat-categories', 'w'], [])
  queryClient.setQueryData(chatQueryKeys.conversationList('w', 'c2', 'pins'), [])
  applyChatEvents(queryClient, 'w', [{ type: 'message.created', message: message('m3') }])
  const stale = queryClient.getQueryCache().findAll({ predicate: (query) => query.state.isInvalidated }).map((query) => query.queryKey.join('/'))
  expect(stale.sort()).toEqual(['conversations/w', 'messages/w/files/c1', 'messages/w/pins/c1', 'messages/w/threads/c1'])
})
