import { View } from 'react-native'

import { useTheme } from '../theme'
import { Button } from './Button'
import { Spinner } from './Spinner'
import { Text } from './Text'

export function LoadingView() {
  return (
    <View style={{ flex: 1, alignItems: 'center', justifyContent: 'center' }}>
      <Spinner size="large" />
    </View>
  )
}

export function ErrorView({ error, onRetry }: { error: unknown; onRetry: () => void }) {
  const { spacing } = useTheme()
  return (
    <View style={{ flex: 1, alignItems: 'center', justifyContent: 'center', gap: spacing[3], padding: spacing[6] }}>
      <Text variant="heading" style={{ textAlign: 'center' }}>
        Could not load
      </Text>
      <Text color="muted" style={{ textAlign: 'center' }}>
        {error instanceof Error ? error.message : String(error)}
      </Text>
      <Button label="Retry" variant="outline" onPress={onRetry} />
    </View>
  )
}
