/**
 * Joins the field's text and the spoken pieces with one space between them. The field's text stays as it is (a
 * trailing space or line break too), the spoken pieces are trimmed, and empty pieces add nothing.
 */
export function joinSpeech(text: string, ...spoken: string[]): string {
  return spoken.reduce((joined, part) => {
    const piece = part.trim()
    if (!piece) return joined
    if (!joined || /\s$/.test(joined)) return joined + piece
    return `${joined} ${piece}`
  }, text)
}

/**
 * The recognizer's language: the device locale when the recognizer has it, else another locale of the same language,
 * else en-US. An empty `supported` list (the web) takes the device locale as it is.
 */
export function pickLocale(device: string | undefined, supported: string[]): string {
  const wanted = device?.split('-u-')[0]?.replace(/_/g, '-')
  if (!wanted) return 'en-US'
  if (supported.length === 0) return wanted
  const normalized = supported.map((locale) => locale.replace(/_/g, '-'))
  const exact = normalized.findIndex((locale) => locale.toLowerCase() === wanted.toLowerCase())
  if (exact !== -1) return supported[exact]
  const language = wanted.split('-')[0].toLowerCase()
  const sameLanguage = normalized.findIndex((locale) => locale.split('-')[0].toLowerCase() === language)
  return sameLanguage !== -1 ? supported[sameLanguage] : 'en-US'
}
