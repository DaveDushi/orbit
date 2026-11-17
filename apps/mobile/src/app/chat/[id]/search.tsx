import { useLocalSearchParams } from 'expo-router'

import { SearchScreen } from '../../../components/chat/SearchScreen'

/** Search from a conversation's header: in it first, "This conversation" off for all. */
export default function ConversationSearchScreen() {
  const { id } = useLocalSearchParams<{ id: string }>()
  return <SearchScreen conversationId={id} />
}
