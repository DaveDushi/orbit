import { useLocalSearchParams } from 'expo-router'

import { SearchScreen } from '../../components/chat/SearchScreen'

/** Search from the chat list: every conversation the reader can read. `?q=` (from the app's Search) runs at once. */
export default function ChatSearchScreen() {
  const { q } = useLocalSearchParams<{ q?: string }>()
  return <SearchScreen initialQuery={q} />
}
