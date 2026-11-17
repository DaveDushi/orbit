import DateTimePicker from '@react-native-community/datetimepicker'
import { Platform } from 'react-native'

import { useTheme } from '../../theme'

/**
 * A date and time from now on: the inline calendar with the time on iOS. Android's picker has no combined mode, so it
 * picks the day and keeps the time of `value`. `DateTimeField.web.tsx` is the browser's field.
 */
export function DateTimeField({ value, minimum, onChange }: { value: Date; minimum: Date; onChange: (at: Date) => void }) {
  const { colors, scheme } = useTheme()
  return (
    <DateTimePicker
      value={value}
      minimumDate={minimum}
      mode={Platform.OS === 'ios' ? 'datetime' : 'date'}
      display={Platform.OS === 'ios' ? 'inline' : 'default'}
      themeVariant={scheme}
      accentColor={colors.primary}
      onValueChange={(_event, picked) => {
        if (Platform.OS === 'ios') return onChange(picked)
        const at = new Date(picked)
        at.setHours(value.getHours(), value.getMinutes(), 0, 0)
        onChange(at)
      }}
    />
  )
}
