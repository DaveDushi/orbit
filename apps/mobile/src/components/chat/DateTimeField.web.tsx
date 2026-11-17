import { createElement } from 'react'

import { fonts, useTheme } from '../../theme'

const pad = (value: number) => String(value).padStart(2, '0')
/** The value of a `datetime-local` field for a time (local, to the minute). */
const localValue = (date: Date) => `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`

/** The browser's date and time field: the system picker has no web version (the web preview of the app). */
export function DateTimeField({ value, minimum, onChange }: { value: Date; minimum: Date; onChange: (at: Date) => void }) {
  const { colors, scheme } = useTheme()
  return createElement('input', {
    type: 'datetime-local',
    'aria-label': 'Date and time',
    defaultValue: localValue(value),
    min: localValue(minimum),
    onChange: (event: { target: { value: string } }) => {
      const at = new Date(event.target.value)
      if (/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}/.test(event.target.value) && !Number.isNaN(at.getTime())) onChange(at)
    },
    style: { padding: '8px 10px', borderRadius: 10, border: `1px solid ${colors.input}`, background: 'transparent', color: colors.foreground, colorScheme: scheme, fontSize: 16, fontFamily: fonts.regular },
  })
}
