import { File, Paths } from 'expo-file-system'

/**
 * Where the query cache is kept: one file per key in the app's cache folder, which iOS does not back up and may empty
 * when the phone is short of space (the app then loads from the server). `cache-file.web.ts` replaces it in the web preview.
 */
const file = (key: string) => new File(Paths.cache, `${key}.json`)

export const cacheStorage = {
  getItem: async (key: string) => {
    const item = file(key)
    return item.exists ? item.text() : null
  },
  setItem: (key: string, value: string) => file(key).write(value),
  removeItem: (key: string) => {
    const item = file(key)
    if (item.exists) item.delete()
  },
}
